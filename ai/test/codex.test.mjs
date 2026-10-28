import test from 'node:test'
import assert from 'node:assert/strict'
import { mkdtemp, rm } from 'node:fs/promises'
import path from 'node:path'
import os from 'node:os'
import { ChatGPTSession } from '../chatgpt.mjs'
import { RpcProcess } from '../rpc.mjs'
import { Store } from '../store.mjs'
import { toolGate } from '../tools.mjs'

// Real pinned binary, no inference and no user credential. This catches invalid
// CLI flags, catalog schemas, initialize attribution and thread/start contracts
// that an RPC mock cannot validate.
test('pinned Codex app-server initializes the restricted model catalog and SimRep MCP', { timeout: 70_000 }, async t => {
  const directory = await mkdtemp(path.join(os.tmpdir(), 'simrep-codex-init-'))
  const store = new Store(directory)
  await store.init()
  const gate = await toolGate(directory, async () => false, () => {})
  const auth = { store, access: async () => ({ access_token: 'test-not-a-real-access-token' }) }
  let diagnostics = ''
  const session = new ChatGPTSession({ auth, account: 'test', workspace: directory, model: 'gpt-6.1-sol', gate, emit: () => {},
    rpcFactory: (command, args, options) => {
      return new RpcProcess(command, args, { ...options, onStderr: text => { diagnostics += text } })
    } })
  t.after(async () => { session.close(); await rm(directory, { recursive: true, force: true }) })
  try { await session.connect() } catch { assert.fail(diagnostics) }
  assert.ok(session.threadId)
  const servers = await session.rpc.request('mcpServerStatus/list', { threadId: session.threadId })
  assert.equal(servers.data.length, 1)
  assert.equal(servers.data[0].name, 'simrep')
  assert.ok(Object.keys(servers.data[0].tools).some(name => name.includes('read_csv_summary')))
})
