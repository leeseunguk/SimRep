import test from 'node:test'
import assert from 'node:assert/strict'
import { createHash } from 'node:crypto'
import { createServer } from 'node:http'
import { mkdtemp, stat, chmod, readdir, rm } from 'node:fs/promises'
import os from 'node:os'
import path from 'node:path'
import { generateKeyPair, SignJWT, exportJWK, createRemoteJWKSet } from 'jose'
import { ChatGPTAuth, ISSUER, RESOURCE, SCOPE, attemptValues, validateCallback, validateIdentity } from '../oauth.mjs'
import { Store, accountId } from '../store.mjs'

const { privateKey, publicKey } = await generateKeyPair('RS256')
const jwk = { ...await exportJWK(publicKey), kid: 'local', alg: 'RS256', use: 'sig' }
async function token({ issuer = ISSUER, audience = 'oaiapp_test', nonce = 'test-nonce', subject = 'subject', expiry = '5m', key = privateKey } = {}) {
  return new SignJWT(nonce ? { nonce } : {}).setProtectedHeader({ alg: 'RS256', kid: 'local' })
    .setIssuer(issuer).setAudience(audience).setSubject(subject).setIssuedAt().setExpirationTime(expiry).sign(key)
}
async function fixtures(t) {
  const directory = await mkdtemp(path.join(os.tmpdir(), 'simrep-auth-'))
  const store = new Store(directory)
  const server = createServer((_req, res) => { res.setHeader('content-type', 'application/json'); res.end(JSON.stringify({ keys: [jwk] })) })
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve))
  const jwks = createRemoteJWKSet(new URL(`http://127.0.0.1:${server.address().port}/jwks`))
  t.after(async () => { server.closeAllConnections(); server.close(); await rm(directory, { recursive: true, force: true }) })
  return { store, jwks, directory }
}
function attempt() {
  return { ...attemptValues(), clientId: 'dynamic_agent_client', redirectUri: 'http://127.0.0.1:1234/auth/callback', consumed: false }
}
function callback(a, query = {}) {
  const url = new URL(a.redirectUri)
  url.search = new URLSearchParams({ state: a.state, code: 'code', client_id: 'oaiapp_test', ...query }).toString()
  return url
}

test('fresh PKCE S256, state and nonce are independent and correctly encoded', () => {
  const a = attemptValues(), b = attemptValues()
  assert.notEqual(a.state, b.state); assert.notEqual(a.nonce, b.nonce); assert.notEqual(a.verifier, b.verifier)
  assert.notEqual(a.state, a.nonce)
  assert.match(a.verifier, /^[A-Za-z0-9_-]{43,128}$/)
  assert.equal(a.challenge, createHash('sha256').update(a.verifier).digest('base64url'))
})

test('callback is one-time, exact URI/state bound and requires issued client identity', () => {
  const a = attempt()
  assert.deepEqual(validateCallback(callback(a), a), { code: 'code', client: 'oaiapp_test' })
  assert.throws(() => validateCallback(callback(a), a))
  for (const changed of [
    a => callback(a, { state: 'wrong' }),
    a => callback(a, { client_id: 'dynamic_agent_client' }),
    a => callback(a, { client_id: '' }),
    a => callback(a, { error: 'access_denied' }),
    a => callback(a, { code: '' }),
    a => new URL(callback(a).toString().replace('127.0.0.1', 'localhost')),
    a => new URL(callback(a).toString().replace('/auth/callback', '/callback')),
    a => { const url = callback(a); url.searchParams.append('state', a.state); return url },
  ]) {
    const pending = attempt()
    assert.throws(() => validateCallback(changed(pending), pending))
  }
  const expired = attempt(); expired.expiresAt = Date.now() - 1
  assert.throws(() => validateCallback(callback(expired), expired))
  const returning = attempt(); returning.clientId = 'oaiapp_existing'
  assert.throws(() => validateCallback(callback(returning), returning))
  const returning2 = attempt(); returning2.clientId = 'oaiapp_existing'
  const url = callback(returning2); url.searchParams.delete('client_id')
  assert.equal(validateCallback(url, returning2).client, returning2.clientId)
})

test('ID token signature, issuer, audience, expiration, nonce and account binding against local JWKS', async t => {
  const { jwks } = await fixtures(t)
  assert.equal((await validateIdentity(await token(), 'oaiapp_test', 'test-nonce', jwks)).sub, 'subject')
  const alien = await generateKeyPair('RS256')
  for (const options of [
    { issuer: 'https://evil.test' }, { audience: 'other-client' }, { nonce: 'wrong' },
    { nonce: null }, { expiry: Math.floor(Date.now() / 1000) - 30 }, { key: alien.privateKey },
  ]) await assert.rejects(validateIdentity(await token(options), 'oaiapp_test', 'test-nonce', jwks))
  await assert.rejects(validateIdentity(await token(), 'oaiapp_test', 'test-nonce', jwks, 'other-user'))
})

test('atomic 0600 per-account credential files, stable host and cross-process refresh exclusion', async t => {
  const { store, directory } = await fixtures(t)
  const first = await store.host()
  assert.equal(first, await new Store(directory).host())
  const id = accountId('client', 'user')
  await store.locked(() => store.write('account-' + id, { access_token: 'private' }))
  assert.deepEqual(await store.credential(id), { access_token: 'private' })
  if (process.platform !== 'win32') {
    assert.equal((await stat(path.join(directory, 'account-' + id + '.json'))).mode & 0o777, 0o600)
    assert.equal((await stat(directory)).mode & 0o777, 0o700)
    await chmod(path.join(directory, 'account-' + id + '.json'), 0o644)
    await assert.rejects(store.credential(id), /owner-only/)
  }
  assert.equal((await readdir(directory)).some(n => n.endsWith('.tmp')), false)
  await store.locked(async () => { await assert.rejects(new Store(directory).locked(() => {}), /locked/) })
})

test('real loopback login exchanges PKCE, validates identity before saving and retains separate registration', async t => {
  const { store, jwks } = await fixtures(t)
  let authorization
  const auth = new ChatGPTAuth({ store, jwks, fetcher: async (url, options) => {
    assert.equal(url, 'https://auth.openai.com/api/accounts/oauth/token')
    assert.equal(options.body.get('resource'), RESOURCE)
    assert.equal(options.body.get('client_id'), 'oaiapp_test')
    assert.equal(options.body.get('redirect_uri'), authorization.searchParams.get('redirect_uri'))
    assert.equal(createHash('sha256').update(options.body.get('code_verifier')).digest('base64url'), authorization.searchParams.get('code_challenge'))
    return Response.json({ id_token: await token({ nonce: authorization.searchParams.get('nonce') }), access_token: 'secret-access',
      refresh_token: 'secret-refresh', token_type: 'Bearer', expires_in: 3600, scope: SCOPE })
  } })
  t.after(() => auth.close())
  const login = await auth.begin()
  authorization = new URL(login.url)
  assert.equal(authorization.searchParams.get('client_id'), 'dynamic_agent_client')
  assert.equal(authorization.searchParams.get('agent_name_hint'), 'SimRep')
  const cb = new URL(authorization.searchParams.get('redirect_uri'))
  assert.equal(cb.hostname, '127.0.0.1')
  cb.search = new URLSearchParams({ state: authorization.searchParams.get('state'), client_id: 'oaiapp_test', code: 'code' }).toString()
  assert.equal((await fetch(cb)).status, 200)
  const id = await login.completed
  assert.equal((await auth.access(id)).access_token, 'secret-access')
  const display = await auth.accounts()
  assert.equal(display[0].planEnabled, true)
  assert.equal(JSON.stringify(display).includes('secret'), false)
  const again = await auth.begin(id)
  assert.equal(new URL(again.url).searchParams.get('client_id'), 'oaiapp_test')
  assert.equal(new URL(again.url).searchParams.has('agent_name_hint'), false)
  assert.ok(new URL(again.url).searchParams.get('id_token_hint'))
  auth.close()
  await assert.rejects(again.completed)
})

test('refreshes serialize and atomically rotate tokens; missing grant blocks inference; terminal errors retain mapping only', async t => {
  const { store, jwks } = await fixtures(t)
  const id = accountId('oaiapp_test', 'subject')
  await store.locked(async () => {
    await store.write('registration-' + id, { id, clientId: 'oaiapp_test', subject: 'subject' })
    await store.write('account-' + id, { id, clientId: 'oaiapp_test', subject: 'subject', nonce: 'old',
      access_token: 'expired', refresh_token: 'refresh-1', scopes: SCOPE.split(' '), expiresAt: 0 })
  })
  let refreshes = 0
  const auth = new ChatGPTAuth({ store, jwks, fetcher: async (_url, options) => {
    refreshes++
    assert.equal(options.body.get('refresh_token'), 'refresh-1')
    assert.equal(options.body.has('scope'), false)
    return Response.json({ access_token: 'new-access', refresh_token: 'refresh-2', token_type: 'Bearer', expires_in: 3600,
      id_token: await token({ nonce: null }) })
  } })
  const records = await Promise.all([auth.access(id), auth.access(id)])
  assert.equal(refreshes, 1)
  assert.equal(records[1].refresh_token, 'refresh-2')
  await store.locked(() => store.write('account-' + id, { ...records[0], scopes: ['openid'] }))
  await assert.rejects(auth.access(id), error => error.code === 'PLAN_PERMISSION_REQUIRED')
  await store.locked(() => store.write('account-' + id, { ...records[0], expiresAt: 0 }))
  auth.fetcher = async () => Response.json({ error: 'invalid_grant' }, { status: 400 })
  await assert.rejects(auth.access(id))
  assert.equal(await store.credential(id), undefined)
  assert.equal((await auth.accounts())[0].authenticated, false)
})

test('logout deletes tokens but preserves registration and reports unconfirmed remote revocation', async t => {
  const { store, jwks } = await fixtures(t)
  const id = accountId('client', 'user')
  await store.locked(async () => {
    await store.write('registration-' + id, { id, clientId: 'client', subject: 'user' })
    await store.write('account-' + id, { refresh_token: 'secret' })
  })
  const auth = new ChatGPTAuth({ store, jwks, fetcher: async () => { throw new Error('network') } })
  assert.equal((await auth.logout(id)).remoteRevocationConfirmed, false)
  assert.equal(await store.credential(id), undefined)
  assert.equal((await auth.accounts()).length, 1)
})

test('logout during a pending token exchange cannot resurrect credentials', async t => {
  const { store, jwks } = await fixtures(t)
  let announce, release
  const entered = new Promise(resolve => { announce = resolve })
  const response = new Promise(resolve => { release = resolve })
  const auth = new ChatGPTAuth({ store, jwks, fetcher: async () => { announce(); return response } })
  t.after(() => auth.close())
  const login = await auth.begin()
  await assert.rejects(auth.begin(), error => error.code === 'BUSY')
  const url = new URL(login.url)
  const cb = new URL(url.searchParams.get('redirect_uri'))
  cb.search = new URLSearchParams({ state: url.searchParams.get('state'), client_id: 'oaiapp_test', code: 'code' }).toString()
  const callbackResponse = fetch(cb)
  await entered
  auth.close()
  await assert.rejects(login.completed, error => error.code === 'CANCELLED')
  release(Response.json({ id_token: await token({ nonce: url.searchParams.get('nonce') }), access_token: 'never-save',
    refresh_token: 'never-save', token_type: 'Bearer', expires_in: 3600, scope: SCOPE }))
  assert.equal((await callbackResponse).status, 400)
  assert.equal((await store.accounts()).length, 0)
})
