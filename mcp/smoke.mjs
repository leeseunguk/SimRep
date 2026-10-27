#!/usr/bin/env node
// Talk to the SimRep MCP server the way a client does, then run the einzel
// gem2pa -> refine -> fly pipeline on CPU and assert the CLI JSON fields.
//
// The stdio client loop (spawn, pending map, line JSON, check helper) is
// adapted from:
//   https://github.com/using76/Iteration-CFD/blob/1475bbf781e08f33f681661ec40e00727f1e7021/mcp/smoke.mjs
//   path: mcp/smoke.mjs
//   commit: 1475bbf781e08f33f681661ec40e00727f1e7021
//   blob: ecc53242a7120341faea5035bf3a6270a778c8b2
// Copyright Iterations Co., Ltd. (주식회사 이터레이션즈)
// Licence: Prosperity Public License 3.0.0
// Licence text and the adapted-file list: licenses/THIRD_PARTY.md
import { spawn } from 'node:child_process'
import { chmodSync, copyFileSync, existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, symlinkSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import path from 'node:path'
import process from 'node:process'
import readline from 'node:readline'
import { fileURLToPath } from 'node:url'

const here = path.dirname(fileURLToPath(import.meta.url))
const repo = path.resolve(here, '..')
const server = path.join(here, 'server.mjs')
const bin = path.resolve(process.env.SIMREP_BIN || path.join(repo, 'target/release/simrep'))
const fixtureGem = path.join(repo, 'examples/data/einzel.gem')
const fixtureIon = path.join(repo, 'examples/data/ions.ion')
const fixtureCad = path.join(repo, 'tools/cad-fixtures/pair.step')

if (!existsSync(bin)) {
  console.error(`simrep binary not found at ${bin}`)
  console.error('Set SIMREP_BIN or build with: cargo build --release -p simrep-cli')
  process.exit(1)
}
if (!existsSync(fixtureGem) || !existsSync(fixtureIon)) {
  console.error(`missing fixture ${fixtureGem} or ${fixtureIon}`)
  process.exit(1)
}

const workspace = mkdtempSync(path.join(tmpdir(), 'simrep-mcp-smoke-'))
mkdirSync(path.join(workspace, 'examples/data'), { recursive: true })
copyFileSync(fixtureGem, path.join(workspace, 'examples/data/einzel.gem'))
copyFileSync(fixtureIon, path.join(workspace, 'examples/data/ions.ion'))
copyFileSync(fixtureCad, path.join(workspace, 'pair.step'))
console.log(`fixture: ${fixtureGem}`)
console.log(`workspace: ${workspace}`)
console.log(`bin: ${bin}`)

const source = readFileSync(server, 'utf8')
let failures = 0
function check(label, ok, detail = '') {
  console.log(`${ok ? 'ok  ' : 'FAIL'}  ${label}${detail ? ' — ' + detail : ''}`)
  if (!ok) failures++
}

check('server does not enable a shell', !/shell\s*:\s*true/.test(source) && !/\bexecSync\b|\bexecFile\b/.test(source))

function startServer(env) {
  const child = spawn(process.execPath, [server], {
    cwd: workspace,
    env: { ...process.env, ...env, SIMREP_BIN: env.SIMREP_BIN ?? bin, SIMREP_WORKSPACE: env.SIMREP_WORKSPACE ?? workspace },
    stdio: ['pipe', 'pipe', 'pipe'],
  })
  const pending = new Map()
  let nextId = 1
  let stderr = ''
  child.stderr.on('data', (chunk) => {
    stderr += chunk
    if (stderr.length > 200_000) stderr = stderr.slice(-100_000)
  })
  readline.createInterface({ input: child.stdout }).on('line', (line) => {
    if (!line.trim()) return
    let msg
    try {
      msg = JSON.parse(line)
    } catch {
      console.log('  ! not JSON:', line.slice(0, 160))
      return
    }
    const resolve = pending.get(msg.id)
    if (resolve) {
      pending.delete(msg.id)
      resolve(msg)
    }
  })
  function call(method, params, timeoutMs = 180000) {
    const id = nextId++
    return new Promise((resolve, reject) => {
      pending.set(id, resolve)
      child.stdin.write(JSON.stringify({ jsonrpc: '2.0', id, method, params }) + '\n')
      setTimeout(() => {
        if (pending.delete(id)) reject(new Error(`${method} timed out after ${timeoutMs}ms`))
      }, timeoutMs)
    })
  }
  return { child, call, stderr: () => stderr }
}

function textOf(msg) {
  return msg.result?.content?.[0]?.text ?? ''
}

function jsonOf(msg) {
  const text = textOf(msg)
  try {
    return JSON.parse(text)
  } catch {
    return null
  }
}

const session = startServer({})
const { child, call } = session

try {
  const init = await call('initialize', {
    protocolVersion: '2025-06-18',
    capabilities: {},
    clientInfo: { name: 'smoke', version: '0' },
  })
  check('initialize', init.result?.serverInfo?.name === 'simrep', init.result?.serverInfo?.name ?? JSON.stringify(init.error))
  child.stdin.write(JSON.stringify({ jsonrpc: '2.0', method: 'notifications/initialized' }) + '\n')

  const list = await call('tools/list', {})
  const names = (list.result?.tools ?? []).map((tool) => tool.name)
  const expected = [
    'gpu_info', 'pa_info', 'gem2pa', 'mesh_import', 'refine', 'refine_geometry', 'cad_measure', 'fly',
    'export_geometry', 'export_grid', 'export_isosurface', 'export_trajectories', 'read_csv_summary',
    'simrep_plan', 'analyzer_scan', 'grid_convergence', 'compare_runs', 'check_geometry', 'summarize_results',
    'load_skill', 'fetch_tool_output',
  ]
  check('tools/list', expected.every((name) => names.includes(name)) && names.length === expected.length, names.join(', '))
  check('every tool has a schema', (list.result?.tools ?? []).every((tool) => tool.inputSchema?.type === 'object'))

  const invalidCad = await call('tools/call', {
    name: 'cad_measure', arguments: { file: 'pair.step', gap: ['0', '1'], all_pairs: true },
  })
  check('cad_measure refuses conflicting modes', invalidCad.result?.isError === true)
  const cad = await call('tools/call', {
    name: 'cad_measure', arguments: { file: 'pair.step', gap: ['pair/left:faces=0', 'pair/right'] },
  })
  const cadBody = jsonOf(cad)
  if (cad.result?.isError) {
    check('cad_measure reports optional CAD feature', cadBody?.error?.message?.includes('--features cad') === true, textOf(cad).slice(0, 200))
  } else {
    const gap = cadBody?.measurements?.[0]
    check('cad_measure schema_version', cadBody?.schema_version === 1)
    check('cad_measure analytic 0.58 mm gap', Math.abs(gap?.min_distance_mm - 0.58) < 1e-9, String(gap?.min_distance_mm))
    check('cad_measure gap is not touching', gap?.touching === false)
    check('cad_measure complete faces and witnesses', gap?.faces_a?.length === 1 && gap?.faces_b?.length === 1 && gap?.closest_points_mm?.length === 2 && gap?.closest_faces?.length === 2)
    check('cad_measure independent cross-check', gap?.cross_check_error_mm <= cadBody.cad_tolerance_mm)
    const matrix = jsonOf(await call('tools/call', {
      name: 'cad_measure', arguments: { file: 'pair.step', all_pairs: true, touch_tolerance: 0.59, cad_tolerance: 0.001 },
    }))
    check('cad_measure symmetric distance matrix', Math.abs(matrix?.distance_matrix_mm?.[0]?.[1] - 0.58) < 1e-9 && matrix.distance_matrix_mm[0][1] === matrix.distance_matrix_mm[1][0])
    check('cad_measure touch threshold matrix', matrix?.touching_matrix?.[0]?.[1] === true && matrix?.touching_matrix?.[1]?.[0] === true)
    const subset = await call('tools/call', {
      name: 'cad_measure', arguments: { file: 'pair.step', gap: ['0:faces=0,0', '1'] },
    })
    check('cad_measure rejects duplicate face coverage', subset.result?.isError === true)
  }

  const badLine = await new Promise((resolve) => {
    const id = null
    const timer = setTimeout(() => resolve(null), 5000)
    const onLine = (line) => {
      if (!line.trim()) return
      try {
        const msg = JSON.parse(line)
        if (msg.id === id && msg.error) {
          clearTimeout(timer)
          resolve(msg)
        }
      } catch {
        /* ignore */
      }
    }
    // The session reader already consumes lines. Send a parse error and then ping,
    // and treat a subsequent successful ping as proof the bad line did not kill the server.
    child.stdin.write('{not json}\n')
    resolve('sent')
    void onLine
  })
  check('parse error does not kill the server', badLine === 'sent')

  const pong = await call('ping', {})
  check('ping', pong.result !== undefined && pong.error === undefined)

  const missing = await call('tools/call', { name: 'nope', arguments: {} })
  check('unknown tool is a JSON-RPC error', missing.error?.code === -32602, String(missing.error?.code))

  const bad = await call('does/not/exist', {})
  check('unknown method is method-not-found', bad.error?.code === -32601, String(bad.error?.code))

  const escape = await call('tools/call', { name: 'pa_info', arguments: { file: '../../../etc/passwd' } })
  check('refuses a path outside the workspace', escape.result?.isError === true, textOf(escape).slice(0, 120))

  const absolute = await call('tools/call', { name: 'pa_info', arguments: { file: '/etc/passwd' } })
  check('refuses an absolute path outside the workspace', absolute.result?.isError === true, textOf(absolute).slice(0, 120))

  const link = path.join(workspace, 'escape-link')
  symlinkSync('/etc/passwd', link)
  const linked = await call('tools/call', { name: 'pa_info', arguments: { file: 'escape-link' } })
  check('refuses a symlink that leaves the workspace', linked.result?.isError === true, textOf(linked).slice(0, 120))

  const marker = path.join(workspace, 'not-created')
  const injected = await call('tools/call', {
    name: 'pa_info',
    arguments: { file: `examples/data/einzel.gem; touch ${marker}` },
  })
  check('metacharacters are not a shell command', injected.result?.isError === true && !existsSync(marker), textOf(injected).slice(0, 120))

  const gpu = await call('tools/call', { name: 'gpu_info', arguments: {} }, 120000)
  const gpuBody = jsonOf(gpu)
  if (gpu.result?.isError) {
    check('gpu_info failure carries stderr', typeof gpuBody?.error?.stderr === 'string' && gpuBody.error.stderr.length > 0, gpuBody?.error?.message?.slice(0, 120))
  } else {
    check('gpu_info adapter_name', typeof gpuBody?.adapter_name === 'string', gpuBody?.adapter_name)
    check('gpu_info schema_version', gpuBody?.schema_version === 1)
  }

  const gem = await call('tools/call', {
    name: 'gem2pa',
    arguments: { input: 'examples/data/einzel.gem', output: 'out/einzel.pa#' },
  })
  const gemBody = jsonOf(gem)
  check('gem2pa is JSON', gem.result?.isError !== true && gemBody?.schema_version === 1, textOf(gem).slice(0, 200))
  check('gem2pa dims', Array.isArray(gemBody?.dims) && gemBody.dims[0] === 121 && gemBody.dims[1] === 21 && gemBody.dims[2] === 1, JSON.stringify(gemBody?.dims))
  check('gem2pa spacing_mm', Array.isArray(gemBody?.spacing_mm) && gemBody.spacing_mm[0] === 0.5 && gemBody.spacing_mm[1] === 0.5, JSON.stringify(gemBody?.spacing_mm))
  check('gem2pa electrode_numbers', [1, 2, 3].every((n) => gemBody?.electrode_numbers?.includes(n)), JSON.stringify(gemBody?.electrode_numbers))
  check('gem2pa compute_ms', typeof gemBody?.compute_ms === 'number')
  check('gem2pa output_paths', Array.isArray(gemBody?.output_paths) && gemBody.output_paths.length > 0, JSON.stringify(gemBody?.output_paths))
  check('gem2pa simrep_version', typeof gemBody?.simrep_version === 'string')

  const paHash = gemBody?.output_paths?.find((p) => p.endsWith('.pa#')) ?? path.join(workspace, 'out/einzel.pa#')
  const info = await call('tools/call', { name: 'pa_info', arguments: { file: paHash } })
  const infoBody = jsonOf(info)
  check('pa_info symmetry', info.result?.isError !== true && infoBody?.symmetry === 'cylindrical', infoBody?.symmetry)
  check('pa_info schema_version', infoBody?.schema_version === 1)
  check('pa_info points', typeof infoBody?.points === 'number' && infoBody.points > 0)

  const refined = await call('tools/call', {
    name: 'refine',
    arguments: { file: paHash, device: 'cpu' },
  })
  const refinedBody = jsonOf(refined)
  check('refine converged', refined.result?.isError !== true && refinedBody?.converged === true, textOf(refined).slice(0, 240))
  check('refine executed cpu', refinedBody?.executed?.device === 'cpu', JSON.stringify(refinedBody?.executed))
  check('refine requested cpu', refinedBody?.requested?.device === 'cpu')
  check('refine cycles', typeof refinedBody?.cycles === 'number' && refinedBody.cycles >= 0)
  check('refine final_residual', typeof refinedBody?.final_residual === 'number')
  check('refine tolerance', typeof refinedBody?.tolerance === 'number')
  check('refine compute_ms', typeof refinedBody?.compute_ms === 'number')
  check('refine prep_ms', typeof refinedBody?.prep_ms === 'number')
  check('refine schema_version', refinedBody?.schema_version === 1)
  const pa0 = refinedBody?.output_paths?.find((p) => String(p).endsWith('.pa0'))
  check('refine wrote pa0', typeof pa0 === 'string' && existsSync(pa0), pa0)

  const flown = await call('tools/call', {
    name: 'fly',
    arguments: {
      pa: pa0,
      particles: 'examples/data/ions.ion',
      voltage: ['1=0', '2=60', '3=0'],
      plane: [1, 0, 0, 58],
      device: 'cpu',
      duration_us: 2000,
      output: 'out/results.csv',
    },
  })
  const flyBody = jsonOf(flown)
  check('fly schema_version', flown.result?.isError !== true && flyBody?.schema_version === 1, textOf(flown).slice(0, 300))
  check('fly ion_count', flyBody?.ion_count === 8, String(flyBody?.ion_count))
  check('fly executed cpu', flyBody?.executed?.device === 'cpu', JSON.stringify(flyBody?.executed))
  check('fly requested cpu', flyBody?.requested?.device === 'cpu')
  check('fly arrivals', typeof flyBody?.arrivals === 'number' && flyBody.arrivals > 0, String(flyBody?.arrivals))
  check('fly splat_counts', flyBody?.splat_counts && typeof flyBody.splat_counts === 'object' && 'plane' in flyBody.splat_counts, JSON.stringify(flyBody?.splat_counts))
  check('fly arrival_tof_us', flyBody?.arrival_tof_us && 'mean' in flyBody.arrival_tof_us && 'std' in flyBody.arrival_tof_us)
  check('fly compute_ms', typeof flyBody?.compute_ms === 'number')
  check('fly prep_ms', typeof flyBody?.prep_ms === 'number')
  check('fly reason', typeof flyBody?.reason === 'string')
  const csv = flyBody?.output_paths?.[0]
  check('fly output_paths', typeof csv === 'string' && existsSync(csv), csv)

  const summary = await call('tools/call', { name: 'read_csv_summary', arguments: { path: csv } })
  const summaryBody = jsonOf(summary)
  check('read_csv_summary rows', summary.result?.isError !== true && summaryBody?.row_count === 8, String(summaryBody?.row_count))
  check('read_csv_summary columns', summaryBody?.columns?.includes('tof_us') && summaryBody.columns.includes('reason'))
  check('read_csv_summary tof', typeof summaryBody?.numeric?.tof_us?.mean === 'number')
  check('read_csv_summary reasons', typeof summaryBody?.reason_counts?.plane === 'number' && summaryBody.reason_counts.plane === flyBody.arrivals)

  const plan = jsonOf(await call('tools/call', { name: 'simrep_plan', arguments: { task: 'einzel focusing' } }))
  check('simrep_plan selects recipe and workspace input', plan?.skill === 'einzel' && plan?.inputs?.geometry === 'examples/data/einzel.gem')
  const table = jsonOf(await call('tools/call', { name: 'summarize_results', arguments: { paths: [csv] } }))
  check('summarize_results compact numeric table', table?.rows?.[0]?.rows === 8 && typeof table?.rows?.[0]?.numeric?.tof_us?.mean === 'number')
  const identity = jsonOf(await call('tools/call', { name: 'compare_runs', arguments: { before: csv, after: csv } }))
  check('compare_runs uses real identity helper', identity?.identical === true && identity?.files_compared === 1)
  const recipe = jsonOf(await call('tools/call', { name: 'load_skill', arguments: { id: 'esa' } }))
  check('load_skill loads one trusted recipe', recipe?.skill === 'esa')
  const fullSkill = jsonOf(await call('tools/call', { name: 'load_skill', arguments: { id: 'ion-optics' } }))
  if (fullSkill?.truncated) {
    const page = jsonOf(await call('tools/call', { name: 'fetch_tool_output', arguments: { handle: fullSkill.handle, length: 1000 } }))
    check('large skill has a retrievable output handle', page?.text?.length === 1000 && page?.next_offset === 1000)
  } else check('shared skill is returned intact', fullSkill?.skill === 'ion-optics' && typeof fullSkill?.text === 'string')
  for (const name of ['compare_runs', 'check_geometry', 'grid_convergence', 'analyzer_scan']) {
    const argumentsForTool = name === 'compare_runs' ? { before: 'escape-link', after: csv }
      : name === 'analyzer_scan' ? { pa: '../outside.pa0' } : { input: '../outside.gem' }
    const refused = await call('tools/call', { name, arguments: argumentsForTool })
    check(`${name} refuses workspace escapes`, refused.result?.isError === true)
  }
  const geometryPacket = await call('tools/call', { name: 'check_geometry', arguments: {
    input: 'examples/data/einzel.gem',
    refine: { electrode: ['1=1:0', '2=2:60', '3=3:0'], device: 'cpu', out: 'out/exact' },
  } })
  check('check_geometry refuses unresolved GEM gaps without disabling the gate', geometryPacket.result?.isError === true && textOf(geometryPacket).includes('gap check failed'), textOf(geometryPacket).slice(0, 240))
  // Cylindrical GEM bounds can be uncertified under the gap search budget.
  // Use exact parallel box faces to prove the successful composite path too.
  writeFileSync(path.join(workspace, 'plates.gem'), 'pa_define(9,5,5,planar)\ne(1){fill{within{box3d(-1,-1,-1,1,3,3)}}}\ne(2){fill{within{box3d(3,-1,-1,5,3,3)}}}\n')
  const platePacket = await call('tools/call', { name: 'check_geometry', arguments: {
    input: 'plates.gem',
    refine: { electrode: ['1=1:0', '2=2:10'], grid: [17,9,9], spacing: [0.25,0.25,0.25], device: 'cpu', out: 'out/plates' },
  } })
  const plateCheck = jsonOf(platePacket)
  check('check_geometry runs certified exact refinement with mandatory gap gate', plateCheck?.refinement?.converged === true && plateCheck?.refinement?.gap_check?.mode === 'error', textOf(platePacket).slice(0, 240))
  if (process.env.SIMREP_PYTHON) {
    const studyPacket = await call('tools/call', { name: 'grid_convergence', arguments: {
      input: 'plates.gem', electrode: ['1=1:0', '2=2:10'],
      levels: [
        { grid: [33,17,17], spacing: [0.125,0.125,0.125] },
        { grid: [17,9,9], spacing: [0.25,0.25,0.25] },
        { grid: [9,5,5], spacing: [0.5,0.5,0.5] },
      ],
      fly: { beam: { count: 4, mass_amu: 40, energy_ev: 1000, position_mm: [1.5,1,1], ion_file: 'out/study.ion' }, plane: [1,0,0,2.5], duration_us: 100 },
      observable: 'arrival_tof_us.mean', out: 'out/study',
    } })
    const study = jsonOf(studyPacket)
    check('grid_convergence runs three native solves and Celik helper', study?.values?.length === 3 && typeof study?.value === 'number' && Object.hasOwn(study, 'u_num'), textOf(studyPacket).slice(0, 700))
    check('grid_convergence keeps solver and flight evidence', study?.runs?.length === 3 && study.runs.every(run => run.solve.converged && run.flight.executed.device === 'cpu'))
  }

  const beam = await call('tools/call', {
    name: 'fly',
    arguments: {
      pa: pa0,
      beam: {
        count: 4,
        mass_amu: 40,
        charge_e: 1,
        energy_ev: 1000,
        position_mm: [1, 1, 0],
        direction: [1, 0, 0],
        line_axis: 'y',
        line_mm: 1,
        ion_file: 'out/beam.ion',
      },
      voltage: '1=0,2=60,3=0',
      plane: [1, 0, 0, 58],
      bounds: [-1, -1, -1, 80, 20, 20],
      bfield: [0, 0, 0],
      device: 'cpu',
      duration_us: 2000,
      output: 'out/beam.csv',
    },
  })
  const beamBody = jsonOf(beam)
  check('generated beam fly', beam.result?.isError !== true && beamBody?.ion_count === 4 && beamBody?.executed?.device === 'cpu' && beamBody?.arrivals > 0, textOf(beam).slice(0, 240))
  check('generated beam ion file', existsSync(path.join(workspace, 'out/beam.ion')))

  const grid = await call('tools/call', {
    name: 'export_grid',
    arguments: { input: pa0, output: 'out/einzel.vti' },
  })
  const gridBody = jsonOf(grid)
  check('export_grid output_paths', grid.result?.isError !== true && gridBody?.schema_version === 1 && existsSync(gridBody?.output_paths?.[0]), JSON.stringify(gridBody?.output_paths))

  writeFileSync(path.join(workspace, 'samples.csv'), 'trajectory_id,time_us,x_mm,y_mm,z_mm,vx_mm_per_us,vy_mm_per_us,vz_mm_per_us\n1,0,0,0,0,1,0,0\n1,1,1,0,0,1,0,0\n')
  const curves = await call('tools/call', {
    name: 'export_trajectories',
    arguments: { input: 'samples.csv', output: 'out/curves.usda' },
  })
  const curvesBody = jsonOf(curves)
  check('export_trajectories USDA output_paths', curves.result?.isError !== true && existsSync(curvesBody?.output_paths?.[0]), JSON.stringify(curvesBody?.output_paths))
  const scene = await call('tools/call', {
    name: 'export_geometry',
    arguments: { input: pa0, output: 'out/scene.usda', trajectories: 'samples.csv' },
  })
  const sceneBody = jsonOf(scene)
  check('export_geometry combined USDA output_paths', scene.result?.isError !== true && existsSync(sceneBody?.output_paths?.[0]), JSON.stringify(sceneBody?.output_paths))
  if (scene.result?.isError !== true && existsSync(sceneBody?.output_paths?.[0])) {
    const usda = readFileSync(sceneBody.output_paths[0], 'utf8')
    check('USDA contains electrode meshes and trajectory curves', usda.startsWith('#usda 1.0\n') && usda.includes('def Mesh "part_0"') && usda.includes('def BasisCurves "Trajectories"'))
  }

  const wrong = await call('tools/call', {
    name: 'export_trajectories',
    arguments: { input: csv, output: 'out/wrong.vtp' },
  })
  const wrongBody = jsonOf(wrong)
  check('export_trajectories rejects a fly CSV', wrong.result?.isError === true && typeof wrongBody?.error?.message === 'string', wrongBody?.error?.message?.slice(0, 140))
  check('export_trajectories error includes stderr', typeof wrongBody?.error?.stderr === 'string' && wrongBody.error.stderr.length > 0)

  const both = await call('tools/call', {
    name: 'fly',
    arguments: { pa: pa0, particles: 'examples/data/ions.ion', beam: { count: 1, mass_amu: 1, energy_ev: 1, position_mm: [0, 0, 0], ion_file: 'out/nope.ion' } },
  })
  check('fly rejects particles and beam together', both.result?.isError === true, textOf(both).slice(0, 120))
} catch (error) {
  failures++
  console.log(`FAIL  unexpected — ${error.stack || error.message}`)
  console.log(session.stderr().slice(-2000))
} finally {
  child.stdin.end()
  child.kill('SIGTERM')
}

// Timeout and missing-binary are separate server processes.
const wrapper = path.join(workspace, 'slow-bin')
writeFileSync(wrapper, '#!/bin/sh\necho still-going >&2\nsleep 30\n')
chmodSync(wrapper, 0o755)
const timed = startServer({ SIMREP_BIN: wrapper, SIMREP_MCP_TIMEOUT_S: '1' })
try {
  await timed.call('initialize', { protocolVersion: '2025-06-18', capabilities: {}, clientInfo: { name: 'smoke', version: '0' } })
  timed.child.stdin.write(JSON.stringify({ jsonrpc: '2.0', method: 'notifications/initialized' }) + '\n')
  const timedOut = await timed.call('tools/call', { name: 'gpu_info', arguments: {} }, 15000)
  const timedBody = jsonOf(timedOut)
  check('timeout kills the child', timedOut.result?.isError === true && timedBody?.error?.kind === 'timeout', textOf(timedOut).slice(0, 180))
  check('timeout captures stderr', typeof timedBody?.error?.stderr === 'string' && timedBody.error.stderr.includes('still-going'))
} catch (error) {
  failures++
  console.log(`FAIL  timeout case — ${error.message}`)
} finally {
  timed.child.kill('SIGTERM')
}

const missingBin = startServer({ SIMREP_BIN: path.join(workspace, 'no-such-simrep') })
try {
  await missingBin.call('initialize', { protocolVersion: '2025-06-18', capabilities: {}, clientInfo: { name: 'smoke', version: '0' } })
  const missingCall = await missingBin.call('tools/call', { name: 'gpu_info', arguments: {} })
  check('missing SIMREP_BIN is a tool error', missingCall.result?.isError === true && textOf(missingCall).includes('SIMREP_BIN'), textOf(missingCall).slice(0, 160))
} catch (error) {
  failures++
  console.log(`FAIL  missing binary — ${error.message}`)
} finally {
  missingBin.child.kill('SIGTERM')
}

rmSync(workspace, { recursive: true, force: true })
console.log(failures ? `\n${failures} check(s) failed` : '\nall checks passed')
process.exit(failures ? 1 : 0)
