// Original SimRep workflow tools. All paths and executions use the server's
// existing workspace guard and argv-only child runner.
import { readFile, readdir, lstat } from 'node:fs/promises'
import path from 'node:path'
import { inspectWorkspace } from '../ai/harness/workspace.mjs'
import { loadSkill, selectRecipe, skillCatalog } from '../ai/harness/skills.mjs'

const object = properties => ({ type: 'object', properties, additionalProperties: false })
const string = { type: 'string' }
const number = { type: 'number' }
const array = items => ({ type: 'array', items })
const json = value => ({ text: JSON.stringify(value), isError: false })
function requireCondition(ok, message) { if (!ok) throw new Error(message) }
function finite(value, what) { requireCondition(typeof value === 'number' && Number.isFinite(value), `${what} must be finite`); return value }
function positive(value, what) { requireCondition(finite(value, what) > 0, `${what} must be positive`); return value }
function only(args, keys) { for (const key of Object.keys(args)) requireCondition(keys.includes(key), `unknown argument ${key}`) }
function parse(result) {
  let value
  try { value = JSON.parse(result.text) } catch { throw new Error(result.text) }
  if (result.isError || value.error) throw new Error(value.error?.message ?? result.text)
  return value
}
function vector(value, size, name) {
  requireCondition(Array.isArray(value) && value.length === size, `${name} needs ${size} numbers`)
  return value.map((item, i) => finite(item, `${name}[${i}]`))
}

/** Connected half-height peak; no FWHM for an unbracketed or empty passband. */
export function passband(x, y) {
  requireCondition(x.length === y.length && x.length >= 3 && x.every((item, i) => Number.isFinite(item) && (!i || item > x[i - 1])), 'scan voltages must be strictly increasing')
  requireCondition(y.every(item => Number.isFinite(item) && item >= 0), 'transmission must be finite and nonnegative')
  const peak = Math.max(...y)
  requireCondition(peak > 0, 'no transmitted particles; inspect beam, voltages, detector and geometry')
  const i = y.indexOf(peak), half = peak / 2
  let left = i, right = i
  while (left > 0 && y[left - 1] >= half) left--
  while (right < y.length - 1 && y[right + 1] >= half) right++
  requireCondition(left > 0 && right < y.length - 1, 'peak is not bracketed; widen the scan')
  const cross = (a, b) => x[a] + (half - y[a]) * (x[b] - x[a]) / (y[b] - y[a])
  const lower = cross(left - 1, left), upper = cross(right, right + 1)
  const vertices = [[lower, half], ...x.slice(left, right + 1).map((value, k) => [value, y[left + k]]), [upper, half]]
  let area = 0, moment = 0
  for (let k = 1; k < vertices.length; k++) {
    const [x0, y0] = vertices[k - 1], [x1, y1] = vertices[k], dx = x1 - x0
    area += dx * (y0 + y1) / 2
    moment += dx * (x0 * (2 * y0 + y1) + x1 * (y0 + 2 * y1)) / 6
  }
  const centroid = moment / area
  requireCondition(Number.isFinite(centroid) && centroid !== 0, 'passband centroid must be nonzero for relative energy resolution')
  const fwhm = upper - lower
  return { centroid_v: centroid, fwhm_v: fwhm, dE_over_E: fwhm / Math.abs(centroid), peak_transmission: peak,
    half_height_crossings_v: [lower, upper], points_above_half: right - left + 1,
    warnings: right - left + 1 < 8 ? ['Fewer than 8 scan points above half height; refine the passband scan before reporting resolution.'] : [] }
}

async function csvRows(file) {
  const info = await lstat(file)
  requireCondition(info.size <= 64_000_000, 'CSV exceeds 64 MB; summarize or split the scan first')
  const lines = (await readFile(file, 'utf8')).trim().split(/\r?\n/)
  const columns = lines.shift().split(',').map(cell => cell.trim())
  requireCondition(lines.length <= 2_000_000, 'CSV exceeds 2000000 rows')
  return lines.filter(line => line.trim()).map(line => Object.fromEntries(line.split(',').map((value, i) => [columns[i], value.trim()])))
}

export function workflowTools({ workspace, inputFile, insideWorkspace, prepareOutput, runTool, runHelper, summarizeCsv, outputs, here }) {
  const tool = (name, description, properties, required, readOnly, run) => ({
    name, title: name.replaceAll('_', ' '), description,
    inputSchema: { ...object(properties), required }, readOnly, run,
  })
  async function validateTree(relative, visited = new Set()) {
    const absolute = insideWorkspace(relative)
    requireCondition(!visited.has(absolute), 'duplicate or cyclic symlink in comparison tree')
    visited.add(absolute)
    const info = await lstat(absolute)
    if (info.isDirectory()) for (const item of await readdir(absolute)) await validateTree(path.join(absolute, item), visited)
    return absolute
  }
  async function csvSummary(relative) { return summarizeCsv(inputFile(relative, 'path'), 2_000_000, 0) }
  return [
    tool('simrep_plan', 'Inspect available workspace geometry, PA families, particles and last JSON results; propose steps and gates without executing a solve. task is the user objective, not a shell command.', { task: string }, ['task'], true, async args => {
      only(args, ['task'])
      requireCondition(typeof args.task === 'string' && args.task.length > 0, 'task is required')
      const inventory = await inspectWorkspace(workspace)
      const recipe = selectRecipe(args.task)
      const geometry = inventory.files.find(file => /\.(gem|step|stp|iges|igs)$/i.test(file.path))?.path
      const paHash = inventory.files.find(file => /\.pa#$/i.test(file.path))?.path
      const pa0 = inventory.files.find(file => /\.pa0$/i.test(file.path))?.path
      return json({ schema_version: 1, workspace: inventory, skill: recipe?.id ?? 'ion-optics',
        inputs: { geometry, unrefined_pa: paHash, refined_pa: pa0 },
        steps: [
          ...(geometry ? [
            { tool: 'check_geometry', gate: 'Every part assigned; units/origin recorded; spacing <= minimum electrode gap / 10.' },
            { tool: 'refine_geometry', gate: 'converged=true; CPU or validated hardware GPU; gap_check error; exact cuts kept with PA0 bases.' },
          ] : paHash ? [{ tool: 'refine', gate: 'Solve the available unrefined PA on CPU; converged=true and residual <= tolerance.' }]
            : pa0 ? [{ tool: 'pa_info', gate: 'Inspect the available PA0 and check same-stem bases/cuts before fast adjust or flight.' }]
              : [{ action: 'Provide geometry or a PA input before solving or flying.', gate: 'No simulator input was found in the workspace.' }]),
          { tool: recipe?.id === 'esa' ? 'analyzer_scan' : 'fly', gate: recipe?.steps ?? 'Define species, beam, detector aperture, bounds and allowed terminations.' },
          { tool: 'grid_convergence', gate: 'Same observable on three physical grids; value +/- u_num, not residual alone.' },
          { tool: 'summarize_results', gate: 'Report requested/executed, fallback reasons, beam definition, uncertainty and compute_ms only for speed.' },
        ] })
    }),
    tool('load_skill', 'Load one trusted bundled SimRep skill or recipe on demand. No arbitrary file paths. IDs: ion-optics, einzel, esa, quadrupole, magnetic, cad.', {
      id: { type: 'string', enum: skillCatalog().map(item => item.id) },
    }, ['id'], true, async args => { only(args, ['id']); const skill = await loadSkill(args.id); return json({ skill: skill.id, text: skill.text }) }),
    tool('fetch_tool_output', 'Fetch a bounded character page from an opaque output handle returned by a truncated tool result in this MCP session. Handles expire after 32 large results or session close.', {
      handle: string, offset: { type: 'integer', minimum: 0 }, length: { type: 'integer', minimum: 1, maximum: 12000 },
    }, ['handle'], true, async args => { only(args, ['handle', 'offset', 'length']); return json(outputs.fetch(args)) }),
    tool('summarize_results', 'Read workspace CSV/JSON results into a compact table. No simulator execution; timings are labeled compute_ms. CSV numeric statistics and termination counts are included; large results have fetch handles.', {
      paths: { ...array(string), minItems: 1, maxItems: 20 },
    }, ['paths'], true, async args => {
      only(args, ['paths'])
      requireCondition(Array.isArray(args.paths) && args.paths.length > 0 && args.paths.length <= 20, 'paths needs 1..20 files')
      const rows = []
      for (const file of args.paths) {
        const absolute = inputFile(file, 'path')
        if (/\.csv$/i.test(file)) {
          const summary = await csvSummary(file)
          rows.push({ path: file, rows: summary.row_count, numeric: summary.numeric, reason_counts: summary.reason_counts, truncated: summary.truncated })
        } else {
          requireCondition(/\.json$/i.test(file), 'summarize_results accepts CSV or JSON')
          requireCondition((await lstat(absolute)).size <= 8_000_000, 'JSON exceeds 8 MB')
          const value = JSON.parse(await readFile(absolute, 'utf8'))
          rows.push({ path: file, ...Object.fromEntries(['converged', 'cycles', 'final_residual', 'requested', 'executed', 'fallback_reason', 'gap_check', 'compute_ms', 'ion_count', 'arrivals', 'arrival_tof_us', 'splat_counts', 'error']
            .filter(key => value[key] !== undefined).map(key => [key, value[key]])) })
        }
      }
      return json({ schema_version: 1, rows })
    }),
    tool('compare_runs', 'Compare two workspace artifact files/directories with tools/result_identity.py. Timing keys are ignored by that helper; counts stay exact, continuous data uses explicit tolerances. Symlink escapes are refused recursively.', {
      before: string, after: string, tolerance: number, relative_tolerance: number, require_byte_identity: { type: 'boolean' },
    }, ['before', 'after'], true, async args => {
      only(args, ['before', 'after', 'tolerance', 'relative_tolerance', 'require_byte_identity'])
      const argv = [await validateTree(args.before), await validateTree(args.after), '--json']
      for (const key of ['tolerance', 'relative_tolerance']) if (args[key] !== undefined) {
        requireCondition(finite(args[key], key) >= 0, `${key} must be nonnegative`)
        argv.push('--' + key.replaceAll('_', '-'), String(args[key]))
      }
      if (args.require_byte_identity !== undefined) requireCondition(typeof args.require_byte_identity === 'boolean', 'require_byte_identity must be boolean')
      if (args.require_byte_identity) argv.push('--require-byte-identity')
      const result = await runHelper(path.resolve(here, '../tools/result_identity.py'), argv)
      requireCondition(!result.killed && !result.truncated && result.code !== null && result.code < 2, result.err || result.out)
      return json(JSON.parse(result.out))
    }),
    tool('check_geometry', 'Check geometry before flight. STEP/IGES first uses exact cad_measure; supplied refine object then runs refine_geometry with gap_check error (cannot disable). This can write a PA family and requires approval. GEM/mesh need refine settings; CAD-only measurement is allowed without refine.', {
      input: string, gap: array(string), spacing_mm: array(number), refine: { type: 'object', description: 'refine_geometry arguments except input/gap_check; electrode and out required.' },
    }, ['input'], false, async args => {
      only(args, ['input', 'gap', 'spacing_mm', 'refine'])
      inputFile(args.input, 'input')
      const cad = /\.(step|stp|iges|igs)$/i.test(args.input)
      const result = { schema_version: 1, measurement: undefined, refinement: undefined, gap_check: undefined }
      if (cad) {
        result.measurement = parse(await runTool('cad_measure', { file: args.input, ...(args.gap ? { gap: args.gap } : { all_pairs: true }) }))
        const measurements = result.measurement.measurements ?? []
        requireCondition(!measurements.some(item => item.touching || item.min_distance_mm === 0), 'gap check failed: CAD bodies are in contact or unresolved')
        const gaps = measurements.map(item => item.min_distance_mm).filter(gap => Number.isFinite(gap) && gap >= 0)
        if (args.spacing_mm) {
          const spacing = vector(args.spacing_mm, 3, 'spacing_mm').map(value => positive(value, 'spacing_mm'))
          const minimum = gaps.length ? Math.min(...gaps) : null
          result.gap_check = { mode: 'error', recommended_spacing_mm: minimum === null ? null : minimum / 10,
            pairs: gaps.map(gap => ({ gap_mm: gap, cells_across: gap / Math.max(...spacing), status: gap === 0 ? 'contact' : gap / Math.max(...spacing) < 2.5 ? 'under-resolved' : 'ok' })) }
          requireCondition(!result.gap_check.pairs.some(pair => pair.status !== 'ok'), 'gap check failed: contact or fewer than 2.5 cells across the CAD gap; inspect contact and refine spacing')
        }
      }
      requireCondition(cad || args.refine, 'GEM/mesh gap check needs refine settings (electrode, out, grid/spacing)')
      if (args.refine) {
        requireCondition(typeof args.refine === 'object' && !Array.isArray(args.refine), 'refine must be an object')
        requireCondition(args.refine.gap_check === undefined, 'check_geometry enforces gap_check error')
        result.refinement = parse(await runTool('refine_geometry', { ...args.refine, input: args.input, ...(args.spacing_mm && { spacing: args.spacing_mm }), gap_check: 'error' }))
      }
      return json(result)
    }),
    tool('analyzer_scan', 'Run validated ESA voltage scans through a PA0 fast-adjust family on CPU. For each energy, generate the same beam and scan absolute electrode volts. Count plane hits only inside every aperture, return connected half-height centroid/FWHM/dE/E and origin-fit k. Both crossings must be bracketed. Width assumes linear analyzer and is beam-dependent; no invented intrinsic resolution.', {
      pa: string, energies_ev: { ...array(number), minItems: 1, maxItems: 64 }, channel_voltages: { ...array(number), minItems: 3, maxItems: 1000 },
      electrode: { type: 'integer', minimum: 1, maximum: 1000 }, fixed_voltage: {}, beam: { type: 'object' },
      plane: array(number), aperture: array(object({ axis: { type: 'string', enum: ['x', 'y', 'z'] }, center_mm: number, width_mm: number })),
      bounds: array(number), origin: array(number), duration_us: number, out: string,
    }, ['pa', 'energies_ev', 'channel_voltages', 'electrode', 'beam', 'plane', 'aperture', 'duration_us', 'out'], false, async args => {
      only(args, ['pa', 'energies_ev', 'channel_voltages', 'electrode', 'fixed_voltage', 'beam', 'plane', 'aperture', 'bounds', 'origin', 'duration_us', 'out'])
      inputFile(args.pa, 'pa')
      requireCondition(Array.isArray(args.energies_ev) && args.energies_ev.length >= 1 && args.energies_ev.length <= 64, 'energies_ev needs 1..64 energies')
      args.energies_ev.forEach(value => positive(value, 'energy_ev'))
      requireCondition(Array.isArray(args.channel_voltages) && args.channel_voltages.length >= 3 && args.channel_voltages.length <= 1000, 'channel_voltages needs 3..1000 voltages')
      args.channel_voltages.forEach((value, i) => requireCondition(Number.isFinite(value) && (!i || value > args.channel_voltages[i - 1]), 'channel_voltages must be finite and strictly increasing'))
      requireCondition(args.energies_ev.length * args.channel_voltages.length <= 10000, 'scan is limited to 10000 flights')
      requireCondition(Number.isInteger(args.electrode) && args.electrode >= 1 && args.electrode <= 1000, 'electrode must be 1..1000')
      requireCondition(args.beam && typeof args.beam === 'object' && !Array.isArray(args.beam), 'beam is required')
      vector(args.plane, 4, 'plane'); positive(args.duration_us, 'duration_us')
      requireCondition(Array.isArray(args.aperture) && args.aperture.length > 0 && args.aperture.length <= 3, 'at least one explicit detector aperture is required')
      for (const aperture of args.aperture) {
        only(aperture, ['axis', 'center_mm', 'width_mm'])
        requireCondition(['x', 'y', 'z'].includes(aperture.axis), 'aperture.axis must be x, y or z')
        finite(aperture.center_mm, 'aperture center'); positive(aperture.width_mm, 'aperture width')
      }
      const out = prepareOutput(args.out, 'out')
      const channels = [], runs = []
      for (const [ei, energy] of args.energies_ev.entries()) {
        const transmission = []
        for (const [vi, voltage] of args.channel_voltages.entries()) {
          const output = `${out}-${ei}-${vi}.csv`
          const fixed = args.fixed_voltage ?? []
          requireCondition(Array.isArray(fixed) && fixed.every(item => typeof item === 'string' && !item.startsWith(`${args.electrode}=`)), 'fixed_voltage must be N=V strings and omit the scanned electrode')
          const run = parse(await runTool('fly', { pa: args.pa, device: 'cpu',
            beam: { ...args.beam, energy_ev: energy, ion_file: `${out}-${ei}.ion` },
            voltage: [...fixed, `${args.electrode}=${voltage}`], plane: args.plane, duration_us: args.duration_us,
            ...(args.bounds && { bounds: args.bounds }), ...(args.origin && { origin: args.origin }), output }))
          const rows = await csvRows(inputFile(output, 'output'))
          requireCondition(rows.length === run.ion_count && rows.length > 0, 'flight CSV row count must match ion_count')
          const hits = rows.filter(row => row.reason === 'plane' && args.aperture.every(a => {
            const position = Number(row[`${a.axis}_mm`])
            requireCondition(Number.isFinite(position), `CSV is missing numeric ${a.axis}_mm`)
            return Math.abs(position - a.center_mm) <= a.width_mm / 2
          })).length
          transmission.push(hits / rows.length)
          runs.push(run)
        }
        channels.push({ energy_ev: energy, voltages_v: args.channel_voltages, transmission, ...passband(args.channel_voltages, transmission) })
      }
      return json({ schema_version: 1, channels, k_v_per_ev: channels.reduce((sum, channel) => sum + channel.centroid_v * channel.energy_ev, 0) / channels.reduce((sum, channel) => sum + channel.energy_ev ** 2, 0),
        voltage_convention: `absolute electrode ${args.electrode} voltage relative to fixed electrodes`, beam: args.beam, aperture: args.aperture,
        compute_ms: runs.reduce((sum, run) => sum + run.compute_ms, 0), runs })
    }),
    tool('grid_convergence', 'Run exactly three exact-surface physical grids, finest first, keeping extent/origin/electrodes/flight fixed. Observable is a flight JSON scalar (e.g. arrival_tof_us.mean). Calls simrep.uncertainty.observed_order (Celik GCI) via SIMREP_PYTHON; requires installed native Python package. Returns value +/- absolute u_num, not residual convergence.', {
      input: string, electrode: array(string), levels: array(object({ grid: array({ type: 'integer' }), spacing: array(number) })),
      origin: array(number), fly: { type: 'object' }, observable: string, out: string,
    }, ['input', 'electrode', 'levels', 'fly', 'observable', 'out'], false, async args => {
      only(args, ['input', 'electrode', 'levels', 'origin', 'fly', 'observable', 'out'])
      inputFile(args.input, 'input')
      requireCondition(Array.isArray(args.levels) && args.levels.length === 3, 'exactly three levels are required, finest first')
      requireCondition(typeof args.observable === 'string' && /^[a-z_]+(?:\.[a-z_]+)*$/.test(args.observable), 'observable must be a dot-separated flight JSON key')
      requireCondition(args.fly && typeof args.fly === 'object' && !Array.isArray(args.fly), 'fly must be flight arguments')
      requireCondition(args.fly.pa === undefined && args.fly.origin === undefined && args.fly.device === undefined, 'grid_convergence owns flight pa, origin and device')
      const spacings = args.levels.map(level => {
        only(level, ['grid', 'spacing'])
        const grid = vector(level.grid, 3, 'grid')
        requireCondition(grid.every((n, i) => Number.isInteger(n) && n >= (i === 2 ? 1 : 2)), 'invalid grid dimensions')
        return vector(level.spacing, 3, 'spacing').map(value => positive(value, 'spacing'))
      })
      const extent = args.levels.map((level, i) => level.grid.map((n, axis) => (n - 1) * spacings[i][axis]))
      requireCondition(extent.every(e => e.every((v, axis) => Math.abs(v - extent[0][axis]) <= Math.max(1e-9, Math.abs(v) * 1e-9))), 'all three grids must cover the same physical extent')
      requireCondition(args.levels.every(level => level.grid.every((n, axis) => (n > 1) === (args.levels[0].grid[axis] > 1))), 'grid dimensionality must stay fixed')
      const h = spacings.map((spacing, i) => {
        const active = spacing.filter((_, axis) => args.levels[i].grid[axis] > 1)
        return active.reduce((a, b) => a * b, 1) ** (1 / active.length)
      })
      requireCondition(h[0] < h[1] && h[1] < h[2], 'grid spacings must be finest first and increasing')
      const out = prepareOutput(args.out, 'out'), values = [], runs = []
      for (const [i, level] of args.levels.entries()) {
        const stem = `${out}-${i}`
        const solve = parse(await runTool('refine_geometry', { input: args.input, electrode: args.electrode, ...level,
          ...(args.origin && { origin: args.origin }), device: 'cpu', gap_check: 'error', out: stem }))
        requireCondition(solve.converged === true, `grid ${i} did not converge`)
        const flight = parse(await runTool('fly', { ...args.fly, ...(args.origin && { origin: args.origin }), pa: `${stem}.pa0`, device: 'cpu', output: `${stem}.csv` }))
        const value = args.observable.split('.').reduce((node, key) => node?.[key], flight)
        finite(value, `observable ${args.observable}`)
        values.push(value); runs.push({ solve, flight })
      }
      const uncertaintyRun = await runHelper(path.join(here, 'grid_uncertainty.py'), [JSON.stringify({ h, values })])
      requireCondition(uncertaintyRun.code === 0 && !uncertaintyRun.killed && !uncertaintyRun.truncated, `Python uncertainty helper failed; install the SimRep native Python package or set SIMREP_PYTHON. ${uncertaintyRun.err}`)
      const uncertainty = JSON.parse(uncertaintyRun.out)
      return json({ schema_version: 1, observable: args.observable, h_mm: h, values, value: values[0], ...uncertainty,
        compute_ms: runs.reduce((sum, run) => sum + run.solve.compute_ms + run.flight.compute_ms, 0), runs })
    }),
  ]
}
