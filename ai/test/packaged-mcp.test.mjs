import test from 'node:test'
import assert from 'node:assert/strict'
import { cp, glob, mkdir, mkdtemp, readFile, rm, stat } from 'node:fs/promises'
import path from 'node:path'
import os from 'node:os'
import { spawn } from 'node:child_process'
import readline from 'node:readline'
import { fileURLToPath } from 'node:url'

const configPath = fileURLToPath(new URL('../../app/src-tauri/tauri.conf.json', import.meta.url))

test('packaged resource tree initializes MCP and lists all tools', { timeout: 180000 }, async t => {
  const root = await mkdtemp(path.join(os.tmpdir(), 'simrep-packaged-'))
  t.after(() => rm(root, { recursive: true, force: true }))
  const config = JSON.parse(await readFile(configPath, 'utf8'))
  const configDir = path.dirname(configPath)
  for (const [source, destination] of Object.entries(config.bundle.resources)) {
    t.signal.throwIfAborted()
    const sourcePath = path.resolve(configDir, source)
    if (source.includes('*')) {
      for await (const file of glob(sourcePath)) {
        t.signal.throwIfAborted()
        const target = path.join(root, destination, path.basename(file))
        await mkdir(path.dirname(target), { recursive: true })
        await cp(file, target)
      }
    } else {
      // Tauri copies a directory's contents to the mapped destination.
      const target = path.join(root, destination)
      assert.ok((await stat(sourcePath)).isFile() || source.endsWith('/'))
      await mkdir(path.dirname(target), { recursive: true })
      await cp(sourcePath, target, { recursive: true })
    }
  }
  t.signal.throwIfAborted()
  const proc = spawn(process.execPath, ['mcp/server.mjs'], {
    cwd: root,
    env: { ...process.env, SIMREP_WORKSPACE: root, SIMREP_BIN: process.execPath },
    stdio: ['pipe', 'pipe', 'pipe'],
  })
  t.after(() => { proc.stdin.end(); proc.kill('SIGKILL') })
  let stderr = ''
  proc.stderr.on('data', chunk => { stderr += chunk })
  let id = 0
  const pending = new Map()
  const lines = readline.createInterface({ input: proc.stdout })
  t.after(() => lines.close())
  lines.on('line', line => {
    const message = JSON.parse(line)
    pending.get(message.id)?.resolve(message)
  })
  proc.on('exit', code => {
    for (const p of pending.values()) p.reject(new Error(`packaged MCP exited ${code}: ${stderr}`))
  })
  const request = (method, params = {}) => new Promise((resolve, reject) => {
    const requestId = ++id
    const timer = setTimeout(() => reject(new Error(`packaged MCP ${method} timeout: ${stderr}`)), 10000)
    pending.set(requestId, {
      resolve: message => { clearTimeout(timer); pending.delete(requestId); resolve(message) },
      reject: error => { clearTimeout(timer); pending.delete(requestId); reject(error) },
    })
    proc.stdin.write(`${JSON.stringify({ jsonrpc: '2.0', id: requestId, method, params })}\n`)
  })
  const initialized = await request('initialize', { protocolVersion: '2025-06-18', capabilities: {}, clientInfo: { name: 'resource-smoke', version: '1' } })
  assert.equal(initialized.error, undefined)
  assert.equal(initialized.result.serverInfo.name, 'simrep')
  const listed = await request('tools/list')
  assert.equal(listed.error, undefined)
  const names = listed.result.tools.map(tool => tool.name)
  assert.ok(names.includes('optimize_design'))
  assert.ok(names.includes('analyzer_scan'))
  assert.ok(names.includes('fly'))
  console.log(`PACKAGED_MCP initialize=pass tools/list=pass tools=${names.length}`)
})
