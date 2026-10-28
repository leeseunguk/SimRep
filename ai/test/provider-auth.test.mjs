import test from 'node:test'
import assert from 'node:assert/strict'
import { createServer, request } from 'node:http'
import { createHash } from 'node:crypto'
import { mkdtemp, rm, stat } from 'node:fs/promises'
import path from 'node:path'
import os from 'node:os'
import { ProviderAuth, pkce, verificationUri } from '../auth/providers.mjs'
import { KiroAuth } from '../auth/kiro.mjs'
import { Store } from '../store.mjs'
import { ClaudeSession } from '../claude.mjs'

const uris = {
  'github-copilot': 'https://github.com/login/device', xai: 'https://auth.x.ai/activate',
  'kimi-coding': 'https://auth.kimi.ai/device', meta: 'https://auth.meta.com/device', radius: 'https://radius.pi.dev/device',
}
async function fixture(t, id, respond = undefined) {
  const directory = await mkdtemp(path.join(os.tmpdir(), 'simrep-provider-auth-'))
  const store = new Store(directory)
  const calls = []
  const server = createServer((req, res) => {
    void (async () => {
      let raw = ''
      for await (const data of req) raw += data
      const body = req.headers['content-type']?.includes('application/json') ? JSON.parse(raw || '{}') : Object.fromEntries(new URLSearchParams(raw))
      const call = { url: req.url, headers: req.headers, body }; calls.push(call)
      const response = respond ? await respond(call, calls) : { body: { access_token: 'access-private', refresh_token: 'refresh-private', expires_in: 3600 } }
      res.writeHead(response.status || 200, { 'content-type': 'application/json' }); res.end(JSON.stringify(response.body))
    })().catch(() => { res.writeHead(500); res.end('{}') })
  })
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve))
  const origin = `http://127.0.0.1:${server.address().port}`
  const clock = { ms: 1_000_000, waits: [] }
  const options = { store, env: {}, port: 0, now: () => clock.ms,
    wait: async (ms, signal) => { signal?.throwIfAborted(); clock.waits.push(ms); clock.ms += ms },
    endpoints: { authorize: origin + '/authorize', device: uris[id] ? origin + '/device' : undefined,
      token: origin + '/token', mint: origin + '/mint', models: origin + '/models', poll: origin + '/poll', refresh: origin + '/refresh' } }
  const auth = id === 'kiro' ? new KiroAuth(options) : new ProviderAuth(id, options)
  t.after(async () => { auth.close(); server.closeAllConnections(); server.close(); await rm(directory, { recursive: true, force: true }) })
  return { auth, calls, store, origin, clock, directory, options }
}
function callback(url) {
  const target = new URL(url)
  return new Promise((resolve, reject) => {
    const outgoing = request({ host: '127.0.0.1', port: target.port, path: target.pathname + target.search, headers: { host: target.host } }, response => {
      response.resume(); response.on('end', () => resolve(response.statusCode))
    })
    outgoing.on('error', reject); outgoing.end()
  })
}
function deviceResponse(id) { return { body: { device_code: 'device-private', user_code: 'ABCD-EFGH',
  verification_uri: uris[id], verification_uri_complete: uris[id] + '?user_code=ABCD-EFGH', expires_in: 900, interval: 1 } } }
function minted(id) {
  return id === 'github-copilot' ? { body: { token: 'tid=one;proxy-ep=proxy.individual.githubcopilot.com', expires_at: 9999999,
    endpoints: { api: 'https://api.individual.githubcopilot.com' } } } : { body: { api_key: 'mm_private' } }
}

test('fresh browser attempts have independent state nonce and S256 PKCE', () => {
  const a = pkce(), b = pkce()
  for (const key of ['state', 'nonce', 'verifier']) assert.notEqual(a[key], b[key])
  assert.notEqual(a.state, a.nonce); assert.notEqual(a.state, a.verifier)
  assert.equal(a.challenge, createHash('sha256').update(a.verifier).digest('base64url'))
})

for (const id of ['claude', 'openrouter', 'devin', 'kiro']) {
  test(`${id}: real loopback callback exchanges only bound PKCE code and stores 0600 credentials`, async t => {
    let loginUrl
    const { auth, calls, store, directory } = await fixture(t, id, call => {
      assert.equal(call.url, '/token')
      assert.equal(call.body.code, 'one-use-code')
      assert.equal(createHash('sha256').update(call.body.code_verifier).digest('base64url'), loginUrl.searchParams.get('code_challenge'))
      if (id === 'openrouter') return { body: { key: 'minted-private-key' } }
      if (id === 'devin') return { body: { token: 'devin-private-token' } }
      if (id === 'kiro') return { body: { accessToken: 'kiro-private', refreshToken: 'rotate-kiro', profileArn: 'arn:profile', expiresIn: 3600 } }
      assert.equal(call.body.client_id, '9d1c250a-e61b-44d9-88ed-5944d1962f5e')
      assert.equal(call.body.state, loginUrl.searchParams.get('state'))
      return { body: { access_token: 'access-private', refresh_token: 'refresh-private', expires_in: 3600 } }
    })
    const login = await auth.begin()
    loginUrl = new URL(login.url)
    let redirect = loginUrl.searchParams.get(id === 'openrouter' ? 'callback_url' : 'redirect_uri')
    if (id === 'kiro') redirect += '/oauth/callback?login_option=google'
    const cb = new URL(redirect); cb.searchParams.set('code', 'one-use-code')
    if (id !== 'openrouter') cb.searchParams.set('state', loginUrl.searchParams.get('state'))
    assert.equal(await callback(cb), 200)
    assert.equal(await login.completed, id + '-local')
    assert.equal(calls.length, 1)
    assert.ok((await auth.access()).access_token)
    assert.equal(JSON.stringify(await auth.accounts()).includes('private'), false)
    assert.ok(await store.read('provider-' + id))
    if (process.platform !== 'win32') assert.equal((await stat(path.join(directory, 'provider-' + id + '.json'))).mode & 0o777, 0o600)
    await assert.rejects(callback(cb))
  })
  test(`${id}: callback state/path are bound and logout cancels a pending browser attempt`, async t => {
    const { auth, calls, store } = await fixture(t, id)
    const login = await auth.begin()
    const url = new URL(login.url)
    let redirect = url.searchParams.get(id === 'openrouter' ? 'callback_url' : 'redirect_uri')
    if (id === 'kiro') redirect += '/oauth/callback?login_option=google'
    const cb = new URL(redirect); cb.searchParams.set('code', 'code'); cb.searchParams.set('state', 'wrong-state')
    if (id === 'openrouter') cb.pathname += '-wrong'
    const rejected = id === 'openrouter' ? undefined : assert.rejects(login.completed, error => error.code === 'AUTH_CALLBACK')
    assert.equal(await callback(cb), id === 'openrouter' ? 404 : 400)
    if (rejected) await rejected
    else { auth.close(); await assert.rejects(login.completed, error => error.code === 'CANCELLED') }
    assert.equal(calls.length, 0); assert.equal(await store.read('provider-' + id), undefined)
    const fresh = await auth.begin(); const pending = assert.rejects(fresh.completed, error => error.code === 'CANCELLED')
    await auth.logout(); await pending
  })
}

for (const id of Object.keys(uris)) {
  test(`${id}: device polling handles pending slow_down and stores the resulting credential`, async t => {
    let polls = 0
    const { auth, calls, clock, store } = await fixture(t, id, call => {
      if (call.url === '/device') return deviceResponse(id)
      if (call.url === '/mint') return minted(id)
      if (call.url === '/models') return { body: { data: [
        { id: 'enabled-tool', model_picker_enabled: true, capabilities: { supports: { tool_calls: true } }, supported_endpoints: ['/chat/completions'] },
        { id: 'no-tools', model_picker_enabled: true, capabilities: { supports: { tool_calls: false } } },
      ] } }
      assert.equal(call.body.grant_type, 'urn:ietf:params:oauth:grant-type:device_code')
      if (++polls === 1) return { status: 400, body: { error: 'authorization_pending' } }
      if (polls === 2) return { status: 400, body: { error: 'slow_down' } }
      return { body: { access_token: id === 'meta' ? 'identity-private' : 'access-private', refresh_token: 'refresh-private', expires_in: 3600 } }
    })
    const login = await auth.begin(undefined, { region: 'global' })
    assert.deepEqual(login.deviceCode, { userCode: 'ABCD-EFGH', verificationUri: uris[id], expiresIn: 900 })
    assert.match(login.url, /^https:/)
    await login.completed
    assert.equal(polls, 3); assert.deepEqual(clock.waits, [1000, 1000, 6000])
    const credential = await auth.access()
    if (id === 'kimi-coding') {
      assert.equal(credential.region, 'global'); assert.equal(credential.baseUrl, 'https://api.kimi.ai/coding')
      assert.ok(calls[0].headers['x-msh-device-id'])
      assert.equal(calls[0].headers['x-msh-device-id'], (await auth.identityHeaders())['X-Msh-Device-Id'])
    }
    if (id === 'github-copilot') assert.deepEqual(await auth.models(), ['enabled-tool'])
    if (id === 'meta') assert.equal(credential.access_token, 'mm_private')
    assert.equal(JSON.stringify(await auth.accounts()).includes('private'), false)
    await auth.logout(); assert.equal(await store.read('provider-' + id), undefined)
  })
  for (const [error, expected] of [['access_denied', 'AUTH_DENIED'], ['expired_token', 'AUTH_TIMEOUT']]) {
    test(`${id}: device ${error} is stable and does not save credentials`, async t => {
      const { auth, store } = await fixture(t, id, call => call.url === '/device' ? deviceResponse(id) : { status: 400, body: { error, error_description: 'secret-response' } })
      const login = await auth.begin()
      await assert.rejects(login.completed, e => e.code === expected && !JSON.stringify(e).includes('secret-response'))
      assert.equal(await store.read('provider-' + id), undefined)
    })
  }
}

test('device polling expiry uses the exact deadline and cancellation interrupts its wait', async t => {
  const { auth, clock, store } = await fixture(t, 'xai', call => call.url === '/device'
    ? { body: { ...deviceResponse('xai').body, expires_in: 2, interval: 1 } } : { status: 400, body: { error: 'authorization_pending' } })
  const login = await auth.begin()
  await assert.rejects(login.completed, e => e.code === 'AUTH_TIMEOUT')
  assert.deepEqual(clock.waits, [1000, 1000])
  let entered, release
  const waiting = new Promise(resolve => { entered = resolve })
  auth.wait = (_ms, signal) => new Promise((resolve, reject) => { release = resolve; entered(); signal.addEventListener('abort', () => reject(signal.reason), { once: true }) })
  const cancelled = await auth.begin()
  await waiting
  const stopped = assert.rejects(cancelled.completed, e => e.code === 'CANCELLED')
  auth.close(); release(); await stopped
  assert.equal(await store.read('provider-xai'), undefined)
})

test('verification URLs are HTTPS and fixed first-party origins only', () => {
  for (const bad of ['file:///tmp/x', 'http://github.com/login/device', 'https://github.com.evil.test/login/device', 'https://user@github.com/login/device']) {
    assert.throws(() => verificationUri(bad, 'github-copilot'), e => e.code === 'AUTH_RESPONSE')
  }
})

test('Cursor uses PKCE browser polling and rotates refresh tokens without leaking them', async t => {
  let poll = 0
  const { auth, store, calls, clock } = await fixture(t, 'cursor', call => {
    if (call.url.startsWith('/poll')) {
      const url = new URL(call.url, 'http://loopback')
      assert.ok(url.searchParams.get('uuid')); assert.ok(url.searchParams.get('verifier'))
      return ++poll === 1 ? { status: 404, body: {} } : { body: { accessToken: 'cursor-private', refreshToken: 'cursor-refresh' } }
    }
    assert.equal(call.headers.authorization, 'Bearer cursor-refresh')
    return { body: { accessToken: 'cursor-renewed' } }
  })
  const login = await auth.begin()
  const url = new URL(login.url); assert.equal(url.searchParams.get('mode'), 'login')
  await login.completed; assert.equal(poll, 2); assert.deepEqual(clock.waits, [1000, 1200])
  const saved = await store.read('provider-cursor')
  await store.locked(() => store.write('provider-cursor', { ...saved, expiresAt: 0 }))
  const result = await auth.access()
  assert.equal(result.access_token, 'cursor-renewed'); assert.equal(result.refresh_token, 'cursor-refresh')
  assert.equal(JSON.stringify(await auth.accounts()).includes('cursor-private'), false)
  assert.equal(calls.length, 3)
})

for (const id of ['claude', 'xai', 'kimi-coding', 'radius', 'kiro', 'github-copilot', 'meta']) {
  test(`${id}: refresh serializes rotation, preserves temporary failures and clears terminal credentials`, async t => {
    const { auth, calls, store } = await fixture(t, id, call => {
      if (call.url === '/mint') return minted(id)
      if (call.url === '/models') return { body: { data: [] } }
      return { body: id === 'kiro' ? { accessToken: 'rotated-private', refreshToken: 'rotated-refresh', expiresIn: 3600 }
        : { access_token: 'rotated-private', refresh_token: 'rotated-refresh', expires_in: 3600 } }
    })
    await store.host()
    await store.locked(() => store.write('provider-' + id, { access_token: 'old', refresh_token: 'original-refresh', expiresAt: 0,
      ...(id === 'kimi-coding' && { region: 'global', baseUrl: 'https://api.kimi.ai/coding' }) }))
    const results = await Promise.all([auth.access(), auth.access()])
    assert.equal(calls.filter(c => c.url !== '/models').length, 1)
    assert.equal(results[0].access_token, results[1].access_token)
    if (id === 'kimi-coding') assert.equal(results[0].baseUrl, 'https://api.kimi.ai/coding')
    await store.locked(() => store.write('provider-' + id, { ...results[0], expiresAt: 0 }))
    auth.fetcher = async () => Response.json({ error: 'server_error', error_description: 'secret' }, { status: 503 })
    await assert.rejects(auth.access(), e => e.code === 'AUTH_HTTP' && !JSON.stringify(e).includes('secret'))
    assert.ok(await store.read('provider-' + id))
    auth.fetcher = async () => Response.json({ error: 'invalid_grant', error_description: 'secret' }, { status: 400 })
    await assert.rejects(auth.access(), e => e.code === 'AUTH_EXPIRED')
    assert.equal(await store.read('provider-' + id), undefined)
  })
}

test('API key alternatives are environment-only; SDK receives only the selected Claude credential', async t => {
  const { auth, store, directory } = await fixture(t, 'claude')
  auth.env.ANTHROPIC_API_KEY = 'key-private'
  assert.deepEqual(await auth.access(), { apiKey: 'key-private' })
  assert.equal((await auth.accounts())[0].planEnabled, false)
  await assert.rejects(auth.logout(), e => e.code === 'ENV_API_KEY')
  let received
  const gate = { config: {}, close() {} }
  const queryFn = ({ options }) => {
    received = options
    const iterator = (async function* () { yield { type: 'result', is_error: false } })()
    iterator.close = () => {}
    return iterator
  }
  const session = new ClaudeSession({ auth, account: 'claude-env', workspace: directory, model: 'sonnet', gate, emit() {},
    systemPrompt: 'Harness instructions', queryFn })
  assert.equal(await session.send('hello'), 'completed')
  assert.equal(received.env.ANTHROPIC_API_KEY, 'key-private')
  assert.equal(received.env.CLAUDE_CODE_OAUTH_TOKEN, undefined)
  assert.equal(received.systemPrompt, 'Harness instructions')
  assert.equal(received.env.CLAUDE_CONFIG_DIR, path.join(store.directory, 'claude-sdk'))
  delete auth.env.ANTHROPIC_API_KEY
  await store.locked(() => store.write('provider-claude', { access_token: 'oauth-private', expiresAt: Number.MAX_SAFE_INTEGER }))
  await session.send('hello again')
  assert.equal(received.env.CLAUDE_CODE_OAUTH_TOKEN, 'oauth-private')
  assert.equal(received.env.ANTHROPIC_API_KEY, undefined)
  session.close()
})
