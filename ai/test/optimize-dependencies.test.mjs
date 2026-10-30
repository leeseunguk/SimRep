import test from 'node:test'
import assert from 'node:assert/strict'
import { mkdtemp, writeFile, rm, mkdir } from 'node:fs/promises'
import path from 'node:path'
import os from 'node:os'
import { optimizeTool } from '../../mcp/optimize.mjs'
import { particlePassband } from '../../mcp/workflows.mjs'
import { workflowDeadline } from '../../mcp/deadline.mjs'

for (const changed of ['ions.ion', 'field.pa1', 'field.cuts.json', 'map.pa']) {
  test(`optimizer rejects resumed history after changing dependency contents: ${changed}`, async t => {
    const dir = await mkdtemp(path.join(os.tmpdir(), 'simrep-resume-'))
    t.after(() => rm(dir, { recursive: true, force: true }))
    const files = { 'template.gem': '{{x}}', 'ions.ion': 'ions', 'field.pa0': 'base', 'field.pa1': 'basis', 'field.cuts.json': '{}', 'map.pa': 'magnetic', 'run.json': JSON.stringify({ physics: { magnetic_pas: [{ path: 'field.pa0' }], magnetic_support: { path: 'map.pa' } } }) }
    for (const [name, bytes] of Object.entries(files)) await writeFile(path.join(dir, name), bytes)
    const resolve = file => path.resolve(dir, file)
    const calls = []
    const tool = optimizeTool({ inputFile: resolve, insideWorkspace: resolve, prepareOutput: resolve, timeoutSeconds: 20,
      runTool: async (name, args) => {
        calls.push(name)
        assert.ok(workflowDeadline.getStore() > Date.now())
        if (name === 'refine_geometry') return { text: JSON.stringify({ converged: true, compute_ms: 1 }) }
        return { text: JSON.stringify({ arrivals: 1, arrival_tof_us: { mean: 2 }, compute_ms: 1 }) }
      },
    })
    const args = { template: 'template.gem', parameters: [{ name: 'x', min: 0, max: 2, initial: 1 }], electrodes: ['1=1:0'], out: 'search', budget: 1,
      objective: { kind: 'flight_scalar', key: 'arrival_tof_us.mean', fly: { particles: 'ions.ion', run_file: 'run.json' } } }
    const first = JSON.parse((await tool.run(args)).text)
    assert.equal(first.history.length, 1)
    const resumed = JSON.parse((await tool.run(args)).text)
    assert.deepEqual(resumed.history, first.history)
    assert.equal(calls.length, 2)
    await writeFile(path.join(dir, changed), changed.endsWith('.json') ? '{"version":2}' : 'changed in place')
    await assert.rejects(tool.run(args))
    assert.equal(calls.length, 2, 'changed input must be rejected before reusing history or running physics')
  })
}

test('bootstrap checks the shared deadline during a resample, not only between child processes', t => {
  let checks = 0
  t.mock.method(Date, 'now', () => ++checks <= 3 ? 0 : 100)
  const x = [1, 2, 3]
  const outcomes = [Array(20).fill(0), Array(20).fill(1), Array(20).fill(0)]
  assert.throws(() => particlePassband(x, outcomes, { resamples: 10000, seed: 1 }, 50))
  assert.equal(checks, 4)
})

test('an expired optimization deadline is inherited by analyzer bootstrap work', async t => {
  const dir = await mkdtemp(path.join(os.tmpdir(), 'simrep-deadline-'))
  t.after(() => rm(dir, { recursive: true, force: true }))
  await writeFile(path.join(dir, 'template.gem'), '{{x}}')
  let now = 0
  t.mock.method(Date, 'now', () => now)
  const resolve = file => path.resolve(dir, file)
  const tool = optimizeTool({ inputFile: resolve, insideWorkspace: resolve, prepareOutput: resolve, timeoutSeconds: 1,
    runTool: async name => {
      if (name === 'refine_geometry') return { text: JSON.stringify({ converged: true }) }
      assert.equal(workflowDeadline.getStore(), 1000)
      now = 1000
      particlePassband([1, 2, 3], [[0], [1], [0]], { resamples: 10000, seed: 1 }, workflowDeadline.getStore())
      assert.fail('expired bootstrap must not produce an objective')
    },
  })
  const result = JSON.parse((await tool.run({ template: 'template.gem', parameters: [{ name: 'x', min: 0, max: 2, initial: 1 }], electrodes: ['1=1:0'], out: 'search', budget: 2,
    objective: { kind: 'analyzer', targets: { k_v_per_ev: 2 }, weights: { k_v_per_ev: 1 } } })).text)
  assert.equal(result.timeout, true)
  assert.equal(result.history.length, 1)
  assert.equal(result.history[0].objective, '+inf')
  assert.equal(result.best_parameters, null)
})
