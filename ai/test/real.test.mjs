import test from 'node:test'
import assert from 'node:assert/strict'
import { mkdtemp, rm, writeFile } from 'node:fs/promises'
import { EventEmitter } from 'node:events'
import os from 'node:os'
import path from 'node:path'
import { Bridge } from '../bridge.mjs'
import { providerById } from '../providers/table.mjs'

test('real authenticated provider reads a CSV through SimRep MCP', {
  skip: !process.env.SIMREP_AI_REAL_SMOKE ? 'Set SIMREP_AI_REAL_SMOKE=<provider> after a real login; may consume subscription/API quota.' : false,
  timeout: 180_000,
}, async t => {
  const provider = process.env.SIMREP_AI_REAL_SMOKE
  assert.equal(providerById(provider)?.toolCalling, true)
  const events = new EventEmitter()
  const bridge = new Bridge({ notify: m => { if (m.method === 'event') events.emit('bridge:' + m.params.type, m.params) } })
  const status = await bridge.dispatch('auth.status', { provider })
  const account = status.providers[0].accounts.find(a => a.authenticated && (provider !== 'chatgpt' || a.planEnabled))
  if (!account) { t.skip('No eligible saved credentials for selected provider.'); bridge.close(); return }
  const workspace = await mkdtemp(path.join(os.tmpdir(), 'simrep-real-'))
  t.after(async () => { bridge.close(); await rm(workspace, { recursive: true, force: true }) })
  await writeFile(path.join(workspace, 'smoke.csv'), 'time_us\n2\n4\n')
  const session = await bridge.dispatch('chat.start', { provider, account: account.id, workspace })
  const result = new Promise((resolve, reject) => {
    const signal = AbortSignal.timeout(170_000)
    let tool
    const cleanup = () => { events.off('bridge:tool_result', onTool); events.off('bridge:error', onError); events.off('bridge:done', onDone); signal.removeEventListener('abort', onAbort) }
    const onTool = payload => { tool = payload }
    const onError = payload => { cleanup(); reject(new Error(`${payload.code}: ${payload.message}`)) }
    const onDone = payload => { cleanup(); resolve({ tool, done: payload }) }
    const onAbort = () => { cleanup(); reject(new Error('Timed out waiting for the turn.')) }
    events.on('bridge:tool_result', onTool); events.once('bridge:error', onError); events.once('bridge:done', onDone); signal.addEventListener('abort', onAbort, { once: true })
  })
  result.catch(() => {})
  await bridge.dispatch('chat.send', { sessionId: session.sessionId, text: 'Call the SimRep read_csv_summary tool on smoke.csv and report the mean time_us. Do not call any other tool.' })
  const outcome = await result
  assert.equal(outcome.tool?.name, 'read_csv_summary')
  assert.equal(outcome.done.status, 'completed')
})
