import test from 'node:test'
import assert from 'node:assert/strict'
import { mkdtemp, mkdir, rm } from 'node:fs/promises'
import path from 'node:path'
import os from 'node:os'
import { Bridge } from '../bridge.mjs'
import { PROVIDERS } from '../providers/table.mjs'
import { RpcProcess } from '../rpc.mjs'
import { Store } from '../store.mjs'

const HARNESS_TOOLS = ['load_skill', 'fetch_tool_output', 'simrep_plan', 'analyzer_scan',
  'grid_convergence', 'compare_runs', 'check_geometry', 'summarize_results', 'optimize_design']

for (const provider of PROVIDERS.filter(p => p.toolCalling)) {
  test(`${provider.id}: Bridge supplies the harness system prompt and every harness MCP tool`, async t => {
    const directory = await mkdtemp(path.join(os.tmpdir(), 'simrep-provider-harness-'))
    const workspace = path.join(directory, 'workspace')
    await mkdir(workspace)
    const store = new Store(path.join(directory, 'config'))
    const auth = Object.fromEntries(PROVIDERS.map(p => [p.id, { store, close() {},
      accounts: async () => [{ id: p.id + '-test', provider: p.id, label: p.label, authenticated: true, planEnabled: true }],
      models: async () => p.models,
      access: async () => ({ access_token: 'not-a-real-token' }),
    }]))
    const bridge = new Bridge({ auth })
    t.after(async () => { bridge.close(); await rm(directory, { recursive: true, force: true }) })
    const started = await bridge.dispatch('chat.start', { provider: provider.id, workspace, language: 'ko' })
    const session = bridge.sessions.get(started.sessionId)
    assert.equal(typeof session.harness.systemPrompt, 'string')
    assert.ok(session.harness.systemPrompt.length > 0)
    assert.equal(session.client.systemPrompt, session.harness.systemPrompt)
    const config = session.client.gate.config
    const rpc = new RpcProcess(config.command, config.args, { env: { ...process.env, ...config.env } })
    try {
      await rpc.request('initialize', { protocolVersion: '2025-06-18', capabilities: {}, clientInfo: { name: 'provider-harness-test', version: '1' } })
      rpc.send({ jsonrpc: '2.0', method: 'notifications/initialized' })
      const { tools } = await rpc.request('tools/list')
      const names = tools.map(tool => tool.name)
      for (const name of HARNESS_TOOLS) assert.ok(names.includes(name), `${provider.id} is missing ${name}`)
      assert.equal(names.includes('Bash'), false)
      assert.equal(names.includes('write_file'), false)
    } finally { rpc.close() }
  })
}
