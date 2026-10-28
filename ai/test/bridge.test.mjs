import test from 'node:test'
import assert from 'node:assert/strict'
import { EventEmitter, once } from 'node:events'
import { mkdtemp, writeFile, rm } from 'node:fs/promises'
import path from 'node:path'
import os from 'node:os'
import { Bridge } from '../bridge.mjs'
import { ClaudeSession } from '../claude.mjs'
import { ChatGPTSession } from '../chatgpt.mjs'
import { RpcProcess } from '../rpc.mjs'
import { Store } from '../store.mjs'
import { publicError, providerDiagnostic } from '../errors.mjs'
import { PROVIDERS } from '../providers/table.mjs'

async function setup(t, { tool = 'read_csv_summary', args = { path: 'results.csv' } } = {}) {
  const workspace = await mkdtemp(path.join(os.tmpdir(), 'simrep-chat-'))
  await writeFile(path.join(workspace, 'results.csv'), 'reason,time_us\nplane,2\nplane,4\n')
  const events = new EventEmitter()
  let optionsSeen
  const fakeSDK = ({ options }) => {
    optionsSeen = options
    return (async function* () {
      assert.deepEqual(options.tools, [])
      assert.equal(options.strictMcpConfig, true)
      assert.deepEqual(options.settingSources, [])
      const server = options.mcpServers.simrep
      const mcp = new RpcProcess(server.command, server.args, { env: { ...process.env, ...server.env } })
      try {
        await mcp.request('initialize', { protocolVersion: '2025-06-18', capabilities: {}, clientInfo: { name: 'test', version: '1' } })
        mcp.send({ jsonrpc: '2.0', method: 'notifications/initialized' })
        yield { type: 'system', subtype: 'init', session_id: 'sdk-session' }
        const result = await mcp.request('tools/call', { name: tool, arguments: args })
        yield { type: 'stream_event', event: { type: 'content_block_delta', delta: { type: 'text_delta', text: result.content[0].text } } }
        yield { type: 'result', is_error: Boolean(result.isError) }
      } finally { mcp.close() }
    })()
  }
  // AsyncGenerator is the real query surface; close is supplied by the SDK.
  const queryFn = input => {
    const stream = fakeSDK(input)
    stream.close = () => {}
    return stream
  }
  const account = { id: 'claude-local', provider: 'claude', label: 'Test', authenticated: true, planEnabled: true }
  const credentialDir = await mkdtemp(path.join(os.tmpdir(), 'simrep-sdk-'))
  t.after(() => rm(credentialDir, { recursive: true, force: true }))
  const claude = { store: new Store(credentialDir), access: async () => ({ access_token: 'fake-sdk-token' }),
    accounts: async () => [account], close() {}, logout: async () => ({ signedOut: true }) }
  const bridge = new Bridge({ claude, notify: message => events.emit('bridge:' + (message.params?.type || message.method), message.params),
    createClaude: opts => new ClaudeSession({ ...opts, queryFn }) })
  t.after(async () => { bridge.close(); await rm(workspace, { recursive: true, force: true }) })
  const session = await bridge.dispatch('chat.start', { provider: 'claude', workspace })
  return { bridge, session, events, workspace, options: () => optionsSeen }
}
function event(events, type) {
  return once(events, 'bridge:' + type, { signal: AbortSignal.timeout(10_000) }).then(([payload]) => payload)
}

test('mock Agent SDK drives real workspace-restricted MCP and streams tool/delta/done events', async t => {
  const { bridge, session, events, options } = await setup(t)
  const done = event(events, 'done'), called = event(events, 'tool_call'), result = event(events, 'tool_result'), delta = event(events, 'delta')
  const turn = await bridge.dispatch('chat.send', { sessionId: session.sessionId, text: 'Summarize results.csv' })
  await assert.rejects(bridge.dispatch('chat.send', { sessionId: session.sessionId, text: 'racing' }), error => error.code === 'BUSY')
  assert.equal((await done).status, 'completed')
  assert.equal((await called).name, 'read_csv_summary')
  assert.equal((await result).result.isError, false)
  const text = (await delta).text
  assert.equal(JSON.parse(text).row_count, 2)
  assert.equal(JSON.parse(text).numeric.time_us.mean, 3)
  assert.ok(turn.turnId)
  assert.equal((await options().canUseTool('Bash', {})).behavior, 'deny')
})

test('mutating tools cannot execute without user approval and cancellation denies outstanding approval', async t => {
  const { bridge, session, events } = await setup(t, { tool: 'gem2pa', args: { input: 'not-there.gem' } })
  const approval = event(events, 'approval'), done = event(events, 'done')
  await bridge.dispatch('chat.send', { sessionId: session.sessionId, text: 'Overwrite geometry' })
  const request = await approval
  assert.equal(request.name, 'gem2pa')
  await bridge.dispatch('chat.cancel', { sessionId: session.sessionId })
  assert.equal((await done).status, 'interrupted')
  await assert.rejects(bridge.dispatch('approval.resolve', { approvalId: request.approvalId, allow: true }))
})

test('MCP refuses path traversal through the mocked provider and never reports success', async t => {
  const { bridge, session, events } = await setup(t, { args: { path: '../outside.csv' } })
  const done = event(events, 'done'), result = event(events, 'tool_result'), error = event(events, 'error')
  await bridge.dispatch('chat.send', { sessionId: session.sessionId, text: 'Read outside' })
  assert.equal((await result).result.isError, true)
  assert.equal((await done).status, 'failed')
  assert.equal((await error).code, 'PROVIDER_ERROR')
})

test('RPC contract rejects malformed methods and never relays arbitrary provider exception text', async t => {
  const { bridge } = await setup(t)
  assert.equal((await bridge.handle({ jsonrpc: '2.0', id: 1, method: 'bad' })).error.code, -32601)
  assert.equal((await bridge.handle({ jsonrpc: '2.0', id: 2, method: 'chat.start', params: { provider: 'bad' } })).error.code, -32602)
  assert.equal((await bridge.handle({ method: 'auth.status' })).error.code, -32600)
  assert.equal(JSON.stringify(publicError(new Error('Bearer SECRET'))).includes('SECRET'), false)
  const diagnostic = providerDiagnostic({
    message: '403 {"detail":"not eligible","code":"subscription_sharing_user_not_eligible"} request_id=req_1 Bearer SECRET',
    codexErrorInfo: { httpConnectionFailed: { httpStatusCode: 403 } },
  }, ['SECRET'])
  assert.equal(JSON.stringify(diagnostic).includes('SECRET'), false)
  assert.equal(diagnostic.details.upstream.httpConnectionFailed.httpStatusCode, 403)
  assert.match(diagnostic.message, /subscription_sharing_user_not_eligible/)
  assert.match(diagnostic.message, /req_1/)
})

test('ChatGPT app-server receives only child env token and restarts/resumes after token renewal', async t => {
  const directory = await mkdtemp(path.join(os.tmpdir(), 'simrep-codex-'))
  const store = new Store(directory); await store.init()
  t.after(() => rm(directory, { recursive: true, force: true }))
  const started = [], methods = [], events = []
  let access = 'access-1'
  const auth = { store, access: async () => ({ access_token: access }) }
  const rpcFactory = (command, args, options) => {
    started.push({ command, args, options })
    const rpc = {
      closed: false, onmessage: async () => {}, onclose: () => {},
      send() {}, close() { this.closed = true },
      async request(method, params) {
        methods.push(method)
        if (method === 'initialize') assert.deepEqual(params.clientInfo, { name: 'SimRep', title: 'SimRep', version: '0.1.0' })
        if (method.startsWith('thread/')) {
          assert.equal(params.cwd, path.join(directory, 'codex'))
          return { thread: { id: 'thread-1' } }
        }
        if (method === 'turn/start') {
          queueMicrotask(() => {
            void rpc.onmessage({ method: 'item/agentMessage/delta', params: { threadId: 'thread-1', delta: 'ok' } })
            void rpc.onmessage({ method: 'turn/completed', params: { threadId: 'thread-1', turn: { status: 'completed' } } })
          })
          return { turn: { id: 'turn-1' } }
        }
        return {}
      },
    }
    return rpc
  }
  const gate = { config: { command: 'node', args: ['proxy'], env: { SIMREP_WORKSPACE: directory } }, close() {} }
  const session = new ChatGPTSession({ auth, account: 'account', workspace: directory, model: 'model', gate, emit: event => events.push(event), rpcFactory })
  t.after(() => session.close())
  assert.equal(await session.send('first'), 'completed')
  access = 'access-2'
  assert.equal(await session.send('second'), 'completed')
  assert.equal(started.length, 2)
  assert.equal(started[0].options.env.ACCESS_TOKEN, 'access-1')
  assert.equal(started[1].options.env.ACCESS_TOKEN, 'access-2')
  assert.equal(started[0].args.join(' ').includes('access-1'), false)
  assert.ok(methods.includes('thread/resume'))
  assert.equal(events.filter(e => e.type === 'delta').length, 2)
})

test('provider discovery, device-code events, fixed usage URLs and unsupported providers preserve the protocol', async t => {
  const messages = []
  let finish
  const completed = new Promise(resolve => { finish = resolve })
  const auth = { 'github-copilot': {
    accounts: async () => [], models: async () => ['enabled-model'], close() {},
    begin: async () => ({ url: 'https://github.com/login/device', deviceCode: { userCode: 'ABCD-EFGH',
      verificationUri: 'https://github.com/login/device', expiresIn: 900 }, completed }),
  } }
  const bridge = new Bridge({ auth, notify: m => messages.push(m) })
  t.after(() => bridge.close())
  const metadata = await bridge.dispatch('auth.providers')
  assert.equal(metadata.providers.length, PROVIDERS.length)
  assert.equal(metadata.providers.find(p => p.id === 'claude').authKind, 'oauth')
  assert.equal(metadata.providers.find(p => p.id === 'openai').authKind, 'api-key')
  assert.equal(metadata.providers.find(p => p.id === 'cursor').toolCalling, false)
  assert.equal(JSON.stringify(metadata).includes('access_token'), false)
  const status = await bridge.dispatch('auth.status', { provider: 'github-copilot' })
  assert.deepEqual(status.providers[0].models, ['enabled-model'])
  await bridge.dispatch('auth.login', { provider: 'github-copilot' })
  assert.equal(messages[0].method, 'host.open_url')
  assert.deepEqual(messages[1].params, { provider: 'github-copilot', type: 'device_code', userCode: 'ABCD-EFGH',
    verificationUri: 'https://github.com/login/device', expiresIn: 900 })
  await bridge.dispatch('auth.openUsage', { provider: 'claude', url: 'https://evil.test' })
  assert.equal(messages.at(-1).params.url, 'https://claude.ai/settings/usage')
  await assert.rejects(bridge.dispatch('chat.start', { provider: 'cursor', workspace: '/tmp' }), e => e.code === 'PROVIDER_UNSUPPORTED')
  await assert.rejects(bridge.dispatch('auth.setApiKey', { provider: 'openai', key: 'never-accepted' }), e => e.code === 'METHOD_NOT_FOUND')
  finish()
})
