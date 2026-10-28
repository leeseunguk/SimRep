import assert from 'node:assert/strict'
import { EventEmitter, once } from 'node:events'
import { copyFile, mkdir, mkdtemp, readFile, rm, writeFile, chmod } from 'node:fs/promises'
import os from 'node:os'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { Bridge } from '../../bridge.mjs'
import { RpcProcess } from '../../rpc.mjs'
import { ClaudeAuth, ClaudeSession } from '../../claude.mjs'

const here = path.dirname(fileURLToPath(import.meta.url))
const repo = path.resolve(here, '../../..')

function checks(value, expected, id) {
  for (const [key, wanted] of Object.entries(expected ?? {})) {
    const got = key.split('.').reduce((node, part) => node?.[part], value)
    if (wanted && typeof wanted === 'object' && 'near' in wanted) assert.ok(Math.abs(got - wanted.near) <= wanted.tolerance, `${id}: ${key}=${got}, expected ${wanted.near} +/- ${wanted.tolerance}`)
    else if (wanted && typeof wanted === 'object' && 'gte' in wanted) assert.ok(got >= wanted.gte, `${id}: ${key}=${got}`)
    else assert.deepEqual(got, wanted, `${id}: ${key}`)
  }
}

async function fixtures(workspace, mock) {
  await copyFile(path.join(repo, 'examples/data/einzel.gem'), path.join(workspace, 'einzel.gem'))
  await copyFile(path.join(repo, 'examples/data/ions.ion'), path.join(workspace, 'ions.ion'))
  await copyFile(path.join(repo, 'tools/cad-fixtures/pair.step'), path.join(workspace, 'pair.step'))
  await writeFile(path.join(workspace, 'results.csv'), 'reason,time_us\nplane,2\nplane,4\n')
  for (const { name, timing } of [{ name: 'before.json', timing: 2 }, { name: 'after.json', timing: 99 }]) await writeFile(path.join(workspace, name), JSON.stringify({ schema_version: 1, converged: true, cycles: 12, compute_ms: timing }))
  if (mock) for (const name of ['einzel.pa#', 'einzel.pa0', 'fallback.pa#', 'unconverged.pa#']) await writeFile(path.join(workspace, name), '{"h":1}')
}

export async function runEvals({ real = false, report = (_line) => {} } = {}) {
  const tasks = JSON.parse(await readFile(path.join(here, 'tasks.json'), 'utf8')).filter(task => !real || task.real)
  const root = await mkdtemp(path.join(os.tmpdir(), 'simrep-harness-evals-'))
  const original = { SIMREP_BIN: process.env.SIMREP_BIN, SIMREP_PYTHON: process.env.SIMREP_PYTHON }
  const results = []
  try {
    if (!real) {
      for (const [source, variable] of [['mock-simrep.mjs', 'SIMREP_BIN'], ['mock-python.mjs', 'SIMREP_PYTHON']]) {
        const destination = path.join(root, source)
        await copyFile(path.join(here, source), destination); await chmod(destination, 0o700)
        process.env[variable] = destination
      }
    }
    for (const task of tasks) {
      const workspace = path.join(root, task.id)
      await mkdir(workspace)
      await fixtures(workspace, !real)
      const events = new EventEmitter(), seen = [], calls = []
      let bridge, activeRpc
      const account = { id: 'eval', provider: 'claude', label: 'Mock', authenticated: true, planEnabled: true }
      class MockAuth extends ClaudeAuth { async accounts() { return [account] } }
      class MockSession extends ClaudeSession {
        /** @returns {Promise<'completed'>} */
        async send(text) {
          assert.ok(this.systemPrompt && text.includes(task.prompt), `${task.id}: harness must prepare provider input`)
          const rpc = new RpcProcess(this.gate.config.command, this.gate.config.args, { env: { ...process.env, ...this.gate.config.env } })
          activeRpc = rpc
          try {
            await rpc.request('initialize', { protocolVersion: '2025-06-18', capabilities: {}, clientInfo: { name: 'harness-eval', version: '1' } })
            rpc.send({ jsonrpc: '2.0', method: 'notifications/initialized' })
            for (const action of task.script) await rpc.request('tools/call', { name: action.tool, arguments: action.args })
            this.emit({ type: 'delta', text: task.claim })
            return 'completed'
          } finally { rpc.close() }
        }
        cancel() { activeRpc?.close() }
        close() { activeRpc?.close(); this.gate.close() }
      }
      const notify = message => {
        if (message.method !== 'event') return
        const event = message.params
        seen.push(event)
        if (event.type === 'tool_result') calls.push(event)
        if (event.type === 'approval') void bridge.dispatch('approval.resolve', { approvalId: event.approvalId, allow: task.approve !== false })
        events.emit(event.type, event)
      }
      bridge = real ? new Bridge({ notify }) : new Bridge({ claude: new MockAuth(), createClaude: options => new MockSession(options), notify })
      try {
        const provider = real ? process.env.SIMREP_AI_REAL_SMOKE : 'claude'
        const session = await bridge.dispatch('chat.start', { provider, workspace, language: 'en' })
        const completed = once(events, 'done', { signal: AbortSignal.timeout(real ? 170_000 : 30_000) })
        // A real provider must compile the real GEM first, rather than reading fake PA data.
        const prompt = real && task.id === 'einzel-refine'
          ? 'First call gem2pa on einzel.gem to output einzel.pa#, then ' + task.prompt : task.prompt
        await bridge.dispatch('chat.send', { sessionId: session.sessionId, text: prompt })
        assert.equal((await completed)[0].status, 'completed', `${task.id}: turn completion`)
        for (const action of task.script) {
          const called = calls.find(call => call.name === action.tool)
          assert.ok(called, `${task.id}: expected ${action.tool} call`)
          assert.equal(Boolean(called.result.isError), Boolean(action.isError), `${task.id}: ${action.tool} isError`)
          if (!action.isError) checks(JSON.parse(called.result.content[0].text), action.checks, task.id)
        }
        if (task.recipe) assert.ok(seen.some(event => event.activeSkill === task.recipe), `${task.id}: active recipe event`)
        const answer = seen.filter(event => event.type === 'delta').map(event => event.text).join('')
        if (task.verify) assert.match(answer, /\[verify\]/, `${task.id}: verification note`)
        if (task.withheld) assert.ok(answer.includes(task.withheld) && answer.includes('[verify]'), `${task.id}: speed claim must remain and be annotated`)
        if (task.approve === false) {
          const files = await readFile(path.join(workspace, 'einzel.pa#'), 'utf8').catch(() => '')
          assert.equal(files, '{"h":1}')
          await assert.rejects(readFile(path.join(workspace, 'denied.pa#')))
        }
        results.push({ id: task.id, passed: true, tools: calls.map(call => call.name) })
        report(`PASS ${task.id}`)
      } finally { bridge.close() }
    }
    return results
  } finally {
    for (const [key, value] of Object.entries(original)) { if (value === undefined) delete process.env[key]; else process.env[key] = value }
    await rm(root, { recursive: true, force: true })
  }
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const real = Boolean(process.env.SIMREP_AI_REAL_SMOKE)
  const results = await runEvals({ real, report: console.log })
  console.log(`${results.length}/${results.length} harness evals passed (${real ? 'real provider' : 'deterministic mock'}).`)
}
