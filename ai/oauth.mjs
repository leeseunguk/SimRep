import { createHash, randomBytes, timingSafeEqual } from 'node:crypto'
import { createServer } from 'node:http'
import { createRemoteJWKSet, jwtVerify } from 'jose'
import { Store, accountId } from './store.mjs'
import { BridgeError } from './errors.mjs'
import { SUBSCRIPTION, subscriptionAccount } from './auth/chatgpt.mjs'

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
  return typeof a === 'string' && typeof b === 'string' && Buffer.byteLength(a) === Buffer.byteLength(b) &&
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
  let payload
  try {
    ;({ payload } = await jwtVerify(idToken, jwks, {
      issuer: ISSUER, audience: client, algorithms: ['RS256'], requiredClaims: ['sub', 'exp', 'iat', ...(nonce ? ['nonce'] : [])], clockTolerance: 5,
    }))
  } catch { throw new BridgeError('AUTH_IDENTITY', 'ChatGPT sign-in identity could not be verified. Sign in again.') }
  if ((nonce && !same(payload.nonce, nonce)) || typeof payload.sub !== 'string' || !payload.sub ||
    (expectedSubject && payload.sub !== expectedSubject) ||
    (Array.isArray(payload.aud) && payload.aud.length > 1 && payload.azp !== client)) {
    throw new BridgeError('AUTH_IDENTITY', 'The verified sign-in identity did not match this attempt.')
  }
  return payload
}

export class ChatGPTAuth {
  constructor({ store = new Store(), fetcher = fetch, jwks = createRemoteJWKSet(new URL(ISSUER + '/.well-known/jwks.json')),
    flow = 'siwc', callbackPorts = undefined, tokenEndpoint = flow === 'subscription' ? SUBSCRIPTION.token : TOKEN } = {}) {
    if (!['siwc', 'subscription'].includes(flow)) throw new BridgeError('INVALID_PARAMS', 'Unknown ChatGPT authorization flow.')
    this.flow = flow; this.callbackPorts = callbackPorts || (flow === 'subscription' ? SUBSCRIPTION.ports : [0])
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
      const error = new BridgeError(code === 'access_denied' ? 'AUTH_DENIED' : 'AUTH_HTTP',
        'ChatGPT authorization failed. Check account permissions or sign in again.', { status: response.status })
      error.oauthCode = code
      throw error
    }
    return body
  }

  async begin(account = undefined) {
    if (this.pending || this.starting) throw new BridgeError('BUSY', 'A ChatGPT sign-in is already in progress.')
    this.starting = true
    const generation = this.generation
    try {
    const saved = account ? (await this.registrations()).find(a => a.id === account) : undefined
    if (account && !saved) throw new BridgeError('INVALID_PARAMS', 'Unknown ChatGPT registration.')
    const tokens = saved ? await this.store.credential(saved.id) : undefined
    const host = await this.store.host()
    const attempt = { ...attemptValues(), consumed: false, cancelled: false, redirectUri: '',
      clientId: saved?.clientId || (this.flow === 'subscription' ? SUBSCRIPTION.clientId : this.unverifiedClient || 'dynamic_agent_client') }
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
              redirect_uri: attempt.redirectUri, ...(this.flow === 'siwc' && { resource: RESOURCE }),
            })
          } catch (error) {
            // Registration may already exist even when its one-time code
            // expires. Retain the issued ID for a fresh attempt, without
            // assigning it to an account before identity has been validated.
            if (!saved && error.oauthCode === 'invalid_grant') this.unverifiedClient = client
            throw error
          }
          const identity = await validateIdentity(tokens.id_token, client, attempt.nonce, this.jwks, saved?.subject)
          const chatgptAccountId = this.flow === 'subscription' ? subscriptionAccount(tokens.access_token) : undefined
          if (saved?.chatgptAccountId && saved.chatgptAccountId !== chatgptAccountId) {
            throw new BridgeError('AUTH_IDENTITY', 'Choose the same ChatGPT workspace/account when reauthorizing this registration.')
          }
          if (attempt.cancelled) throw new BridgeError('CANCELLED', 'Sign-in cancelled.')
          const id = accountId(client, chatgptAccountId ? JSON.stringify([identity.sub, chatgptAccountId]) : identity.sub)
          const record = this.record(tokens, { id, flow: this.flow, clientId: client, subject: identity.sub, email: identity.email, nonce: attempt.nonce, host })
          await this.store.locked(async () => {
            if (attempt.cancelled) throw new BridgeError('CANCELLED', 'Sign-in cancelled.')
            await this.store.write('account-' + id, record)
            await this.store.write('registration-' + id, { id, flow: this.flow, clientId: client, subject: identity.sub,
              email: identity.email, ...(chatgptAccountId && { chatgptAccountId }) })
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
    let bound = false
    for (const port of this.callbackPorts) {
      try {
        await new Promise((yes, no) => {
          const failed = error => { server.removeListener('listening', ready); no(error) }
          const ready = () => { server.removeListener('error', failed); yes(undefined) }
          server.once('error', failed); server.once('listening', ready); server.listen(port, '127.0.0.1')
        })
        bound = true; break
      } catch (error) { if (error.code !== 'EADDRINUSE') throw new BridgeError('AUTH_CALLBACK', 'Cannot open the ChatGPT callback listener.') }
    }
    if (!bound) throw new BridgeError('AUTH_CALLBACK', 'ChatGPT callback ports are busy. Close the other sign-in and retry.')
    const address = server.address()
    if (!address || typeof address === 'string') throw new Error('No callback port')
    attempt.redirectUri = `http://${this.flow === 'subscription' ? 'localhost' : '127.0.0.1'}:${address.port}/auth/callback`
    timer = setTimeout(() => { attempt.cancelled = true; reject(new BridgeError('AUTH_TIMEOUT', 'Sign-in expired. Try again.')); finish() }, 10 * 60_000)
    this.pending = { close: () => { attempt.cancelled = true; reject(new BridgeError('CANCELLED', 'Sign-in cancelled.')); finish() } }
    if (generation !== this.generation) {
      this.pending.close()
      throw new BridgeError('CANCELLED', 'Sign-in cancelled.')
    }
    const query = new URLSearchParams({
      client_id: attempt.clientId, ext_agent_host_id: host, response_type: 'code',
      redirect_uri: attempt.redirectUri, scope: this.flow === 'subscription' ? SUBSCRIPTION.scope : SCOPE,
      ...(this.flow === 'siwc' && { resource: RESOURCE }), state: attempt.state,
      nonce: attempt.nonce, code_challenge_method: 'S256', code_challenge: attempt.challenge,
    })
    if (attempt.clientId === 'dynamic_agent_client') query.set('agent_name_hint', 'SimRep')
    if (tokens?.id_token) query.set('id_token_hint', tokens.id_token)
    if (saved?.email) query.set('login_hint', saved.email)
    if (tokens && this.flow === 'siwc' && !tokens.scopes?.includes('chatgpt.tokens.use.direct')) query.set('prompt', 'consent')
    if (this.flow === 'subscription') {
      query.delete('ext_agent_host_id')
      query.set('id_token_add_organizations', 'true'); query.set('codex_cli_simplified_flow', 'true'); query.set('originator', 'codex_cli_rs')
    }
    return { url: (this.flow === 'subscription' ? SUBSCRIPTION.authorize : AUTHORIZE) + '?' + query, completed }
    } finally { this.starting = false }
  }

  record(tokens, previous) {
    const tokenType = tokens.token_type ?? (this.flow === 'subscription' ? 'Bearer' : undefined)
    if (typeof tokens.access_token !== 'string' || !tokens.access_token || typeof tokenType !== 'string' || tokenType.toLowerCase() !== 'bearer' ||
      !Number.isFinite(tokens.expires_in) || tokens.expires_in <= 0) {
      throw new BridgeError('AUTH_RESPONSE', 'Invalid OAuth token response.')
    }
    const chatgptAccountId = this.flow === 'subscription' ? subscriptionAccount(tokens.access_token) : undefined
    if (this.flow === 'subscription' && !chatgptAccountId) throw new BridgeError('AUTH_RESPONSE', 'ChatGPT did not return an account-scoped Codex credential.')
    if (previous.chatgptAccountId && previous.chatgptAccountId !== chatgptAccountId) {
      throw new BridgeError('AUTH_EXPIRED', 'The refreshed ChatGPT account did not match. Sign in again.')
    }
    return {
      ...previous,
      access_token: tokens.access_token,
      refresh_token: tokens.refresh_token || previous.refresh_token,
      id_token: tokens.id_token || previous.id_token,
      token_type: tokenType,
      ...(chatgptAccountId && { chatgptAccountId, apiBaseUrl: SUBSCRIPTION.apiBaseUrl }),
      scopes: typeof tokens.scope === 'string' ? tokens.scope.split(/\s+/).filter(Boolean) : previous.scopes || [],
      expiresAt: Date.now() + tokens.expires_in * 1000,
    }
  }

  async access(account) {
    return this.store.locked(async () => {
      let record = await this.store.credential(account)
      if (!record) throw new BridgeError('AUTH_REQUIRED', 'Continue with ChatGPT to sign in.')
      if (this.flow === 'siwc' && !record.scopes.includes('chatgpt.tokens.use.direct')) throw new BridgeError('PLAN_PERMISSION_REQUIRED', 'ChatGPT plan use is not enabled. Continue with ChatGPT to authorize it.')
      if (record.expiresAt < Date.now() + 60_000) {
        if (!record.refresh_token) {
          await this.store.remove('account-' + account)
          throw new BridgeError('AUTH_EXPIRED', 'ChatGPT session expired. Sign in again.')
        }
        try {
          const tokens = await this.post({
            grant_type: 'refresh_token', client_id: record.clientId, refresh_token: record.refresh_token, ...(this.flow === 'siwc' && { resource: RESOURCE }),
          })
          if (tokens.id_token) await validateIdentity(tokens.id_token, record.clientId, undefined, this.jwks, record.subject)
          record = this.record(tokens, record)
          await this.store.write('account-' + account, record)
        } catch (error) {
          if (error.code === 'AUTH_EXPIRED' || TERMINAL_REFRESH.has(error.oauthCode) || [401, 403].includes(error.details?.status)) {
            await this.store.remove('account-' + account)
            throw new BridgeError('AUTH_EXPIRED', 'ChatGPT session expired. Continue with ChatGPT to sign in again.')
          }
          throw error
        }
      }
      if (this.flow === 'siwc' && !record.scopes.includes('chatgpt.tokens.use.direct')) throw new BridgeError('PLAN_PERMISSION_REQUIRED', 'ChatGPT plan use is not enabled.')
      return record
    })
  }

  async accounts() {
    const registrations = await this.registrations()
    return Promise.all(registrations.map(async r => {
      const record = await this.store.credential(r.id)
      return { id: r.id, provider: 'chatgpt', label: `${r.email || 'ChatGPT'} (${r.id.slice(0, 8)})`,
        ...(r.email ? { email: r.email } : {}), authenticated: Boolean(record),
        planEnabled: Boolean(record && (this.flow === 'subscription' || record.scopes.includes('chatgpt.tokens.use.direct'))) }
    }))
  }
  async registrations() { return (await this.store.accounts()).filter(r => (r.flow || 'siwc') === this.flow) }

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
