import test from 'node:test'
import assert from 'node:assert/strict'
import { mkdtemp, rm, writeFile, symlink } from 'node:fs/promises'
import path from 'node:path'
import os from 'node:os'
import { runEvals } from '../harness/evals/run.mjs'
import { createHarness } from '../harness/index.mjs'
import { OutputStore } from '../harness/context.mjs'
import { verifyResult, checkClaims } from '../harness/verification.mjs'
import { passband } from '../../mcp/workflows.mjs'

test('deterministic mock models exercise harness, bridge approval proxy and MCP workflows', { timeout: 120_000 }, async () => {
  const results = await runEvals()
  assert.ok(results.length >= 10)
  assert.ok(results.every(result => result.passed))
})

test('output handles recover exact bounded pages and expire with bounded storage', () => {
  const outputs = new OutputStore({ limit: 300, capacity: 1 })
  const source = JSON.stringify({ requested: { device: 'gpu' }, executed: { device: 'cpu' }, converged: false, data: 'x'.repeat(5000) })
  const summary = JSON.parse(outputs.compact(source))
  assert.equal(summary.truncated, true)
  assert.equal(summary.verification.ok, false)
  let restored = '', offset = 0
  do {
    const page = outputs.fetch({ handle: summary.handle, offset, length: 333 })
    restored += page.text; offset = page.next_offset
  } while (offset !== null)
  assert.equal(restored, source)
  assert.throws(() => outputs.fetch({ handle: summary.handle, length: 12001 }))
  outputs.compact('y'.repeat(5000))
  assert.throws(() => outputs.fetch({ handle: summary.handle }), /expired/)
})

test('verification inspects nested fallback, surface mode and disabled or unresolved gap checks', () => {
  const result = verifyResult({ runs: [{ converged: false, fallback_reason: 'adapter', requested: { device: 'gpu', surface_mode: 'source-geometry' }, executed: { device: 'cpu', surface_mode: 'staircase' }, gap_check: { mode: 'warn', pairs: [{ status: 'unresolved' }] } }] })
  assert.equal(result.ok, false)
  assert.equal(result.issues.length, 5)
  assert.equal(verifyResult({ gap_check: { mode: 'off' } }).ok, false)
  assert.equal(verifyResult({ converged: true, requested: { device: 'auto' }, executed: { device: 'cpu' } }).ok, true)
})

test('speed claims require the exact observed compute_ms value and cannot quote setup or speedup', () => {
  assert.equal(checkClaims('compute_ms: solve took 2.5 ms.', [2.5]).notes.length, 0)
  assert.equal(checkClaims('compute_ms: solve took 99 ms.', [2.5]).text, '')
  assert.equal(checkClaims('The runtime was 99 ms total_ms.', [2.5]).text, '')
  assert.equal(checkClaims('The solver was 3x faster.', [2.5]).text, '')
  assert.equal(checkClaims('compute_ms=99', [2.5]).text, '')
  assert.equal(checkClaims('total_ms=99', [2.5]).text, '')
  assert.equal(checkClaims('compute_ms=2.5', [2.5]).text, 'compute_ms=2.5')
  assert.equal(checkClaims('TOF is 1.2 us.', []).text, 'TOF is 1.2 us.')
})

test('workspace assembly skips external symlinks and loads only the selected recipe', async t => {
  const workspace = await mkdtemp(path.join(os.tmpdir(), 'simrep-context-'))
  t.after(() => rm(workspace, { recursive: true, force: true }))
  await writeFile(path.join(workspace, 'lens.gem'), 'geometry')
  await symlink('/etc/passwd', path.join(workspace, 'outside.json'))
  const events = []
  const harness = await createHarness({ workspace, language: 'ko', emit: fields => events.push(fields) })
  assert.ok(harness.systemPrompt.includes('lens.gem'))
  assert.ok(!harness.systemPrompt.includes('outside.json'))
  const prepared = await harness.prepare('einzel 렌즈 초점')
  assert.equal(harness.activeSkill, 'einzel')
  assert.ok(prepared.includes('Einzel focusing'))
  assert.ok(!prepared.includes('# Quadrupole transmission'))
  harness.emit({ type: 'delta', text: 'CPU result.' })
  assert.equal(events.length, 0)
  harness.finish('completed')
  assert.equal(events[0].activeSkill, 'einzel')
  const second = await harness.prepare('einzel')
  assert.ok(!second.includes('<loaded_skill>'))
})

test('passband rejects unbracketed, empty and nonmonotonic voltage scans', () => {
  assert.throws(() => passband([1,2,3], [1,1,1]), /bracketed/)
  assert.throws(() => passband([1,2,3], [0,0,0]), /no transmitted/)
  assert.throws(() => passband([1,1,3], [0,1,0]), /increasing/)
  const result = passband([10,15,20,25,30], [0,0.5,1,0.5,0])
  assert.equal(result.centroid_v, 20)
  assert.equal(result.fwhm_v, 10)
})
