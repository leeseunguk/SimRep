import { createHash, randomBytes, timingSafeEqual } from 'node:crypto'
import { createServer } from 'node:http'
import { createRemoteJWKSet, jwtVerify } from 'jose'
import { Store, accountId } from './store.mjs'
import { BridgeError } from './errors.mjs'

export const ISSUER = 'https://auth.openai.com'
export const AUTHORIZE = ISSUER + '/api/accounts/authorize'
export const TOKEN = ISSUER + '/api/accounts/oauth/token'
export const RESOURCE = 'https://api.openai.com/v1'
export const SCOPE = 'openid profile email offline_access resource.invoke chatgpt.tokens.use.direct'
const TERMINAL_REFRESH = new Set(['invalid_grant', 'invalid_refresh_token', 'token_expired', 'refresh_token_expired', 'refresh_token_invalidated', 'refresh_token_reused'])

export function attemptValues() {
  const verifier = randomBytes(64).toString('base64url')
  return {
    state: randomBytes(32).toString('base64url'),
    nonce: randomBytes(32).toString('base64url'),
    verifier,
    challenge: createHash('sha256').update(verifier).digest('base64url'),
    expiresAt: Date.now() + 10 * 60_000,
  }
}

function same(a, b) {
  return typeof a === 'string' && typeof b === 'string' && a.length === b.length &&
    timingSafeEqual(Buffer.from(a), Buffer.from(b))
}

export function validateCallback(url, attempt) {
  if (attempt.consumed || Date.now() >= attempt.expiresAt) throw new BridgeError('AUTH_CALLBACK', 'Sign-in attempt expired or was already used.')
  attempt.consumed = true
  for (const key of ['state', 'code', 'client_id', 'error']) {
    if (url.searchParams.getAll(key).length > 1) throw new BridgeError('AUTH_CALLBACK', 'Duplicate callback parameter.')
  }
  if (url.origin + url.pathname !== attempt.redirectUri || !same(url.searchParams.get('state'), attempt.state)) {
    throw new BridgeError('AUTH_CALLBACK', 'Sign-in callback could not be verified.')
  }
  if (url.searchParams.has('error')) throw new BridgeError('AUTH_DENIED', 'ChatGPT sign-in was declined. You can sign in again.')
  const issued = url.searchParams.get('client_id')
  const client = attempt.clientId === 'dynamic_agent_client' ? issued : attempt.clientId
  if (!client || client === 'dynamic_agent_client' || (issued && client !== issued)) {
    throw new BridgeError('AUTH_CALLBACK', 'The returned client registration did not match.')
  }
  const code = url.searchParams.get('code')
  if (!code) throw new BridgeError('AUTH_CALLBACK', 'Sign-in callback has no authorization code.')
  return { client, code }
}

export async function validateIdentity(idToken, client, nonce, jwks, expectedSubject = undefined) {
  const { payload } = await jwtVerify(idToken, jwks, {
    issuer: ISSUER, audience: client, algorithms: ['RS256'], requiredClaims: ['sub', 'exp', 'iat', ...(nonce ? ['nonce'] : [])], clockTolerance: 5,
  })
  if ((nonce && !same(payload.nonce, nonce)) || typeof payload.sub !== 'string' || !payload.sub ||
    (expectedSubject && payload.sub !== expectedSubject) ||
    (Array.isArray(payload.aud) && payload.aud.length > 1 && payload.azp !== client)) {
    throw new BridgeError('AUTH_IDENTITY', 'The verified sign-in identity did not match this attempt.')
  }
  return payload
}

export class ChatGPTAuth {
  constructor({ store = new Store(), fetcher = fetch, jwks = createRemoteJWKSet(new URL(ISSUER + '/.well-known/jwks.json')), tokenEndpoint = TOKEN } = {}) {
    this.store = store
    this.fetcher = fetcher
    this.jwks = jwks
    this.tokenEndpoint = tokenEndpoint
    this.pending = undefined
    this.starting = false
    this.unverifiedClient = undefined
    this.generation = 0
  }

  async post(form, endpoint = this.tokenEndpoint) {
    const response = await this.fetcher(endpoint, {
      method: 'POST', redirect: 'error', signal: AbortSignal.timeout(20_000),
      headers: { 'content-type': 'application/x-www-form-urlencoded', accept: 'application/json' },
      body: new URLSearchParams(form),
    })
    const body = await response.json().catch(() => ({}))
    if (!response.ok) {
      const code = typeof body.error === 'string' ? body.error : body.error?.code
      // Never include a raw body: a failed exchange may echo credential values.
      throw new BridgeError(code || 'AUTH_HTTP', 'ChatGPT authorization failed. Check account permissions or sign in again.', {
        status: response.status, code, requestId: response.headers.get('x-request-id') || response.headers.get('openai-request-id'),
      })
    }
    return body
  }

  async begin(account = undefined) {
    if (this.pending || this.starting) throw new BridgeError('BUSY', 'A ChatGPT sign-in is already in progress.')
    this.starting = true
    const generation = this.generation
    try {
    const saved = account ? (await this.store.accounts()).find(a => a.id === account) : undefined
    if (account && !saved) throw new BridgeError('INVALID_PARAMS', 'Unknown ChatGPT registration.')
    const tokens = saved ? await this.store.credential(saved.id) : undefined
    const host = await this.store.host()
    const attempt = { ...attemptValues(), consumed: false, cancelled: false, redirectUri: '', clientId: saved?.clientId || this.unverifiedClient || 'dynamic_agent_client' }
    let resolve, reject
    const completed = new Promise((yes, no) => { resolve = yes; reject = no })
    // Caller subscribes immediately after begin; prevent a callback race from
    // creating an unhandled rejection before that subscription is attached.
    completed.catch(() => {})
    const server = createServer((req, res) => {
      res.setHeader('Cache-Control', 'no-store')
      res.setHeader('Content-Security-Policy', "default-src 'none'")
      if (req.method !== 'GET' || req.url?.split('?')[0] !== '/auth/callback' || req.headers.host !== new URL(attempt.redirectUri).host) {
        res.writeHead(404); res.end(); return
      }
      if (attempt.consumed) { res.writeHead(400); res.end('Callback already used.'); return }
      void (async () => {
        try {
          const { client, code } = validateCallback(new URL(req.url, attempt.redirectUri), attempt)
          let tokens
          try {
            tokens = await this.post({
              grant_type: 'authorization_code', client_id: client, code, code_verifier: attempt.verifier,
              redirect_uri: attempt.redirectUri, resource: RESOURCE,
            })
          } catch (error) {
            // Registration may already exist even when its one-time code
            // expires. Retain the issued ID for a fresh attempt, without
            // assigning it to an account before identity has been validated.
            if (!saved && error.code === 'invalid_grant') this.unverifiedClient = client
            throw error
          }
          const identity = await validateIdentity(tokens.id_token, client, attempt.nonce, this.jwks, saved?.subject)
          if (attempt.cancelled) throw new BridgeError('CANCELLED', 'Sign-in cancelled.')
          const id = accountId(client, identity.sub)
          const record = this.record(tokens, { id, clientId: client, subject: identity.sub, email: identity.email, nonce: attempt.nonce, host })
          await this.store.locked(async () => {
            if (attempt.cancelled) throw new BridgeError('CANCELLED', 'Sign-in cancelled.')
            await this.store.write('account-' + id, record)
            await this.store.write('registration-' + id, { id, clientId: client, subject: identity.sub, email: identity.email })
            if (attempt.cancelled) {
              await this.store.remove('account-' + id)
              throw new BridgeError('CANCELLED', 'Sign-in cancelled.')
            }
          })
          this.unverifiedClient = undefined
          res.end('Signed in to SimRep. You can close this window.')
          resolve(id)
        } catch (error) {
          res.writeHead(400); res.end('Sign-in failed. Return to SimRep and try again.')
          reject(error)
        } finally { finish() }
      })()
    })
    let timer
    const finish = () => {
      clearTimeout(timer)
      server.close()
      this.pending = undefined
    }
    await new Promise((yes, no) => { server.once('error', no); server.listen(0, '127.0.0.1', () => yes(undefined)) })
    const address = server.address()
    if (!address || typeof address === 'string') throw new Error('No callback port')
    attempt.redirectUri = `http://127.0.0.1:${address.port}/auth/callback`
    timer = setTimeout(() => { attempt.cancelled = true; reject(new BridgeError('AUTH_TIMEOUT', 'Sign-in expired. Try again.')); finish() }, 10 * 60_000)
    this.pending = { close: () => { attempt.cancelled = true; reject(new BridgeError('CANCELLED', 'Sign-in cancelled.')); finish() } }
    if (generation !== this.generation) {
      this.pending.close()
      throw new BridgeError('CANCELLED', 'Sign-in cancelled.')
    }
    const query = new URLSearchParams({
      client_id: attempt.clientId, ext_agent_host_id: host, response_type: 'code',
      redirect_uri: attempt.redirectUri, scope: SCOPE, resource: RESOURCE, state: attempt.state,
      nonce: attempt.nonce, code_challenge_method: 'S256', code_challenge: attempt.challenge,
    })
    if (attempt.clientId === 'dynamic_agent_client') query.set('agent_name_hint', 'SimRep')
    if (tokens?.id_token) query.set('id_token_hint', tokens.id_token)
    if (saved?.email) query.set('login_hint', saved.email)
    if (tokens && !tokens.scopes?.includes('chatgpt.tokens.use.direct')) query.set('prompt', 'consent')
    return { url: AUTHORIZE + '?' + query, completed }
    } finally { this.starting = false }
  }

  record(tokens, previous) {
    if (typeof tokens.access_token !== 'string' || tokens.token_type?.toLowerCase() !== 'bearer' ||
      !Number.isFinite(tokens.expires_in) || tokens.expires_in <= 0) {
      throw new BridgeError('AUTH_RESPONSE', 'Invalid OAuth token response.')
    }
    return {
      ...previous,
      access_token: tokens.access_token,
      refresh_token: tokens.refresh_token || previous.refresh_token,
      id_token: tokens.id_token || previous.id_token,
      token_type: tokens.token_type,
      scopes: typeof tokens.scope === 'string' ? tokens.scope.split(/\s+/).filter(Boolean) : previous.scopes || [],
      expiresAt: Date.now() + tokens.expires_in * 1000,
    }
  }

  async access(account) {
    return this.store.locked(async () => {
      let record = await this.store.credential(account)
      if (!record) throw new BridgeError('AUTH_REQUIRED', 'Continue with ChatGPT to sign in.')
      if (!record.scopes.includes('chatgpt.tokens.use.direct')) throw new BridgeError('PLAN_PERMISSION_REQUIRED', 'ChatGPT plan use is not enabled. Continue with ChatGPT to authorize it.')
      if (record.expiresAt < Date.now() + 60_000) {
        if (!record.refresh_token) throw new BridgeError('AUTH_REQUIRED', 'ChatGPT session expired. Sign in again.')
        try {
          const tokens = await this.post({
            grant_type: 'refresh_token', client_id: record.clientId, refresh_token: record.refresh_token, resource: RESOURCE,
          })
          if (tokens.id_token) await validateIdentity(tokens.id_token, record.clientId, undefined, this.jwks, record.subject)
          record = this.record(tokens, record)
          await this.store.write('account-' + account, record)
        } catch (error) {
          if (TERMINAL_REFRESH.has(error.code)) await this.store.remove('account-' + account)
          throw error
        }
      }
      if (!record.scopes.includes('chatgpt.tokens.use.direct')) throw new BridgeError('PLAN_PERMISSION_REQUIRED', 'ChatGPT plan use is not enabled.')
      return record
    })
  }

  async accounts() {
    const registrations = await this.store.accounts()
    return Promise.all(registrations.map(async r => {
      const record = await this.store.credential(r.id)
      return { id: r.id, provider: 'chatgpt', label: `${r.email || 'ChatGPT'} (${r.id.slice(0, 8)})`,
        ...(r.email ? { email: r.email } : {}), authenticated: Boolean(record),
        planEnabled: Boolean(record?.scopes.includes('chatgpt.tokens.use.direct')) }
    }))
  }

  async logout(account) {
    this.close()
    return this.store.locked(async () => {
      const record = await this.store.credential(account)
      let confirmed = !record?.refresh_token
      try {
        if (record?.refresh_token) {
          const discovery = await this.fetcher(ISSUER + '/.well-known/openid-configuration', { redirect: 'error', signal: AbortSignal.timeout(15_000) })
          const config = await discovery.json()
          const endpoint = new URL(config.revocation_endpoint)
          if (!discovery.ok || endpoint.origin !== ISSUER || endpoint.username || endpoint.password) throw new Error('Invalid revocation endpoint')
          const response = await this.fetcher(endpoint, {
            method: 'POST', redirect: 'error', signal: AbortSignal.timeout(15_000),
            headers: { 'content-type': 'application/x-www-form-urlencoded' },
            body: new URLSearchParams({ token: record.refresh_token, token_type_hint: 'refresh_token', client_id: record.clientId }),
          })
          confirmed = response.status === 200
        }
      } catch { confirmed = false } finally { await this.store.remove('account-' + account) }
      return { signedOut: true, remoteRevocationConfirmed: confirmed }
    })
  }

  close() { this.generation++; this.pending?.close() }
}
