/*
 * OAuth flows adapted from @code-yeongyu/senpi-ai 2026.10.25-2:
 * dist/auth/oauth/{anthropic,anthropic-callback-listener,github-copilot,
 * xai,openrouter,kimi-coding,kimi-region,kimi-identity,devin,devin-token,
 * devin-callback,meta,cursor,radius,device-code,pkce,callback-server}.js
 * and dist/api/github-copilot-endpoint.js; modified for SimRep.
 * MIT License
 * Copyright (c) 2025 Mario Zechner (upstream pi-mono)
 * Copyright (c) 2026 Yeongyu Kim and senpi contributors
 * Permission is hereby granted, free of charge, to any person obtaining a copy
 * of this software and associated documentation files (the "Software"), to deal
 * in the Software without restriction, including without limitation the rights
 * to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
 * copies of the Software, and to permit persons to whom the Software is
 * furnished to do so, subject to the following conditions:
 * The above copyright notice and this permission notice shall be included in all
 * copies or substantial portions of the Software.
 * THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
 * IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
 * FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
 * AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
 * LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
 * OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
 * SOFTWARE.
 */
import { createServer } from 'node:http'
import { randomBytes, createHash, timingSafeEqual } from 'node:crypto'
import { setTimeout as delay } from 'node:timers/promises'
import { hostname, platform, arch, release } from 'node:os'
import { Store } from '../store.mjs'
import { BridgeError } from '../errors.mjs'
import { providerById } from '../providers/table.mjs'

export const AUTH = {
  claude: { client: '9d1c250a-e61b-44d9-88ed-5944d1962f5e', authorize: 'https://claude.ai/oauth/authorize',
    token: 'https://platform.claude.com/v1/oauth/token', port: 53692, callback: '/callback',
    scope: 'org:create_api_key user:profile user:inference user:sessions:claude_code user:mcp_servers user:file_upload' },
  'github-copilot': { client: 'Iv1.b507a08c87ecfe98', device: 'https://github.com/login/device/code',
    token: 'https://github.com/login/oauth/access_token', mint: 'https://api.github.com/copilot_internal/v2/token', scope: 'read:user' },
  xai: { client: 'b1a00492-073a-47ea-816f-4c329264a828', device: 'https://auth.x.ai/oauth2/device/code',
    token: 'https://auth.x.ai/oauth2/token', scope: 'openid profile email offline_access grok-cli:access api:access' },
  openrouter: { authorize: 'https://openrouter.ai/auth', token: 'https://openrouter.ai/api/v1/auth/keys', port: 0 },
  'kimi-coding': { client: '17e5f671-d194-4dfb-9706-5516cb48c098' },
  devin: { authorize: 'https://app.devin.ai/auth/cli/continue', token: 'https://api.devin.ai/auth/cli/token',
    port: 59653, fixedPort: true, callback: '/callback' },
  meta: { client: '1031625952748946', device: 'https://auth.meta.com/oidc/device/authorization/',
    token: 'https://auth.meta.com/oidc/device/token/', mint: 'https://api.meta.ai/muse-code/key' },
  cursor: { authorize: 'https://cursor.com/loginDeepControl', poll: 'https://api2.cursor.sh/auth/poll',
    token: 'https://api2.cursor.sh/auth/exchange_user_api_key' },
  radius: { client: 'pi-gateway', device: 'https://radius.pi.dev/v1/oauth/device',
    token: 'https://radius.pi.dev/v1/oauth/token', scope: 'gateway offline_access' },
}
export const COPILOT_HEADERS = {
  'User-Agent': 'GitHubCopilotChat/0.35.0', 'Editor-Version': 'vscode/1.107.0',
  'Editor-Plugin-Version': 'copilot-chat/0.35.0', 'Copilot-Integration-Id': 'vscode-chat',
}
const TERMINAL = new Set(['invalid_grant', 'invalid_refresh_token', 'token_expired', 'refresh_token_expired',
  'refresh_token_invalidated', 'refresh_token_reused', 'expired_token'])
const text = value => typeof value === 'string' && value.length > 0
const positive = value => typeof value === 'number' && Number.isFinite(value) && value > 0
const secret = () => randomBytes(32).toString('base64url')
const ascii = value => value.replace(/[^\x20-\x7e]/g, '') || 'unknown'
function same(a, b) {
  return typeof a === 'string' && typeof b === 'string' && Buffer.byteLength(a) === Buffer.byteLength(b) &&
    timingSafeEqual(Buffer.from(a), Buffer.from(b))
}
export function pkce() {
  const verifier = secret() + secret()
  return { verifier, challenge: createHash('sha256').update(verifier).digest('base64url'), state: secret(), nonce: secret() }
}
function cancelled() { return new BridgeError('CANCELLED', 'Sign-in cancelled.') }
function responseError(status, body, refresh = false) {
  const terminal = status === 401 || status === 403 || TERMINAL.has(body?.error)
  return new BridgeError(refresh && terminal ? 'AUTH_EXPIRED' : 'AUTH_HTTP',
    refresh && terminal ? 'The provider session expired. Sign in again.' : 'Provider authorization failed. Try signing in again.',
    { status })
}
export function verificationUri(raw, provider) {
  let url
  try { url = new URL(raw) } catch { throw new BridgeError('AUTH_RESPONSE', 'Invalid verification URL.') }
  const hosts = {
    'github-copilot': ['github.com'], xai: ['auth.x.ai', 'accounts.x.ai', 'x.ai'],
    'kimi-coding': ['auth.kimi.com', 'auth.kimi.ai', 'kimi.com', 'kimi.ai', 'www.kimi.com', 'www.kimi.ai'],
    meta: ['auth.meta.com', 'meta.com', 'www.meta.com'], radius: ['radius.pi.dev'],
  }
  if (url.protocol !== 'https:' || url.port || url.username || url.password || !hosts[provider]?.includes(url.hostname)) {
    throw new BridgeError('AUTH_RESPONSE', 'Untrusted provider verification URL.')
  }
  return url.href
}

export class ProviderAuth {
  constructor(id, { store = new Store(), fetcher = fetch, env = process.env, endpoints = {}, port = undefined,
    now = Date.now, wait = (ms, signal) => delay(ms, undefined, { signal }) } = {}) {
    this.id = id; this.info = providerById(id); this.store = store; this.fetcher = fetcher; this.env = env
    this.endpoints = endpoints; this.port = port; this.now = now; this.wait = wait
    this.name = 'provider-' + id; this.pending = undefined; this.generation = 0
  }
  config(region = undefined) {
    let config = { ...AUTH[this.id] }
    if (this.id === 'kimi-coding') {
      region = region || this.env.KIMI_CODE_REGION || 'mainland-cn'
      if (!['mainland-cn', 'global'].includes(region)) throw new BridgeError('INVALID_PARAMS', 'Kimi region must be mainland-cn or global.')
      const suffix = region === 'global' ? 'ai' : 'com'
      config = { ...config, region, device: `https://auth.kimi.${suffix}/api/oauth/device_authorization`,
        token: `https://auth.kimi.${suffix}/api/oauth/token`, baseUrl: `https://api.kimi.${suffix}/coding` }
    }
    return { ...config, ...this.endpoints }
  }
  async request(url, fields, { json = false, headers = {}, signal = undefined, refresh = false, get = false, raw = false } = {}) {
    let response
    try {
      response = await this.fetcher(url, { method: get ? 'GET' : 'POST', redirect: 'error',
        signal: signal ? AbortSignal.any([signal, AbortSignal.timeout(30_000)]) : AbortSignal.timeout(30_000),
        headers: { accept: 'application/json', ...(!get && { 'content-type': json ? 'application/json' : 'application/x-www-form-urlencoded' }), ...headers },
        ...(!get && { body: json ? JSON.stringify(fields) : new URLSearchParams(fields) }) })
    } catch {
      if (signal?.aborted) throw cancelled()
      throw new BridgeError('AUTH_HTTP', 'Provider authorization could not be reached.')
    }
    const body = await response.json().catch(() => ({}))
    if (raw) return { status: response.status, ok: response.ok, body }
    if (!response.ok || body?.error) throw responseError(response.status, body, refresh)
    if (!body || typeof body !== 'object' || Array.isArray(body)) throw new BridgeError('AUTH_RESPONSE', 'Invalid provider response.')
    return body
  }
  async identityHeaders() {
    const host = await this.store.host()
    return { 'User-Agent': 'KimiCLI/1.0', 'X-Msh-Platform': 'kimi_cli', 'X-Msh-Version': '1.0',
      'X-Msh-Device-Id': host.replace(/^urn:uuid:/, '').replaceAll('-', ''), 'X-Msh-Device-Name': ascii(hostname()),
      'X-Msh-Device-Model': ascii(`${platform()} ${release()} ${arch()}`), 'X-Msh-Os-Version': ascii(release()) }
  }
  tokenRecord(body, previous = {}, config = this.config()) {
    const access = body.access_token || (this.id === 'cursor' ? body.accessToken : undefined)
    if (!text(access) || (body.expires_in !== undefined && !positive(body.expires_in))) {
      throw new BridgeError('AUTH_RESPONSE', 'Provider did not return a valid access token.')
    }
    return { ...previous, access_token: access,
      refresh_token: body.refresh_token || body.refreshToken || previous.refresh_token,
      expiresAt: this.now() + (body.expires_in || 3600) * 1000,
      ...(config.region && { region: config.region, baseUrl: config.baseUrl }),
    }
  }
  async save(record, generation) {
    await this.store.locked(async () => {
      if (generation !== this.generation) throw cancelled()
      await this.store.write(this.name, record)
    })
    return this.id + '-local'
  }
  async begin(_account = undefined, options = {}) {
    if (this.pending) throw new BridgeError('BUSY', 'A sign-in is already in progress for this provider.')
    if (this.info?.authKind === 'api-key') throw new BridgeError('ENV_API_KEY', 'Set the provider API key in the app environment.')
    if (!AUTH[this.id] && this.id !== 'kiro') throw new BridgeError('PROVIDER_UNSUPPORTED', this.info?.unsupportedReason || 'Provider sign-in is unavailable.')
    const controller = new AbortController(), generation = this.generation
    this.pending = controller
    try {
      const config = this.config(options.region)
      let login
      if (config.device) login = await this.device(config, controller.signal)
      else if (this.id === 'cursor') login = this.cursor(config, controller.signal)
      else login = await this.browser(config, controller.signal)
      const completed = login.completed.then(record => {
        if (controller.signal.aborted) throw cancelled()
        return this.save(record, generation)
      }).finally(() => { if (this.pending === controller) this.pending = undefined })
      completed.catch(() => {})
      return { ...login, completed }
    } catch (error) {
      controller.abort(); if (this.pending === controller) this.pending = undefined
      throw error
    }
  }
  async device(config, signal) {
    const headers = this.id === 'kimi-coding' ? await this.identityHeaders()
      : this.id === 'github-copilot' ? COPILOT_HEADERS : {}
    const device = await this.request(config.device, { client_id: config.client, ...(config.scope && { scope: config.scope }),
      ...(this.id === 'xai' && { referrer: 'pi' }) }, { headers, signal })
    if (!text(device.device_code) || !text(device.user_code) || !positive(device.expires_in)) {
      throw new BridgeError('AUTH_RESPONSE', 'Invalid device authorization response.')
    }
    const uri = verificationUri(device.verification_uri, this.id)
    const url = device.verification_uri_complete ? verificationUri(device.verification_uri_complete, this.id) : uri
    const completed = this.poll(config, device, headers, signal)
    completed.catch(() => {})
    return { url, deviceCode: { userCode: device.user_code, verificationUri: uri, expiresIn: device.expires_in }, completed }
  }
  async poll(config, device, headers, signal) {
    const deadline = this.now() + device.expires_in * 1000
    let interval = positive(device.interval) ? device.interval * 1000 : 5000
    while (this.now() < deadline) {
      try { await this.wait(Math.min(interval, deadline - this.now()), signal) } catch { throw cancelled() }
      if (signal.aborted) throw cancelled()
      if (this.now() >= deadline) break
      const { ok, status, body } = await this.request(config.token, { client_id: config.client, device_code: device.device_code,
        grant_type: 'urn:ietf:params:oauth:grant-type:device_code' }, { headers, signal, raw: true })
      if (body.error === 'authorization_pending') continue
      if (body.error === 'slow_down') { interval = Math.max(interval + 5000, positive(body.interval) ? body.interval * 1000 : 0); continue }
      if (['access_denied', 'authorization_denied'].includes(body.error)) throw new BridgeError('AUTH_DENIED', 'Device sign-in was declined.')
      if (body.error === 'expired_token') break
      if (!ok || body.error) throw responseError(status, body)
      if (this.id === 'github-copilot') return this.mintCopilot(body.access_token, signal)
      if (this.id === 'meta') return this.mintMeta(body.access_token, signal)
      return this.tokenRecord(body, {}, config)
    }
    throw new BridgeError('AUTH_TIMEOUT', 'Device authorization expired. Sign in again.')
  }
  async browser(config, signal) {
    const values = pkce()
    const callbackPath = this.id === 'openrouter' ? `/oauth/callback/${values.state}` : config.callback || '/oauth/callback'
    let redirectUri, claimed = false, resolve, reject, timer
    const completed = new Promise((yes, no) => { resolve = yes; reject = no })
    completed.catch(() => {})
    const close = () => { clearTimeout(timer); server.close(); signal.removeEventListener('abort', abort) }
    const abort = () => { reject(cancelled()); close() }
    const server = createServer((req, res) => {
      res.setHeader('cache-control', 'no-store'); res.setHeader('content-security-policy', "default-src 'none'")
      if (req.method !== 'GET' || req.headers.host !== new URL(redirectUri).host || req.url?.split('?')[0] !== callbackPath || req.headers.origin) {
        res.writeHead(404); res.end(); return
      }
      void (async () => {
        try {
          if (claimed || signal.aborted) throw new BridgeError('AUTH_CALLBACK', 'Sign-in callback was already used.')
          const url = new URL(req.url, redirectUri)
          for (const key of ['code', 'state', 'error']) if (url.searchParams.getAll(key).length > 1) throw new BridgeError('AUTH_CALLBACK', 'Duplicate callback parameter.')
          // OpenRouter returns no state; its unpredictable, one-time path is
          // the CSRF binding and PKCE binds the returned code to this attempt.
          if (this.id !== 'openrouter' && !same(url.searchParams.get('state'), values.state)) throw new BridgeError('AUTH_CALLBACK', 'Sign-in callback state did not match.')
          claimed = true
          if (url.searchParams.has('error')) throw new BridgeError('AUTH_DENIED', 'Browser sign-in was declined.')
          const code = url.searchParams.get('code')
          if (!text(code)) throw new BridgeError('AUTH_CALLBACK', 'Missing authorization code.')
          const body = this.id === 'openrouter' ? { code, code_verifier: values.verifier, code_challenge_method: 'S256' }
            : this.id === 'kiro' ? { code, code_verifier: values.verifier, redirect_uri: redirectUri }
            : this.id === 'devin' ? { code, code_verifier: values.verifier }
              : { grant_type: 'authorization_code', client_id: config.client, code, state: values.state, redirect_uri: redirectUri, code_verifier: values.verifier }
          const data = await this.request(config.token, body, { json: true, signal,
            ...(this.id === 'kiro' && { headers: { 'User-Agent': 'Kiro-CLI' } }) })
          let record
          if (this.id === 'openrouter' || this.id === 'devin') {
            const token = this.id === 'openrouter' ? data.key : data.token
            if (!text(token)) throw new BridgeError('AUTH_RESPONSE', 'Provider did not return a credential.')
            record = { access_token: token, expiresAt: this.id === 'openrouter' ? Number.MAX_SAFE_INTEGER : this.jwtExpiry(token, 365 * 86400_000) }
          } else record = this.tokenRecord(data, {}, config)
          if (signal.aborted) throw cancelled()
          res.end('Signed in to SimRep. You can close this window.')
          resolve(record)
        } catch (error) { res.writeHead(400); res.end('Sign-in failed. Return to SimRep.'); reject(error) }
        finally { close() }
      })()
    })
    const listen = port => new Promise((yes, no) => {
      const fail = error => { server.removeListener('listening', ready); no(error) }
      const ready = () => { server.removeListener('error', fail); yes(undefined) }
      server.once('error', fail); server.once('listening', ready); server.listen(port, '127.0.0.1')
    })
    const preferred = this.port ?? config.port ?? 0
    try { await listen(preferred) } catch (error) {
      if (error.code !== 'EADDRINUSE' || config.fixedPort || preferred === 0) {
        server.close()
        throw new BridgeError('AUTH_CALLBACK', 'Required provider callback port is unavailable. Close the other sign-in and retry.')
      }
      await listen(0)
    }
    const address = server.address()
    if (!address || typeof address === 'string') throw new BridgeError('AUTH_CALLBACK', 'Cannot open sign-in callback.')
    // Anthropic registers localhost redirect URIs, while the socket remains
    // IPv4 loopback-only, matching its first-party client's redirect contract.
    redirectUri = `http://${['claude', 'kiro'].includes(this.id) ? 'localhost' : '127.0.0.1'}:${address.port}${callbackPath}${this.id === 'kiro' ? '?login_option=google' : ''}`
    signal.addEventListener('abort', abort, { once: true })
    if (signal.aborted) abort()
    timer = setTimeout(() => { reject(new BridgeError('AUTH_TIMEOUT', 'Browser sign-in expired.')); close() }, 600_000)
    const url = new URL(config.authorize)
    if (this.id === 'openrouter') url.search = new URLSearchParams({ callback_url: redirectUri, code_challenge: values.challenge, code_challenge_method: 'S256' }).toString()
    else if (this.id === 'kiro') url.search = new URLSearchParams({ redirect_uri: new URL(redirectUri).origin,
      code_challenge: values.challenge, code_challenge_method: 'S256', state: values.state, nonce: values.nonce, redirect_from: 'kirocli' }).toString()
    else url.search = new URLSearchParams({ response_type: 'code', redirect_uri: redirectUri,
      code_challenge: values.challenge, code_challenge_method: 'S256', state: values.state, nonce: values.nonce,
      ...(config.client && { client_id: config.client }), ...(config.scope && { scope: config.scope }),
      ...(this.id === 'claude' && { code: 'true' }), ...(this.id === 'devin' && { prompt: 'select_account' }) }).toString()
    return { url: url.href, completed }
  }
  jwtExpiry(token, fallback = 3600_000) {
    // Only an expiry hint, never trusted identity or authorization.
    try {
      const exp = JSON.parse(Buffer.from(token.split('.')[1], 'base64url').toString()).exp
      if (positive(exp)) return exp * 1000
    } catch { /* Opaque tokens use the provider's documented lifetime. */ }
    return this.now() + fallback
  }
  cursor(config, signal) {
    const values = pkce()
    const url = new URL(config.authorize)
    url.search = new URLSearchParams({ challenge: values.challenge, uuid: values.state, mode: 'login', redirectTarget: 'cli' }).toString()
    const completed = (async () => {
      const endpoint = new URL(config.poll); endpoint.search = new URLSearchParams({ uuid: values.state, verifier: values.verifier }).toString()
      let interval = 1000
      for (let i = 0; i < 150; i++) {
        try { await this.wait(interval, signal) } catch { throw cancelled() }
        if (signal.aborted) throw cancelled()
        const { status, ok, body } = await this.request(endpoint, undefined, { get: true, signal, raw: true })
        if (ok) return { ...this.tokenRecord(body), expiresAt: this.jwtExpiry(body.accessToken) }
        if (status !== 404 && status !== 429) throw responseError(status, body)
        interval = Math.min(interval * 1.2, 10_000)
      }
      throw new BridgeError('AUTH_TIMEOUT', 'Cursor sign-in expired.')
    })()
    completed.catch(() => {})
    return { url: url.href, completed }
  }
  async mintCopilot(githubToken, signal = undefined) {
    if (!text(githubToken)) throw new BridgeError('AUTH_RESPONSE', 'Missing GitHub credential.')
    const config = this.config()
    const data = await this.request(config.mint, undefined, { get: true, signal, refresh: true,
      headers: { ...COPILOT_HEADERS, authorization: `Bearer ${githubToken}` } })
    if (!text(data.token) || !positive(data.expires_at)) throw new BridgeError('AUTH_RESPONSE', 'Invalid Copilot token response.')
    const field = key => data.token.split(';').find(v => v.startsWith(key + '='))?.slice(key.length + 1)
    const endpoint = data.endpoints?.api || (field('proxy-ep') ? `https://${field('proxy-ep').replace(/^proxy\./, 'api.')}` : 'https://api.individual.githubcopilot.com')
    let base
    try { base = new URL(endpoint) } catch { throw new BridgeError('AUTH_RESPONSE', 'Invalid Copilot endpoint.') }
    if (base.protocol !== 'https:' || base.port || base.username || base.password ||
      !(base.hostname === 'api.githubcopilot.com' || base.hostname.endsWith('.githubcopilot.com'))) throw new BridgeError('AUTH_RESPONSE', 'Untrusted Copilot endpoint.')
    const baseUrl = base.href.replace(/\/$/, '')
    const catalog = await this.request(config.models || baseUrl + '/models', undefined, { get: true, signal,
      headers: { ...COPILOT_HEADERS, authorization: `Bearer ${data.token}`, 'X-GitHub-Api-Version': '2026-06-01' } })
    const models = (Array.isArray(catalog.data) ? catalog.data : []).filter(m =>
      text(m.id) && m.capabilities?.supports?.tool_calls === true && (m.model_picker_enabled === true || m.policy?.state === 'enabled') &&
      (!m.supported_endpoints || m.supported_endpoints.includes('/chat/completions'))).map(m => m.id)
    return { access_token: data.token, refresh_token: githubToken, expiresAt: data.expires_at * 1000, baseUrl, models, headers: COPILOT_HEADERS }
  }
  async mintMeta(identity, signal = undefined) {
    if (!text(identity)) throw new BridgeError('AUTH_RESPONSE', 'Missing Meta identity credential.')
    const data = await this.request(this.config().mint, {}, { json: true, signal, refresh: true,
      headers: { authorization: `Bearer ${identity}`, 'x-api-version': '1.0.0' } })
    if (!text(data.api_key)) throw new BridgeError('AUTH_RESPONSE', 'Meta did not issue a Model API key. Complete account setup with the provider.')
    return { access_token: data.api_key, refresh_token: identity, expiresAt: this.now() + 86400_000 }
  }
  async access(_account = undefined) {
    const key = this.info?.envKey && this.env[this.info.envKey]
    if (key) {
      const config = this.config()
      return { apiKey: key, ...(this.id === 'kimi-coding' && { baseUrl: config.baseUrl }) }
    }
    return this.store.locked(async () => {
      let record = await this.store.read(this.name)
      if (!record) throw new BridgeError('AUTH_REQUIRED', 'Sign in with the selected provider first.')
      if (record.expiresAt > this.now() + 60_000) return record
      try {
        if (this.id === 'github-copilot') record = await this.mintCopilot(record.refresh_token)
        else if (this.id === 'meta') record = await this.mintMeta(record.refresh_token)
        else {
          if (!record.refresh_token) throw new BridgeError('AUTH_EXPIRED', 'The provider session expired. Sign in again.')
          const config = this.config(record.region)
          const cursor = this.id === 'cursor'
          const kiro = this.id === 'kiro'
          const data = await this.request(kiro ? config.refresh : config.token, cursor ? {} : kiro ? { refreshToken: record.refresh_token } : { grant_type: 'refresh_token',
            client_id: config.client, refresh_token: record.refresh_token }, {
            json: cursor || kiro || this.id === 'claude', refresh: true,
            headers: cursor ? { authorization: `Bearer ${record.refresh_token}` } : this.id === 'kimi-coding' ? await this.identityHeadersUnlocked() : {},
          })
          record = this.tokenRecord(data, record, config)
          if (cursor) record.expiresAt = this.jwtExpiry(data.accessToken)
        }
        await this.store.write(this.name, record)
        return record
      } catch (error) {
        if (error.code === 'AUTH_EXPIRED') await this.store.remove(this.name)
        throw error
      }
    })
  }
  async identityHeadersUnlocked() {
    // access() already owns Store.locked; host.json was minted before device login.
    const host = await this.store.read('host')
    return { 'User-Agent': 'KimiCLI/1.0', 'X-Msh-Platform': 'kimi_cli', 'X-Msh-Version': '1.0',
      'X-Msh-Device-Id': host.id.replace(/^urn:uuid:/, '').replaceAll('-', ''),
      'X-Msh-Device-Name': ascii(hostname()), 'X-Msh-Device-Model': ascii(`${platform()} ${release()} ${arch()}`), 'X-Msh-Os-Version': ascii(release()) }
  }
  /** @returns {Promise<{id: string, provider: string, label: string, authenticated: boolean, planEnabled: boolean}[]>} */
  async accounts() {
    const key = this.info?.envKey && this.env[this.info.envKey]
    await this.store.init()
    const record = await this.store.read(this.name)
    const authenticated = Boolean(key || record)
    if (!authenticated) return []
    return [{ id: this.id + (key ? '-env' : '-local'), provider: this.id, label: this.info.label + (key ? ' (API key environment)' : ''),
      authenticated, planEnabled: authenticated && !key }]
  }
  async models() {
    await this.store.init()
    const record = await this.store.read(this.name)
    return record?.models ?? this.info.models
  }
  async logout(_account = undefined) {
    this.close()
    await this.store.locked(() => this.store.remove(this.name))
    if (this.info?.envKey && this.env[this.info.envKey]) throw new BridgeError('ENV_API_KEY', 'Remove the provider API key from the app environment to sign out.')
    return { signedOut: true, remoteRevocationConfirmed: false }
  }
  close() { this.generation++; this.pending?.abort(); this.pending = undefined }
}
export const createProviderAuth = (id, options = {}) => new ProviderAuth(id, options)
