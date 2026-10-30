import test from 'node:test'
import assert from 'node:assert/strict'
import { spawn } from 'node:child_process'
import { createServer } from 'node:http'
import { mkdtemp, writeFile, readFile, rm, symlink, access, mkdir } from 'node:fs/promises'
import { once } from 'node:events'
import readline from 'node:readline'
import path from 'node:path'
import os from 'node:os'
import { fileURLToPath } from 'node:url'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..')
async function fixture(t) {
  const dir = await mkdtemp(path.join(os.tmpdir(), 'simrep-boundary-'))
  const workspace = path.join(dir, 'workspace')
  await mkdir(workspace)
  await writeFile(path.join(workspace, 'ions.ion'), '0,1,1,0,0,0,0,0,1\n')
  await writeFile(path.join(workspace, 'input.gem'), 'pa_define(2,2,1,planar)\n')
  const bin = path.join(dir, 'fake-simrep')
  await writeFile(bin, `#!${process.execPath}\nimport fs from 'node:fs';\nconst args = process.argv.slice(2); const out = args[args.indexOf('--output') + 1];\nfs.writeFileSync(${JSON.stringify(path.join(dir, 'executed'))}, 'executed');\nif (args.includes('--output')) { fs.writeFileSync(out, 'csv'); fs.writeFileSync(out + '.json', 'OVERWRITTEN'); }\nconsole.log(JSON.stringify({schema_version:1,output_paths:[]}));\n`, { mode: 0o700 })
  t.after(() => rm(dir, { recursive: true, force: true }))
  return { dir, workspace, bin }
}
function start(t, file, env) {
  const child = spawn(process.execPath, [path.join(root, file)], { env: { ...process.env, ...env }, stdio: ['pipe', 'pipe', 'pipe'] })
  const stdout = readline.createInterface({ input: child.stdout })
  const stderr = readline.createInterface({ input: child.stderr })
  const messages = [], audits = []
  stdout.on('line', line => messages.push(JSON.parse(line)))
  stderr.on('line', line => { try { audits.push(JSON.parse(line)) } catch { /* Startup diagnostic. */ } })
  t.after(async () => { const closed = once(child, 'close'); child.kill('SIGTERM'); await closed; stdout.close(); stderr.close() })
  const waitFor = (stream, predicate) => new Promise((resolve, reject) => {
    const timer = setTimeout(() => { stream.off('line', line); reject(new Error('RPC event timeout')) }, 5000)
    const line = text => {
      let msg
      try { msg = JSON.parse(text) } catch { return }
      if (predicate(msg)) { clearTimeout(timer); stream.off('line', line); resolve(msg) }
    }
    stream.on('line', line)
  })
  let id = 0
  return {
    child, messages, audits,
    send: msg => child.stdin.write(JSON.stringify(msg) + '\n'),
    rejected: () => waitFor(stderr, msg => msg.event === 'rejected' && msg.method === 'tools/call'),
    async request(method, params) {
      const requestId = ++id
      const response = waitFor(stdout, msg => msg.id === requestId)
      child.stdin.write(JSON.stringify({ jsonrpc: '2.0', id: requestId, method, params }) + '\n')
      return response
    },
  }
}

for (const surface of ['mcp/server.mjs', 'ai/mcp-proxy.mjs']) {
  for (const mode of ['ask', 'session', 'bypass']) {
    for (const id of [undefined, null]) {
      test(`${surface} rejects and audits ${mode} tools/call with id=${id}`, async t => {
        const f = await fixture(t)
        const gateCalls = []
        const gate = createServer((req, res) => { gateCalls.push(req.url); res.setHeader('content-type', 'application/json'); res.end('{"allow":true}') })
        await new Promise(resolve => gate.listen(0, '127.0.0.1', resolve))
        t.after(() => { gate.closeAllConnections(); gate.close() })
        const rpc = start(t, surface, { SIMREP_WORKSPACE: f.workspace, SIMREP_BIN: f.bin, SIMREP_AI_APPROVAL: mode,
          SIMREP_AI_GATE: `http://127.0.0.1:${gate.address().port}/tool`, SIMREP_AI_GATE_KEY: 'test' })
        await rpc.request('initialize', {})
        for (const name of ['gem2pa', 'Bash']) {
          const rejected = rpc.rejected()
          rpc.send({ jsonrpc: '2.0', ...(id === null ? { id: null } : {}), method: 'tools/call', params: { name, arguments: { input: 'input.gem' } } })
          assert.equal((await rejected).reason, 'request_id_required')
        }
        await rpc.request('ping', {})
        assert.equal(rpc.messages.some(m => m.id === undefined || m.id === null), false, 'notification must have no response')
        assert.deepEqual(gateCalls, [])
        await assert.rejects(access(path.join(f.dir, 'executed')))
      })
    }
  }
}

for (const dangling of [false, true]) {
  test(`MCP refuses ${dangling ? 'dangling' : 'existing'} outside flight-sidecar symlink before execution`, async t => {
    const f = await fixture(t)
    const sentinel = path.join(f.dir, 'outside.json')
    if (!dangling) await writeFile(sentinel, 'KEEP')
    await symlink(sentinel, path.join(f.workspace, 'flight.csv.json'))
    const rpc = start(t, 'mcp/server.mjs', { SIMREP_WORKSPACE: f.workspace, SIMREP_BIN: f.bin })
    const response = await rpc.request('tools/call', { name: 'fly', arguments: { particles: 'ions.ion', output: 'flight.csv', device: 'cpu' } })
    assert.equal(response.result.isError, true)
    await assert.rejects(access(path.join(f.dir, 'executed')))
    if (dangling) await assert.rejects(access(sentinel))
    else assert.equal(await readFile(sentinel, 'utf8'), 'KEEP')
  })
}

for (const [name, args, derived] of [
  ['gem2pa', { input: 'input.gem' }, 'input.pa#'],
  ['refine', { file: 'input.pa#' }, 'input.pa1000'],
  ['refine_geometry', { input: 'input.gem', electrode: ['1=1:0'], out: 'lens' }, 'lens.cuts.json'],
]) {
  test(`MCP validates derived ${name} output before execution`, async t => {
    const f = await fixture(t)
    if (name === 'refine') await writeFile(path.join(f.workspace, 'input.pa#'), 'potential')
    const sentinel = path.join(f.dir, 'outside')
    await writeFile(sentinel, 'KEEP')
    await symlink(sentinel, path.join(f.workspace, derived))
    const rpc = start(t, 'mcp/server.mjs', { SIMREP_WORKSPACE: f.workspace, SIMREP_BIN: f.bin })
    const response = await rpc.request('tools/call', { name, arguments: args })
    assert.equal(response.result.isError, true)
    await assert.rejects(access(path.join(f.dir, 'executed')))
    assert.equal(await readFile(sentinel, 'utf8'), 'KEEP')
  })
}

test('MCP refuses unsupported trajectory output arguments without executing a flight', async t => {
  const f = await fixture(t)
  const rpc = start(t, 'mcp/server.mjs', { SIMREP_WORKSPACE: f.workspace, SIMREP_BIN: f.bin })
  const response = await rpc.request('tools/call', { name: 'fly', arguments: { particles: 'ions.ion', output: 'flight.csv', trajectories: '../outside.csv' } })
  assert.equal(response.result.isError, true)
  await assert.rejects(access(path.join(f.dir, 'executed')))
})
