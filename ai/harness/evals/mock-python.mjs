#!/usr/bin/env node
// Isolate only the native uncertainty boundary; result_identity runs for real.
import { spawnSync } from 'node:child_process'
if (process.argv[2].endsWith('grid_uncertainty.py')) {
  const { h, values } = JSON.parse(process.argv[3])
  if (JSON.stringify(h) !== '[0.25,0.5,1]' || JSON.stringify(values) !== '[1.0625,1.25,2]') process.exit(2)
  console.log(JSON.stringify({ p: 2, extrapolated: 1, gci_fine: 0.078125 / 1.0625, u_num: 0.078125, behavior: 'Monotone', estimator: 'mock Celik boundary' }))
} else {
  const run = spawnSync('python3', process.argv.slice(2), { encoding: 'utf8', shell: false })
  process.stdout.write(run.stdout ?? '')
  process.stderr.write(run.stderr ?? '')
  process.exitCode = run.status ?? 2
}
