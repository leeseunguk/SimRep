// Design search uses the existing workspace guards and MCP workflow implementations.
import { readFile, writeFile, mkdir, rename, readdir, lstat } from 'node:fs/promises'
import { createHash } from 'node:crypto'
import path from 'node:path'

const fail = message => { throw new Error(message) }
const check = (condition, message) => { if (!condition) fail(message) }
const obj = x => x && typeof x === 'object' && !Array.isArray(x)
const finite = x => typeof x === 'number' && Number.isFinite(x)
const keys = (x, allowed) => { check(obj(x), 'expected an object'); for (const k of Object.keys(x)) check(allowed.includes(k), `unknown argument ${k}`) }
const parse = result => { let data; try { data = JSON.parse(result.text) } catch { fail(result.text) }; if (result.isError || data.error) fail(data.error?.message ?? result.text); return data }
async function rejectSymlinks(dir) {
  for (const name of await readdir(dir)) {
    const file = path.join(dir, name), stat = await lstat(file)
    check(!stat.isSymbolicLink(), `symlink in evaluation directory: ${file}`)
    if (stat.isDirectory()) await rejectSymlinks(file)
  }
}
const warnings = (value, out = []) => { if (!obj(value) && !Array.isArray(value)) return out; for (const [k, v] of Object.entries(value)) { if (k === 'warnings' && Array.isArray(v)) out.push(...v.map(String)); else if (v && typeof v === 'object') warnings(v, out) }; return out }

export function optimizeTool({ inputFile, insideWorkspace, prepareOutput, runTool, timeoutSeconds }) {
  return {
    name: 'optimize_design', title: 'Optimize design', readOnly: false,
    description: 'Write and evaluate a numeric GEM template in bounded per-evaluation folders. Requires approval. Resumes an identical run from history; budget can be increased. Infeasible objective is represented as "+inf".',
    inputSchema: { type: 'object', additionalProperties: false, required: ['template', 'parameters', 'electrodes', 'objective', 'out'], properties: {
      template: { type: 'string' }, parameters: { type: 'array', minItems: 1, maxItems: 6, items: { type: 'object', additionalProperties: false, required: ['name', 'min', 'max', 'initial'], properties: { name: { type: 'string' }, min: { type: 'number' }, max: { type: 'number' }, initial: { type: 'number' }, step: { type: 'number' } } } },
      electrodes: { type: 'array', items: { type: 'string' }, minItems: 1 }, objective: { type: 'object', description: 'flight_scalar: key, fly, target?, minimize?; analyzer: analyzer_scan inputs, targets and weights.' }, out: { type: 'string' }, refine: { type: 'object', description: 'Optional refine_geometry settings, excluding input, electrode, out and gap_check.' }, fixed: { type: 'object', description: 'Fixed named numeric placeholders; gap_check is always error.' }, budget: { type: 'integer', minimum: 1, maximum: 100, default: 20 }, method: { type: 'string', enum: ['nelder-mead', 'grid'] }, xtol: { type: 'number', exclusiveMinimum: 0 }, ftol: { type: 'number', exclusiveMinimum: 0 }, confirm_grid: { type: 'boolean' }
    } },
    async run(args) {
      keys(args, ['template','parameters','electrodes','objective','out','refine','fixed','budget','method','xtol','ftol','confirm_grid'])
      const template = await readFile(inputFile(args.template, 'template'), 'utf8')
      check(/\.gem$/i.test(args.template), 'template must be a GEM file')
      check(Array.isArray(args.parameters) && args.parameters.length >= 1 && args.parameters.length <= 6, 'parameters needs 1..6 entries')
      const names = new Set()
      for (const p of args.parameters) { keys(p, ['name','min','max','initial','step']); check(typeof p.name === 'string' && /^[A-Za-z_][A-Za-z0-9_]*$/.test(p.name) && !names.has(p.name), 'parameter name must be a unique identifier'); names.add(p.name); check([p.min,p.max,p.initial].every(finite) && p.min < p.max && p.initial >= p.min && p.initial <= p.max && (p.step === undefined || finite(p.step) && p.step > 0), `invalid bounds or step for ${p.name}`) }
      const fixed = args.fixed ?? {}; check(obj(fixed), 'fixed must be an object of named numeric values')
      for (const [name, value] of Object.entries(fixed)) { check(/^[A-Za-z_][A-Za-z0-9_]*$/.test(name) && !names.has(name) && finite(value), `invalid fixed parameter ${name}`); names.add(name) }
      const matches = [...template.matchAll(/\{\{([A-Za-z_][A-Za-z0-9_]*)\}\}/g)].map(m => m[1])
      check(!template.replace(/\{\{[A-Za-z_][A-Za-z0-9_]*\}\}/g, '').match(/\{\{|\}\}/), 'invalid placeholder syntax')
      check(matches.every(n => names.has(n)) && [...names].every(n => matches.includes(n)), 'placeholder names must match parameters and fixed values')
      check(Array.isArray(args.electrodes) && args.electrodes.length && args.electrodes.every(x => typeof x === 'string'), 'electrodes required')
      const budget = args.budget ?? 20, method = args.method ?? 'nelder-mead', xtol = args.xtol ?? 1e-4, ftol = args.ftol ?? 1e-6
      check(Number.isInteger(budget) && budget >= 1 && budget <= 100, 'budget must be 1..100')
      check(['nelder-mead','grid'].includes(method) && (method !== 'grid' || args.parameters.length <= 2), 'grid supports only 1..2 parameters')
      check(finite(xtol) && xtol > 0 && finite(ftol) && ftol > 0, 'xtol and ftol must be positive')
      check(args.confirm_grid === undefined || typeof args.confirm_grid === 'boolean', 'confirm_grid must be boolean')
      const refine = args.refine ?? {}; check(obj(refine), 'refine must be an object'); for (const k of ['input','electrode','out','gap_check']) check(refine[k] === undefined, `refine.${k} is controlled by optimize_design`)
      const objective = args.objective; check(obj(objective), 'objective is required')
      if (objective.kind === 'flight_scalar') { keys(objective, ['kind','key','fly','target','minimize']); check(typeof objective.key === 'string' && /^[a-z_]+(?:\.[a-z_]+)*$/.test(objective.key), 'invalid flight scalar key'); check(obj(objective.fly), 'objective.fly is required'); for (const k of ['pa','output','device']) check(objective.fly[k] === undefined, `objective.fly.${k} is controlled by optimize_design`); check(objective.target === undefined || finite(objective.target), 'target must be finite'); check(objective.minimize === undefined || typeof objective.minimize === 'boolean', 'minimize must be boolean') }
      else if (objective.kind === 'analyzer') { check(obj(objective.targets) && obj(objective.weights), 'analyzer targets and weights are required'); keys(objective.targets, ['k_v_per_ev','de_over_e']); keys(objective.weights, ['k_v_per_ev','de_over_e']); check(Object.keys(objective.targets).length > 0, 'analyzer needs a target'); for (const [k,v] of Object.entries(objective.targets)) check(finite(v) && finite(objective.weights[k]) && objective.weights[k] > 0, `invalid analyzer target or weight ${k}`); for (const k of Object.keys(objective.weights)) check(k in objective.targets, `weight ${k} has no target`) }
      else fail('objective.kind must be flight_scalar or analyzer')
      const root = insideWorkspace(args.out, 'out'); await mkdir(root, { recursive: true }); insideWorkspace(root, 'out')
      const historyPath = prepareOutput(path.join(root, 'history.json'), 'history')
      const fingerprint = createHash('sha256').update(JSON.stringify({ template, parameters: args.parameters, electrodes: args.electrodes, objective, refine, fixed, method, xtol, ftol })).digest('hex')
      let history = []
      try { const saved = JSON.parse(await readFile(inputFile(historyPath, 'history'), 'utf8')); check(saved.fingerprint === fingerprint && Array.isArray(saved.history), 'history does not match this design'); history = saved.history } catch (error) { if (error.code !== 'ENOENT' && !String(error.message).includes('does not exist:')) throw error }
      check(history.length <= budget, 'budget is smaller than existing history')
      let cursor = 0, expired = false
      const deadline = Date.now() + timeoutSeconds * 1000
      const save = async () => { const tmp = prepareOutput(path.join(root, 'history.tmp'), 'history'); await writeFile(tmp, JSON.stringify({ fingerprint, history }, null, 2)); await rename(tmp, historyPath) }
      const evaluate = async (point, confirmation = false) => {
        if (Date.now() >= deadline) { expired = true; return null }
        const values = Object.fromEntries(args.parameters.map((p,i) => [p.name, Math.max(p.min, Math.min(p.max, point[i]))]))
        if (!confirmation && cursor < history.length) { const entry = history[cursor++]; check(JSON.stringify(entry.parameters) === JSON.stringify(values), 'history replay mismatch'); return entry }
        if (!confirmation && cursor >= budget) return null
        const dir = insideWorkspace(path.join(root, confirmation ? 'confirm' : `eval-${String(cursor + 1).padStart(4,'0')}`), 'evaluation'); await mkdir(dir, { recursive: true }); insideWorkspace(dir, 'evaluation'); await rejectSymlinks(dir)
        const gem = prepareOutput(path.join(dir, 'model.gem'), 'model'); const all = { ...fixed, ...values }
        await writeFile(gem, template.replace(/\{\{([A-Za-z_][A-Za-z0-9_]*)\}\}/g, (_, name) => String(all[name])))
        const stem = path.join(dir, 'field'); let cost = 0, notes = [], value = '+inf', reason = null, metric = null
        try {
          const r = { ...refine }; if (confirmation) { check(Array.isArray(r.grid) && Array.isArray(r.spacing), 'confirm_grid requires explicit refine.grid and refine.spacing'); r.grid = r.grid.map(n => n === 1 ? 1 : 2 * (n - 1) + 1); r.spacing = r.spacing.map(x => x / 2) }
          const solve = parse(await runTool('refine_geometry', { ...r, input: gem, electrode: args.electrodes, out: stem, gap_check: 'error', device: 'cpu' })); cost += solve.compute_ms ?? 0; notes.push(...warnings(solve)); check(solve.converged === true, 'refinement did not converge')
          if (Date.now() >= deadline) { expired = true; fail('optimization timeout') }
          if (objective.kind === 'flight_scalar') {
            const fly = { ...objective.fly }; if (fly.beam) fly.beam = { ...fly.beam, ion_file: path.join(dir, 'beam.ion') }; const result = parse(await runTool('fly', { ...fly, seed: fly.seed ?? 1, pa: `${stem}.pa0`, output: path.join(dir, 'flight.csv'), device: 'cpu' })); cost += result.compute_ms ?? 0; notes.push(...warnings(result)); check(result.arrivals > 0, 'no transmission'); metric = objective.key.split('.').reduce((x,k) => x?.[k], result); check(finite(metric), `flight scalar ${objective.key} is not finite`); value = objective.target !== undefined ? Math.abs(metric - objective.target) : (objective.minimize === false ? -metric : metric)
          } else {
            const { kind, targets, weights, ...scan } = objective; check(scan.pa === undefined && scan.out === undefined, 'analyzer pa/out are controlled by optimize_design'); const result = parse(await runTool('analyzer_scan', { ...scan, pa: `${stem}.pa0`, out: path.join(dir, 'scan') })); cost += result.compute_ms ?? 0; notes.push(...warnings(result)); metric = { k_v_per_ev: result.k_v_per_ev, de_over_e: result.channels.reduce((s,c) => s + c.dE_over_E, 0) / result.channels.length }; value = Object.entries(targets).reduce((s,[k,v]) => { check(finite(metric[k]), `analyzer ${k} is not finite`); return s + weights[k] * (metric[k] - v) ** 2 }, 0)
          }
          check(finite(value), 'objective is not finite')
        } catch (error) { if (Date.now() >= deadline) expired = true; reason = error.message; notes.push(reason); value = '+inf' }
        const entry = { parameters: values, objective: value, metric, reason, warnings: [...new Set(notes)], compute_ms: cost }
        if (!confirmation) { history.push(entry); cursor++; await save() }
        return entry
      }
      const score = e => e?.objective === '+inf' ? Infinity : e?.objective ?? Infinity
      const n = args.parameters.length, initial = args.parameters.map(p => p.initial)
      let converged = false
      if (method === 'grid') {
        const axes = args.parameters.map(p => { const step = p.step ?? (p.max-p.min)/4; const count = Math.min(101, Math.floor((p.max-p.min)/step)+1); return Array.from({length:count}, (_,i) => p.min+i*step) })
        const points = n === 1 ? axes[0].map(x => [x]) : axes[0].flatMap(x => axes[1].map(y => [x,y]))
        for (const point of points) if (!await evaluate(point)) break
      } else {
        const simplex = []; let first = await evaluate(initial); if (first) simplex.push({ p: initial, e: first })
        for (let i=0; i<n && simplex.length === i+1; i++) { const p = [...initial], d = args.parameters[i].step ?? (args.parameters[i].max-args.parameters[i].min)*0.05; p[i] = Math.min(args.parameters[i].max, initial[i]+d); if (p[i] === initial[i]) p[i] = Math.max(args.parameters[i].min, initial[i]-d); const e = await evaluate(p); if (e) simplex.push({p,e}) }
        const clamp = p => p.map((v,i) => Math.max(args.parameters[i].min, Math.min(args.parameters[i].max,v)))
        while (simplex.length === n+1 && !expired) {
          simplex.sort((a,b) => score(a.e)-score(b.e)); const best = simplex[0], worst = simplex[n]
          const size = Math.max(...simplex.slice(1).flatMap(x => x.p.map((v,i) => Math.abs(v-best.p[i])))), spread = Math.max(...simplex.map(x => Math.abs(score(x.e)-score(best.e))))
          if (Number.isFinite(score(best.e)) && size <= xtol && spread <= ftol) { converged = true; break }
          const center = initial.map((_,i) => simplex.slice(0,n).reduce((s,x) => s+x.p[i],0)/n)
          const candidate = async p => { p = clamp(p); const e = await evaluate(p); return e && {p,e} }
          const r = await candidate(center.map((x,i) => 2*x-worst.p[i])); if (!r) break
          if (score(r.e) < score(best.e)) { const e = await candidate(center.map((x,i) => x+2*(r.p[i]-x))); if (!e) break; simplex[n] = score(e.e) < score(r.e) ? e : r }
          else if (score(r.e) < score(simplex[n-1].e)) simplex[n] = r
          else { const outside = score(r.e) < score(worst.e); const c = await candidate(center.map((x,i) => x+0.5*((outside ? r.p[i] : worst.p[i])-x))); if (!c) break; if (score(c.e) < (outside ? score(r.e) : score(worst.e))) simplex[n] = c; else { let complete = true; for (let i=1;i<=n;i++) { const s = await candidate(best.p.map((x,j) => x+0.5*(simplex[i].p[j]-x))); if (!s) { complete=false; break }; simplex[i]=s } if (!complete) break } }
        }
      }
      const feasible = history.filter(e => finite(e.objective)).sort((a,b) => a.objective-b.objective), best = feasible[0] ?? null
      let confirmation = null
      if (args.confirm_grid && best && !expired) { confirmation = await evaluate(args.parameters.map(p => best.parameters[p.name]), true); if (confirmation && finite(confirmation.objective)) confirmation.change = confirmation.objective-best.objective }
      return { text: JSON.stringify({ schema_version: 1, best_parameters: best?.parameters ?? null, best_objective: best?.objective ?? '+inf', history, converged, verdict: converged ? 'converged' : 'budget_exhausted', ...(expired && { timeout: true }), recommendation_note: 'A non-converged result is a candidate, not an optimum. Quantities not in the objective were not checked.', confirmation, history_file: historyPath }), isError: false }
    },
  }
}
