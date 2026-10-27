#!/usr/bin/env node
// Deterministic CLI boundary fixture, not a physics oracle.
import { readFileSync, writeFileSync } from 'node:fs'

const args = process.argv.slice(2).filter(arg => arg !== '--json')
const command = args[0]
const flag = name => args[args.indexOf(name) + 1]
const has = name => args.includes(name)
const requested = { device: has('--device') ? flag('--device') : 'cpu', precision: 'f64', backend: null }
const base = { schema_version: 1, simrep_version: 'eval-mock', requested, executed: { ...requested }, fallback_reason: null, compute_ms: 2.5, prep_ms: 99 }
let result
switch (command) {
  case 'gem2pa':
    writeFileSync(args[2], '{}')
    result = { ...base, output_paths: [args[2]], electrode_numbers: [1,2,3], dims: [61,31,1], spacing_mm: [1,1,1] }
    break
  case 'refine':
  case 'refine-geometry': {
    const fallback = args[1].includes('fallback'), unconverged = args[1].includes('unconverged')
    if (command === 'refine-geometry') {
      const stem = flag('--out')
      const h = has('--spacing') ? Number(flag('--spacing').split(',')[0]) : 1
      for (const extension of ['.pa0', '.pa1', '.pa2', '.pa3', '.pa#', '.cuts.json']) writeFileSync(stem + extension, JSON.stringify({ h }))
    }
    result = { ...base, executed: { ...requested, device: fallback ? 'cpu' : requested.device }, fallback_reason: fallback ? 'mock adapter unavailable' : null,
      converged: !unconverged, cycles: 12, final_residual: unconverged ? 0.1 : 1e-11, tolerance: 1e-10,
      gap_check: { mode: has('--gap-check') ? flag('--gap-check') : 'error', pairs: [{ status: 'ok', cells_across: 10 }] } }
    break
  }
  case 'cad-measure':
    result = { ...base, measurements: [{ min_distance_mm: 0.58, touching: false, cross_check_error_mm: 0 }] }
    break
  case 'fly': {
    const rows = readFileSync(flag('--particles'), 'utf8').trim().split('\n')
    const energy = Number(rows[0].split(',')[8])
    let h = 1
    if (has('--pa')) { try { h = JSON.parse(readFileSync(flag('--pa'), 'utf8')).h ?? 1 } catch { /* real PA fixture */ } }
    const scanned = has('--voltage')
    const voltage = scanned ? Number(flag('--voltage').split(',').find(pair => pair.startsWith('2='))?.split('=')[1]) : 0
    const count = rows.length
    const hits = scanned ? Math.round(count * Math.max(0, 1 - Math.abs(voltage - 0.2 * energy) / 10)) : count
    if (has('--output')) writeFileSync(flag('--output'), 'ion,tof_us,x_mm,y_mm,z_mm,reason\n' + rows.map((_, i) => `${i + 1},${1 + h * h},58,${i < hits ? 0 : 5},0,plane`).join('\n') + '\n')
    result = { ...base, ion_count: count, arrivals: count, arrival_tof_us: { mean: 1 + h * h, std: 0, min: 1 + h * h, max: 1 + h * h },
      splat_counts: { plane: count }, executed: { ...requested, surface_mode: 'source-geometry' }, output_paths: has('--output') ? [flag('--output')] : [] }
    break
  }
  default:
    result = { ...base, error: { message: `Unsupported mock command ${command}`, kind: 'mock' } }
    process.exitCode = 1
}
process.stdout.write(JSON.stringify(result) + '\n')
