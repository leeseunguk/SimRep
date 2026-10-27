#!/usr/bin/env node
// Stdio MCP server that runs the simrep CLI and returns its --json summary.
//
// The JSON-RPC loop (initialize, tools/list, tools/call, ping, notifications),
// the workspace path refusal, and the argv-only child spawn are adapted from:
//   https://github.com/using76/Iteration-CFD/blob/1475bbf781e08f33f681661ec40e00727f1e7021/mcp/server.mjs
//   path: mcp/server.mjs
//   commit: 1475bbf781e08f33f681661ec40e00727f1e7021
//   blob: c6097254d905e7c78c7e7731ca2bbbdd45490431
// Copyright Iterations Co., Ltd. (주식회사 이터레이션즈)
// Licence: Prosperity Public License 3.0.0
// Licence text and the adapted-file list: licenses/THIRD_PARTY.md
// The SimRep tool definitions below are not from that repository.
//
// No shell. Every child is spawn() with an argument array. The executable is
// SIMREP_BIN, never a caller string. Every caller path is resolved and refused
// unless it stays inside SIMREP_WORKSPACE, including after symlink resolution.
import { spawn } from 'node:child_process'
import { createReadStream, existsSync, mkdirSync, realpathSync, statSync, writeFileSync } from 'node:fs'
import path from 'node:path'
import process from 'node:process'
import readline from 'node:readline'
import { fileURLToPath } from 'node:url'

const PROTOCOL_VERSION = '2025-06-18'
const SERVER = { name: 'simrep', version: '0.1.0' }
const STDOUT_CAP = 8_000_000
const STDERR_CAP = 1_000_000
const CSV_ROW_CAP = 2_000_000

const here = path.dirname(fileURLToPath(import.meta.url))

function workspaceRoot() {
  const raw = process.env.SIMREP_WORKSPACE
    ? path.resolve(process.env.SIMREP_WORKSPACE)
    : process.cwd()
  if (!existsSync(raw)) {
    console.error(`SIMREP_WORKSPACE does not exist: ${raw}`)
    process.exit(1)
  }
  const real = realpathSync(raw)
  if (!statSync(real).isDirectory()) {
    console.error(`SIMREP_WORKSPACE is not a directory: ${real}`)
    process.exit(1)
  }
  return real
}

function defaultBin() {
  if (process.env.SIMREP_BIN) return path.resolve(process.env.SIMREP_BIN)
  const exe = process.platform === 'win32' ? '.exe' : ''
  for (const rel of ['../target/release/simrep', '../target/debug/simrep']) {
    const abs = path.resolve(here, rel) + exe
    if (existsSync(abs)) return abs
  }
  return null
}

const WORKSPACE = workspaceRoot()
const children = new Set()
function shutdown() {
  for (const child of children) {
    if (process.platform !== 'win32') {
      try { process.kill(-child.pid, 'SIGKILL'); continue } catch { /* fall through */ }
    }
    child.kill('SIGKILL')
  }
  process.exit(0)
}
process.on('SIGTERM', shutdown)
process.on('SIGINT', shutdown)
const SIMREP_BIN = defaultBin()
const TIMEOUT_S = Number(process.env.SIMREP_MCP_TIMEOUT_S || 3600)
if (!Number.isFinite(TIMEOUT_S) || TIMEOUT_S <= 0) {
  console.error('SIMREP_MCP_TIMEOUT_S must be a positive number of seconds')
  process.exit(1)
}

class ToolError extends Error {}

function isInside(abs) {
  const root = WORKSPACE.endsWith(path.sep) ? WORKSPACE : WORKSPACE + path.sep
  return abs === WORKSPACE || abs.startsWith(root)
}

/** Resolve a caller path and refuse it unless the real path stays in the workspace. */
function insideWorkspace(rel, what = 'path') {
  if (typeof rel !== 'string' || rel.length === 0) throw new ToolError(`${what} is required`)
  if (rel.includes('\0')) throw new ToolError(`${what} must not contain NUL`)
  const abs = path.resolve(WORKSPACE, rel)
  let cursor = abs
  const missing = []
  while (!existsSync(cursor)) {
    const parent = path.dirname(cursor)
    if (parent === cursor) throw new ToolError(`${what} is not inside the workspace`)
    missing.push(path.basename(cursor))
    cursor = parent
  }
  let real
  try {
    real = realpathSync(cursor)
  } catch (error) {
    throw new ToolError(`${what} could not be resolved: ${error.message}`)
  }
  const full = missing.length === 0 ? real : path.join(real, ...missing.reverse())
  if (!isInside(full)) {
    throw new ToolError(`${what} must stay inside the workspace (${WORKSPACE}): got ${rel}`)
  }
  return full
}

function inputFile(rel, what) {
  const abs = insideWorkspace(rel, what)
  let st
  try {
    st = statSync(abs)
  } catch {
    throw new ToolError(`${what} does not exist: ${rel}`)
  }
  if (!st.isFile()) throw new ToolError(`${what} is not a file: ${rel}`)
  return abs
}

function prepareOutput(rel, what) {
  const abs = insideWorkspace(rel, what)
  const parent = path.dirname(abs)
  if (!existsSync(parent)) mkdirSync(parent, { recursive: true })
  const realParent = realpathSync(parent)
  if (!isInside(realParent)) throw new ToolError(`${what} parent is outside the workspace`)
  return abs
}

function only(args, keys) {
  for (const key of Object.keys(args)) {
    if (!keys.includes(key)) throw new ToolError(`unknown argument ${key}`)
  }
}

function reqString(args, key) {
  if (typeof args[key] !== 'string' || args[key].length === 0) throw new ToolError(`${key} is required`)
  return args[key]
}

function optString(args, key) {
  if (args[key] === undefined) return undefined
  if (typeof args[key] !== 'string' || args[key].length === 0) throw new ToolError(`${key} must be a non-empty string`)
  return args[key]
}

function optBool(args, key) {
  if (args[key] === undefined) return undefined
  if (typeof args[key] !== 'boolean') throw new ToolError(`${key} must be a boolean`)
  return args[key]
}

function optEnum(args, key, values) {
  if (args[key] === undefined) return undefined
  if (!values.includes(args[key])) throw new ToolError(`${key} must be one of ${values.join(', ')}`)
  return args[key]
}

function finite(value, what) {
  if (typeof value !== 'number' || !Number.isFinite(value)) throw new ToolError(`${what} must be a finite number`)
  return value
}

function numbers(value, n, what) {
  if (!Array.isArray(value) || value.length !== n) throw new ToolError(`${what} must be ${n} numbers`)
  return value.map((item, i) => finite(item, `${what}[${i}]`)).join(',')
}

function integers(value, n, what, min = 1) {
  if (!Array.isArray(value) || value.length !== n) throw new ToolError(`${what} must be ${n} integers`)
  return value.map((item, i) => {
    if (!Number.isInteger(item) || item < min) throw new ToolError(`${what}[${i}] must be an integer >= ${min}`)
    return String(item)
  }).join(',')
}

function optNumbers(args, key, n) {
  if (args[key] === undefined) return undefined
  return numbers(args[key], n, key)
}

function flag(argv, name, value) {
  if (value !== undefined) argv.push(name, String(value))
}

function tail(text, lines) {
  const all = text.split(/\r?\n/)
  if (all.length <= lines) return text.trimEnd()
  return ['… ' + (all.length - lines) + ' earlier line(s) omitted', ...all.slice(-lines)].join('\n').trimEnd()
}

function simrepBin() {
  if (!SIMREP_BIN) {
    throw new ToolError(
      'SIMREP_BIN is not set and target/release/simrep was not found next to mcp/. Build with `cargo build --release -p simrep-cli` and set SIMREP_BIN to that executable.',
    )
  }
  if (!existsSync(SIMREP_BIN)) throw new ToolError(`SIMREP_BIN does not exist: ${SIMREP_BIN}`)
  return SIMREP_BIN
}

/**
 * Run simrep --json with an argument array. No shell.
 * stderr is kept so a failure can put it on the error object.
 */
function runSimrep(args) {
  const bin = simrepBin()
  const argv = ['--json', ...args]
  return new Promise((resolve) => {
    const child = spawn(bin, argv, {
      cwd: WORKSPACE,
      shell: false,
      windowsHide: true,
      stdio: ['ignore', 'pipe', 'pipe'],
      // Own process group so a timeout can kill the binary and anything it spawned.
      detached: process.platform !== 'win32',
    })
    children.add(child)
    child.on('close', () => children.delete(child))
    let out = ''
    let err = ''
    let killed = false
    let truncated = false
    const killTree = () => {
      if (process.platform !== 'win32') {
        try {
          process.kill(-child.pid, 'SIGKILL')
          return
        } catch {
          /* already gone, or not a group leader */
        }
      }
      child.kill('SIGKILL')
    }
    child.stdout.on('data', (chunk) => {
      out += chunk
      if (out.length > STDOUT_CAP) {
        truncated = true
        out = out.slice(0, STDOUT_CAP)
        killTree()
      }
    })
    child.stderr.on('data', (chunk) => {
      err += chunk
      if (err.length > STDERR_CAP) err = err.slice(-STDERR_CAP)
    })
    const timer = setTimeout(() => {
      killed = true
      killTree()
    }, TIMEOUT_S * 1000)
    child.on('error', (error) => {
      clearTimeout(timer)
      resolve({ code: null, out, err: `${err}\n${error.message}`, killed, truncated, argv })
    })
    child.on('close', (code) => {
      clearTimeout(timer)
      resolve({ code, out, err, killed, truncated, argv })
    })
  })
}

function parseCliJson(text) {
  const trimmed = text.trim()
  if (!trimmed) return null
  try {
    return JSON.parse(trimmed)
  } catch {
    const lines = trimmed.split(/\r?\n/).filter(Boolean)
    try {
      return JSON.parse(lines[lines.length - 1])
    } catch {
      return null
    }
  }
}

function errorResult(error) {
  return {
    text: JSON.stringify({ schema_version: 1, error }, null, 2),
    isError: true,
  }
}

function cliResult(run) {
  const stderr = tail(run.err, 80)
  if (run.killed) {
    return errorResult({
      message: `simrep exceeded ${TIMEOUT_S}s and was killed; raise SIMREP_MCP_TIMEOUT_S`,
      kind: 'timeout',
      stderr,
      argv: run.argv,
    })
  }
  if (run.truncated) {
    return errorResult({
      message: 'simrep stdout exceeded the capture cap and was killed',
      kind: 'truncated',
      stderr,
      argv: run.argv,
    })
  }
  const parsed = parseCliJson(run.out)
  if (!parsed || typeof parsed !== 'object') {
    return errorResult({
      message: `simrep exited ${run.code === null ? 'without a status' : run.code} without a JSON summary`,
      kind: 'command_failed',
      stderr,
      stdout: run.out.trim().slice(0, 2000),
      argv: run.argv,
    })
  }
  if (run.code !== 0 || parsed.error) {
    const error = parsed.error && typeof parsed.error === 'object'
      ? { ...parsed.error }
      : { message: `simrep exited ${run.code}`, kind: 'command_failed' }
    if (stderr) error.stderr = stderr
    error.argv = run.argv
    return {
      text: JSON.stringify({ ...parsed, error }, null, 2),
      isError: true,
    }
  }
  return { text: JSON.stringify(parsed, null, 2), isError: false }
}

async function simrep(args) {
  return cliResult(await runSimrep(args))
}

const PAIR = /^(\d+)=([+-]?(?:\d+\.?\d*|\.\d+)(?:[eE][+-]?\d+)?)$/

function voltageArg(value) {
  const pairs = []
  const pushPair = (text) => {
    const match = String(text).trim().match(PAIR)
    if (!match) throw new ToolError(`voltage must look like 1=0 or 2=-110, got ${text}`)
    pairs.push(`${match[1]}=${match[2]}`)
  }
  if (typeof value === 'string') {
    for (const part of value.split(',')) pushPair(part)
  } else if (Array.isArray(value)) {
    for (const item of value) {
      if (typeof item === 'string') pushPair(item)
      else if (Array.isArray(item) && item.length === 2) pushPair(`${item[0]}=${item[1]}`)
      else if (item && typeof item === 'object') {
        const n = item.electrode ?? item.number
        const v = item.volts ?? item.voltage
        pushPair(`${n}=${v}`)
      } else {
        throw new ToolError('voltage entries must be "N=V", [N, V], or {electrode, volts}')
      }
    }
  } else {
    throw new ToolError('voltage must be a string or an array')
  }
  if (pairs.length === 0) throw new ToolError('voltage is empty')
  return pairs.join(',')
}

function assignmentList(value, flagName, pattern, what) {
  if (!Array.isArray(value) || value.length === 0) throw new ToolError(`${what} must be a non-empty array of strings`)
  const out = []
  for (const item of value) {
    if (typeof item !== 'string' || !pattern.test(item)) throw new ToolError(`${what} entry must look like ${flagName}, got ${item}`)
    out.push(item)
  }
  return out
}

function writeBeam(beam) {
  if (!beam || typeof beam !== 'object' || Array.isArray(beam)) throw new ToolError('beam must be an object')
  const allowed = ['count', 'mass_amu', 'charge_e', 'energy_ev', 'position_mm', 'azimuth_deg', 'elevation_deg', 'direction', 'line_axis', 'line_mm', 'birth_us', 'ion_file']
  only(beam, allowed)
  if (!Number.isInteger(beam.count) || beam.count < 1 || beam.count > 1_000_000) {
    throw new ToolError('beam.count must be an integer from 1 to 1000000')
  }
  const mass = finite(beam.mass_amu, 'beam.mass_amu')
  const charge = beam.charge_e === undefined ? 1 : finite(beam.charge_e, 'beam.charge_e')
  const energy = finite(beam.energy_ev, 'beam.energy_ev')
  if (!Array.isArray(beam.position_mm)) throw new ToolError('beam.position_mm must be [x, y, z] in mm')
  const position = beam.position_mm.map((v, i) => finite(v, `beam.position_mm[${i}]`))
  if (position.length !== 3) throw new ToolError('beam.position_mm must be [x, y, z] in mm')
  let azimuth = beam.azimuth_deg === undefined ? 0 : finite(beam.azimuth_deg, 'beam.azimuth_deg')
  let elevation = beam.elevation_deg === undefined ? 0 : finite(beam.elevation_deg, 'beam.elevation_deg')
  if (beam.direction !== undefined) {
    if (beam.azimuth_deg !== undefined || beam.elevation_deg !== undefined) {
      throw new ToolError('beam.direction cannot be combined with azimuth_deg or elevation_deg')
    }
    const dir = beam.direction
    if (!Array.isArray(dir) || dir.length !== 3) throw new ToolError('beam.direction must be [dx, dy, dz]')
    const [x, y, z] = dir.map((v, i) => finite(v, `beam.direction[${i}]`))
    const n = Math.hypot(x, y, z)
    if (!(n > 0)) throw new ToolError('beam.direction must be non-zero')
    elevation = Math.asin(Math.min(1, Math.max(-1, y / n))) * 180 / Math.PI
    azimuth = Math.atan2(-z / n, x / n) * 180 / Math.PI
  }
  const axisName = beam.line_axis ?? 'y'
  if (!['x', 'y', 'z'].includes(axisName)) throw new ToolError('beam.line_axis must be x, y, or z')
  const line = beam.line_mm === undefined ? 0 : finite(beam.line_mm, 'beam.line_mm')
  if (line < 0) throw new ToolError('beam.line_mm must be >= 0')
  const birth = beam.birth_us === undefined ? 0 : finite(beam.birth_us, 'beam.birth_us')
  const ionFile = prepareOutput(reqString(beam, 'ion_file'), 'beam.ion_file')
  const axis = { x: 0, y: 1, z: 2 }[axisName]
  const lines = []
  for (let i = 0; i < beam.count; i++) {
    const t = beam.count === 1 ? 0.5 : i / (beam.count - 1)
    const pos = position.slice()
    pos[axis] += (t - 0.5) * line
    lines.push([birth, mass, charge, pos[0], pos[1], pos[2], azimuth, elevation, energy].join(','))
  }
  writeFileSync(ionFile, lines.join('\n') + '\n')
  return ionFile
}

function deviceFlags(args, devices) {
  const argv = []
  const device = optEnum(args, 'device', devices)
  const backend = optEnum(args, 'backend', ['auto', 'vulkan', 'dx12'])
  flag(argv, '--device', device)
  flag(argv, '--backend', backend)
  if (args.tolerance !== undefined) flag(argv, '--tolerance', finite(args.tolerance, 'tolerance'))
  if (args.max_cycles !== undefined) {
    if (!Number.isInteger(args.max_cycles) || args.max_cycles < 0) throw new ToolError('max_cycles must be an integer >= 0')
    flag(argv, '--max-cycles', args.max_cycles)
  }
  return argv
}

const UNITS = ['um', 'micron', 'mm', 'millimeter', 'cm', 'centimeter', 'm', 'meter', 'in', 'inch', 'ft', 'foot']

const TOOLS = [
  {
    name: 'gpu_info',
    title: 'Report the GPU adapter',
    description:
      'Report the adapter simrep would use. JSON keys: adapter_name, adapter_type, backend, driver, driver_info, f64_support, software_adapter, schema_version, simrep_version. On WSL or CI there is often no hardware Vulkan/DX12 device; software_adapter is then true. Later tools with device "gpu" refuse a software adapter — use device "cpu". Native f64 solves need f64_support; otherwise GPU corrections are f32 with an f64 residual (precision mixed-f32-f64). A missing adapter is an error JSON object, not a crash.',
    inputSchema: {
      type: 'object',
      properties: {
        backend: { type: 'string', enum: ['auto', 'vulkan', 'dx12'], description: 'Native compute backend. Default auto.' },
      },
      additionalProperties: false,
    },
    readOnly: true,
    async run(args) {
      only(args, ['backend'])
      const argv = ['gpu-info']
      flag(argv, '--backend', optEnum(args, 'backend', ['auto', 'vulkan', 'dx12']))
      return simrep(argv)
    },
  },
  {
    name: 'pa_info',
    title: 'Inspect a potential array',
    description:
      'Inspect a SIMION PA, PA#, PA0, or numbered basis file. spacing_mm is millimetres; potentials are volts. Electrode numbers in a .pa# are 1..1000, not stored volts. PA files do not store an origin. Returns symmetry (planar|cylindrical), field_type (electrostatic|magnetic), dims, spacing_mm, mirrors, anti_mirrors, refinable, potential_min/max/mean, and adjustable_electrodes (null unless the file is a PA0). A non-refinable file is a fixed field map; refine the source .pa# instead.',
    inputSchema: {
      type: 'object',
      properties: {
        file: { type: 'string', description: 'Workspace-relative PA, PA#, PA0, or .paN path.' },
      },
      required: ['file'],
      additionalProperties: false,
    },
    readOnly: true,
    async run(args) {
      only(args, ['file'])
      return simrep(['pa-info', inputFile(reqString(args, 'file'), 'file')])
    },
  },
  {
    name: 'gem2pa',
    title: 'Compile GEM geometry to a PA#',
    description:
      'Compile a SIMION GEM file to an unrefined PA#. Electrode nodes store electrode numbers 1..1000, not volts. output defaults to the input stem with extension .pa#. Returns output_paths, dims [nx,ny,nz], spacing_mm (millimetres), electrode_numbers, compute_ms, schema_version, simrep_version. Paths outside the workspace are refused.',
    inputSchema: {
      type: 'object',
      properties: {
        input: { type: 'string', description: 'Workspace-relative .gem file.' },
        output: { type: 'string', description: 'Workspace-relative .pa# to write. Default: input stem with .pa#.' },
      },
      required: ['input'],
      additionalProperties: false,
    },
    async run(args) {
      only(args, ['input', 'output'])
      const argv = ['gem2pa', inputFile(reqString(args, 'input'), 'input')]
      if (args.output !== undefined) argv.push(prepareOutput(reqString(args, 'output'), 'output'))
      return simrep(argv)
    },
  },
  {
    name: 'mesh_import',
    title: 'Voxelize a triangle mesh to a PA#',
    description:
      'Import STL, OBJ, PLY, 3MF, glTF, or GLB as grouped electrodes in a PA#. grid is nx,ny,nz node counts (at least 2 on each axis). spacing and origin are millimetres; the PA file does not store origin, so pass the same origin to fly and export. electrodes is one number per part (default 1,2,... in reported order). units: um, mm, cm, m, in, ft. Returns output_paths, dims, spacing_mm, electrode_numbers.',
    inputSchema: {
      type: 'object',
      properties: {
        input: { type: 'string' },
        output: { type: 'string', description: 'Workspace-relative .pa# output.' },
        grid: { type: 'array', items: { type: 'integer' }, minItems: 3, maxItems: 3, description: 'Node counts nx,ny,nz. Each axis at least 2.' },
        spacing: { type: 'array', items: { type: 'number' }, minItems: 3, maxItems: 3, description: 'Node spacing sx,sy,sz in mm. Default 1,1,1.' },
        origin: { type: 'array', items: { type: 'number' }, minItems: 3, maxItems: 3, description: 'Position of grid node (0,0,0) in mm. Default 0,0,0.' },
        units: { type: 'string', enum: UNITS },
        electrodes: { type: 'array', items: { type: 'integer' }, description: 'One electrode number per part.' },
        repair: { type: 'boolean' },
        allow_defects: { type: 'boolean' },
      },
      required: ['input', 'output', 'grid'],
      additionalProperties: false,
    },
    async run(args) {
      only(args, ['input', 'output', 'grid', 'spacing', 'origin', 'units', 'electrodes', 'repair', 'allow_defects'])
      const argv = [
        'mesh-import',
        inputFile(reqString(args, 'input'), 'input'),
        prepareOutput(reqString(args, 'output'), 'output'),
        '--grid', integers(args.grid, 3, 'grid', 2),
      ]
      if (args.spacing !== undefined) argv.push('--spacing', numbers(args.spacing, 3, 'spacing'))
      if (args.origin !== undefined) argv.push('--origin', numbers(args.origin, 3, 'origin'))
      flag(argv, '--units', optEnum(args, 'units', UNITS))
      if (args.electrodes !== undefined) {
        if (!Array.isArray(args.electrodes) || args.electrodes.some((n) => !Number.isInteger(n) || n < 1 || n > 1000)) {
          throw new ToolError('electrodes must be integers 1..1000')
        }
        argv.push('--electrodes', args.electrodes.join(','))
      }
      if (optBool(args, 'repair')) argv.push('--repair')
      if (optBool(args, 'allow_defects')) argv.push('--allow-defects')
      return simrep(argv)
    },
  },
  {
    name: 'refine',
    title: 'Solve a PA or PA#',
    description:
      'Solve a .pa or .pa# (those extensions are required). A .pa# writes a fast-adjust family beside it: .pa0 plus .pa1..paN, and .pa_ when a scalable basis exists. device auto stays on CPU unless a measured dielectric crossover selects GPU; cpu is the safe choice when GPU support is uncertain; gpu refuses a software adapter ("--device gpu requires a hardware GPU ... use --device cpu"). tolerance is the relative infinity-norm of the physical f64 Laplace residual (default 1e-10). If refinement does not converge, raise max_cycles or loosen tolerance, and check floating or overlapping electrodes. Nonlinear permittivity cannot use PA# fast-adjust bases. Returns requested, executed {device, precision, backend}, reason, fallback_reason, cycles, final_residual, tolerance, converged, compute_ms, prep_ms, output_paths. Compare requested with executed.',
    inputSchema: {
      type: 'object',
      properties: {
        file: { type: 'string', description: 'Workspace-relative .pa or .pa#.' },
        device: { type: 'string', enum: ['auto', 'cpu', 'gpu'] },
        backend: { type: 'string', enum: ['auto', 'vulkan', 'dx12'] },
        tolerance: { type: 'number', exclusiveMinimum: 0 },
        max_cycles: { type: 'integer', minimum: 0 },
        permittivity: { type: 'string', description: 'Workspace-relative permittivity PA or material JSON.' },
        magnetic_material: { type: 'string', description: 'Workspace-relative magnetic material JSON.' },
      },
      required: ['file'],
      additionalProperties: false,
    },
    async run(args) {
      only(args, ['file', 'device', 'backend', 'tolerance', 'max_cycles', 'permittivity', 'magnetic_material'])
      const argv = ['refine', inputFile(reqString(args, 'file'), 'file'), ...deviceFlags(args, ['auto', 'cpu', 'gpu'])]
      if (args.permittivity !== undefined) argv.push('--permittivity', inputFile(reqString(args, 'permittivity'), 'permittivity'))
      if (args.magnetic_material !== undefined) argv.push('--magnetic-material', inputFile(reqString(args, 'magnetic_material'), 'magnetic_material'))
      return simrep(argv)
    },
  },
  {
    name: 'refine_geometry',
    title: 'Refine exact electrode surfaces',
    description:
      'Refine GEM, mesh, or STEP/IGES CAD at the continuous surface. Writes out.pa0, out.paN, out.cuts.json, and out.pa#. Every part needs exactly one electrode assignment part=number:voltage (GEM source id, for example "1=1:0"). Numbers are 1..1000 and are not volts; the voltage after the colon is volts. device is cpu (default) or gpu — there is no auto. gpu refuses a software adapter. STEP/IGES needs a simrep built with --features cad; otherwise the error says so. If an electrode has no inside/on-surface node, use a finer grid or a thicker part. grid nx,ny must be >= 2 and nz >= 1; spacing and origin are millimetres. Returns the same solve summary as refine.',
    inputSchema: {
      type: 'object',
      properties: {
        input: { type: 'string', description: 'Workspace-relative .gem, mesh, or STEP/IGES file.' },
        electrode: {
          type: 'array',
          items: { type: 'string' },
          minItems: 1,
          description: 'Assignments part=number:voltage, one per part. GEM: source id, e.g. 1=1:0.',
        },
        out: { type: 'string', description: 'Workspace-relative output stem (no extension).' },
        device: { type: 'string', enum: ['cpu', 'gpu'] },
        backend: { type: 'string', enum: ['auto', 'vulkan', 'dx12'] },
        grid: { type: 'array', items: { type: 'integer' }, minItems: 3, maxItems: 3 },
        spacing: { type: 'array', items: { type: 'number' }, minItems: 3, maxItems: 3, description: 'sx,sy,sz in mm.' },
        origin: { type: 'array', items: { type: 'number' }, minItems: 3, maxItems: 3, description: 'First node in mm.' },
        tolerance: { type: 'number', exclusiveMinimum: 0 },
        max_cycles: { type: 'integer', minimum: 0 },
        units: { type: 'string', enum: UNITS },
        repair: { type: 'boolean' },
        cad_mode: { type: 'string', enum: ['auto', 'exact', 'tessellated'] },
        cad_tolerance: { type: 'number', exclusiveMinimum: 0, description: 'Chordal deflection in mm.' },
      },
      required: ['input', 'electrode', 'out'],
      additionalProperties: false,
    },
    async run(args) {
      only(args, ['input', 'electrode', 'out', 'device', 'backend', 'grid', 'spacing', 'origin', 'tolerance', 'max_cycles', 'units', 'repair', 'cad_mode', 'cad_tolerance'])
      const argv = [
        'refine-geometry',
        inputFile(reqString(args, 'input'), 'input'),
        ...deviceFlags(args, ['cpu', 'gpu']),
        '--out', prepareOutput(reqString(args, 'out'), 'out'),
      ]
      for (const item of assignmentList(args.electrode, 'part=number:voltage', /^[^=]+=\d+:[+-]?(?:\d+\.?\d*|\.\d+)(?:[eE][+-]?\d+)?$/, 'electrode')) {
        argv.push('--electrode', item)
      }
      if (args.grid !== undefined) argv.push('--grid', integers(args.grid, 3, 'grid', 1))
      if (args.spacing !== undefined) argv.push('--spacing', numbers(args.spacing, 3, 'spacing'))
      if (args.origin !== undefined) argv.push('--origin', numbers(args.origin, 3, 'origin'))
      flag(argv, '--units', optEnum(args, 'units', UNITS))
      if (optBool(args, 'repair')) argv.push('--repair')
      flag(argv, '--cad-mode', optEnum(args, 'cad_mode', ['auto', 'exact', 'tessellated']))
      if (args.cad_tolerance !== undefined) flag(argv, '--cad-tolerance', finite(args.cad_tolerance, 'cad_tolerance'))
      return simrep(argv)
    },
  },
  {
    name: 'cad_measure',
    title: 'Measure CAD face-set gaps',
    description:
      'Measure complete placed STEP/IGES body face sets, or explicit :faces=0,1 subsets, in mm. Select parts by full XCAF path or zero-based body index. Choose gap (two selectors) or all_pairs. Returns exact minimum distances, closest points and face indices, touching, and an independent tessellation cross-check; disagreement beyond cad_tolerance fails. touching means distance <= touch_tolerance, not proof of electrical contact. Requires simrep built with --features cad.',
    inputSchema: {
      type: 'object',
      properties: {
        file: { type: 'string', description: 'Workspace-relative STEP or IGES file.' },
        gap: { type: 'array', items: { type: 'string' }, minItems: 2, maxItems: 2, description: 'Two part selectors, optionally with :faces=0,1.' },
        all_pairs: { type: 'boolean', description: 'Return all unique pairs and symmetric distance/touch matrices.' },
        cad_tolerance: { type: 'number', exclusiveMinimum: 0, description: 'Absolute allowed cross-check discrepancy in mm; default 0.01.' },
        touch_tolerance: { type: 'number', minimum: 0, description: 'Touch threshold in mm; default 1e-7.' },
      },
      required: ['file'],
      oneOf: [
        { required: ['gap'], properties: { all_pairs: { const: false } } },
        { required: ['all_pairs'], properties: { all_pairs: { const: true } }, not: { required: ['gap'] } },
      ],
      additionalProperties: false,
    },
    readOnly: true,
    async run(args) {
      only(args, ['file', 'gap', 'all_pairs', 'cad_tolerance', 'touch_tolerance'])
      const allPairs = optBool(args, 'all_pairs') ?? false
      if (allPairs === (args.gap !== undefined)) throw new ToolError('provide gap or all_pairs, not both')
      const argv = ['cad-measure', inputFile(reqString(args, 'file'), 'file')]
      if (allPairs) {
        argv.push('--all-pairs')
      } else {
        if (!Array.isArray(args.gap) || args.gap.length !== 2 || args.gap.some((s) => typeof s !== 'string' || s.length === 0)) {
          throw new ToolError('gap must contain exactly two nonempty selectors')
        }
        argv.push('--gap', ...args.gap)
      }
      for (const key of ['cad_tolerance', 'touch_tolerance']) {
        if (args[key] === undefined) continue
        const value = finite(args[key], key)
        if (value < 0 || (key === 'cad_tolerance' && value === 0)) throw new ToolError(`${key} is out of range`)
        flag(argv, '--' + key.replaceAll('_', '-'), value)
      }
      return simrep(argv)
    },
  },
  {
    name: 'fly',
    title: 'Fly ions through a field',
    description:
      'Fly ions through a PA or PA0 and return the --json summary. Provide particles (an .ion CSV or .fly2 file) or beam (a generated .ion written inside the workspace), not both. ION columns are birth_us, mass_amu, charge_e, x_mm, y_mm, z_mm, azimuth_deg, elevation_deg, energy_ev. Azimuth 0 and elevation 0 fly toward +X; SIMION azimuth +90 points toward -Z. voltage is fast-adjust overrides such as ["1=0","2=110"] in volts and needs a PA0 family via pa ("electrode voltages require --pa"). plane is the detector nx,ny,nz,d in workbench millimetres (nx*x+ny*y+nz*z = d). bounds is xmin,ymin,zmin,xmax,ymax,zmax in millimetres and permits field-free drift outside the PA. bfield is uniform Bx,By,Bz in gauss. collision is hs1 or sds; pressure is pascals, temperature kelvin, gas mass amu. SDS requires mbmr, a path to SIMION mbmr.dat, which is not bundled. device auto stays on CPU for small beams and for any magnetic flight; gpu refuses a software adapter. Exact-surface (--cuts) flying, direct Coulomb, and beam mode are CPU-only: use device cpu, or no_cuts for staircase GPU flying. output is the result CSV. Ions that only hit the time or step limit are an error unless allow_time_limit is set, a plane is reached, or bounds and duration_us (default 100) are extended. Returns ion_count, arrivals (detector-plane hits), splat_counts, arrival_tof_us {mean, std, min, max}, compute_ms, prep_ms, requested, executed (adds surface_mode), reason, fallback_reason, output_paths.',
    inputSchema: {
      type: 'object',
      properties: {
        pa: { type: 'string', description: 'Workspace-relative PA or PA0. Required for voltages and for a solved field.' },
        particles: { type: 'string', description: 'Workspace-relative .ion or .fly2 file. Omit when beam is set.' },
        beam: {
          type: 'object',
          description: 'Generate an ION file inside the workspace, then fly it. Azimuth 0 elevation 0 is +X.',
          properties: {
            count: { type: 'integer', minimum: 1, maximum: 1000000 },
            mass_amu: { type: 'number' },
            charge_e: { type: 'number' },
            energy_ev: { type: 'number' },
            position_mm: { type: 'array', items: { type: 'number' }, minItems: 3, maxItems: 3 },
            azimuth_deg: { type: 'number' },
            elevation_deg: { type: 'number' },
            direction: { type: 'array', items: { type: 'number' }, minItems: 3, maxItems: 3, description: 'Alternate to azimuth/elevation. Converted with the SIMION convention.' },
            line_axis: { type: 'string', enum: ['x', 'y', 'z'] },
            line_mm: { type: 'number', minimum: 0, description: 'Spread positions along line_axis, centered on position_mm. Default 0.' },
            birth_us: { type: 'number' },
            ion_file: { type: 'string', description: 'Workspace-relative .ion path to write.' },
          },
          required: ['count', 'mass_amu', 'energy_ev', 'position_mm', 'ion_file'],
          additionalProperties: false,
        },
        voltage: { description: 'Fast-adjust volts: "1=0,2=110", ["1=0","2=110"], or [{electrode, volts}].' },
        voltage_expr: { type: 'array', items: { type: 'string' }, description: 'Per-electrode expressions "N=EXPR" at time t in microseconds.' },
        param: { type: 'array', items: { type: 'string' }, description: 'Named parameters "name=value".' },
        run_file: { type: 'string' },
        plane: { type: 'array', items: { type: 'number' }, minItems: 4, maxItems: 4, description: 'Detector nx,ny,nz,d in mm.' },
        bounds: { type: 'array', items: { type: 'number' }, minItems: 6, maxItems: 6, description: 'xmin,ymin,zmin,xmax,ymax,zmax in mm.' },
        device: { type: 'string', enum: ['auto', 'cpu', 'gpu'] },
        backend: { type: 'string', enum: ['auto', 'vulkan', 'dx12'] },
        bfield: { type: 'array', items: { type: 'number' }, minItems: 3, maxItems: 3, description: 'Uniform Bx,By,Bz in gauss.' },
        magnetic_pa: { type: 'array', items: { type: 'string' }, description: 'Workspace-relative magnetic PA files.' },
        bfield_coil: { type: 'string', description: 'Workspace-relative coil JSON.' },
        collision: { type: 'string', enum: ['hs1', 'sds'] },
        seed: { type: 'integer', minimum: 0 },
        pressure_pa: { type: 'number' },
        temperature_k: { type: 'number' },
        gas_mass_amu: { type: 'number' },
        mbmr: { type: 'string', description: 'Required for collision sds. Path to SIMION mbmr.dat.' },
        output: { type: 'string', description: 'Workspace-relative result CSV.' },
        duration_us: { type: 'number', exclusiveMinimum: 0 },
        origin: { type: 'array', items: { type: 'number' }, minItems: 3, maxItems: 3, description: 'PA origin in workbench mm.' },
        tolerance: { type: 'number', exclusiveMinimum: 0 },
        integrator: { type: 'string', enum: ['rk45', 'boris'] },
        allow_time_limit: { type: 'boolean' },
        no_cuts: { type: 'boolean', description: 'Staircase collisions. Required for GPU flying when a .cuts.json sibling exists.' },
      },
      additionalProperties: false,
    },
    async run(args) {
      only(args, ['pa', 'particles', 'beam', 'voltage', 'voltage_expr', 'param', 'run_file', 'plane', 'bounds', 'device', 'backend', 'bfield', 'magnetic_pa', 'bfield_coil', 'collision', 'seed', 'pressure_pa', 'temperature_k', 'gas_mass_amu', 'mbmr', 'output', 'duration_us', 'origin', 'tolerance', 'integrator', 'allow_time_limit', 'no_cuts'])
      if ((args.particles === undefined) === (args.beam === undefined)) {
        throw new ToolError('provide exactly one of particles or beam')
      }
      const particles = args.beam ? writeBeam(args.beam) : inputFile(reqString(args, 'particles'), 'particles')
      const argv = ['fly', '--particles', particles]
      if (args.pa !== undefined) argv.push('--pa', inputFile(reqString(args, 'pa'), 'pa'))
      flag(argv, '--device', optEnum(args, 'device', ['auto', 'cpu', 'gpu']))
      flag(argv, '--backend', optEnum(args, 'backend', ['auto', 'vulkan', 'dx12']))
      if (args.voltage !== undefined) argv.push('--voltage', voltageArg(args.voltage))
      if (args.voltage_expr !== undefined) {
        for (const item of assignmentList(args.voltage_expr, 'N=EXPR', /^\d+=\S/, 'voltage_expr')) argv.push('--voltage-expr', item)
      }
      if (args.param !== undefined) {
        for (const item of assignmentList(args.param, 'name=value', /^[A-Za-z_][A-Za-z0-9_]*=[+-]?(?:\d+\.?\d*|\.\d+)(?:[eE][+-]?\d+)?$/, 'param')) {
          argv.push('--param', item)
        }
      }
      if (args.run_file !== undefined) argv.push('--run-file', inputFile(reqString(args, 'run_file'), 'run_file'))
      if (args.plane !== undefined) argv.push('--plane', numbers(args.plane, 4, 'plane'))
      if (args.bounds !== undefined) argv.push('--bounds', numbers(args.bounds, 6, 'bounds'))
      if (args.bfield !== undefined) argv.push('--bfield', numbers(args.bfield, 3, 'bfield'))
      if (args.magnetic_pa !== undefined) {
        if (!Array.isArray(args.magnetic_pa)) throw new ToolError('magnetic_pa must be an array of paths')
        for (const item of args.magnetic_pa) argv.push('--magnetic-pa', inputFile(item, 'magnetic_pa'))
      }
      if (args.bfield_coil !== undefined) argv.push('--bfield-coil', inputFile(reqString(args, 'bfield_coil'), 'bfield_coil'))
      const collision = optEnum(args, 'collision', ['hs1', 'sds'])
      flag(argv, '--collision', collision)
      if (collision === 'sds' && args.mbmr === undefined) {
        throw new ToolError('SDS requires mbmr, a path to SIMION mbmr.dat; it is not bundled')
      }
      if (args.seed !== undefined) {
        if (!Number.isInteger(args.seed) || args.seed < 0) throw new ToolError('seed must be an integer >= 0')
        flag(argv, '--seed', args.seed)
      }
      if (args.pressure_pa !== undefined) flag(argv, '--pressure-pa', finite(args.pressure_pa, 'pressure_pa'))
      if (args.temperature_k !== undefined) flag(argv, '--temperature-k', finite(args.temperature_k, 'temperature_k'))
      if (args.gas_mass_amu !== undefined) flag(argv, '--gas-mass-amu', finite(args.gas_mass_amu, 'gas_mass_amu'))
      if (args.mbmr !== undefined) argv.push('--mbmr', inputFile(reqString(args, 'mbmr'), 'mbmr'))
      if (args.output !== undefined) argv.push('--output', prepareOutput(reqString(args, 'output'), 'output'))
      if (args.duration_us !== undefined) flag(argv, '--duration-us', finite(args.duration_us, 'duration_us'))
      if (args.origin !== undefined) argv.push('--origin', numbers(args.origin, 3, 'origin'))
      if (args.tolerance !== undefined) flag(argv, '--tolerance', finite(args.tolerance, 'tolerance'))
      flag(argv, '--integrator', optEnum(args, 'integrator', ['rk45', 'boris']))
      if (optBool(args, 'allow_time_limit')) argv.push('--allow-time-limit')
      if (optBool(args, 'no_cuts')) argv.push('--no-cuts')
      return simrep(argv)
    },
  },
  {
    name: 'export_geometry',
    title: 'Export electrode surfaces',
    description:
      'Export exposed electrode voxel faces of a PA to STL, OBJ, glTF, or GLB. electrode selects one electrode number or potential in volts; omit to export all. origin is the millimetre position of grid node (0,0,0). angular_segments and sweep_degrees revolve a cylindrical PA (sweep is from +Y toward +Z, degrees, default 360). extrusion_depth is the planar 2D extrusion depth in millimetres. Returns output_paths.',
    inputSchema: {
      type: 'object',
      properties: {
        input: { type: 'string' },
        output: { type: 'string' },
        electrode: { type: 'number', description: 'Electrode number or potential. Omit for all.' },
        origin: { type: 'array', items: { type: 'number' }, minItems: 3, maxItems: 3 },
        angular_segments: { type: 'integer', minimum: 1 },
        sweep_degrees: { type: 'number' },
        extrusion_depth: { type: 'number', description: 'Planar extrusion depth in mm.' },
      },
      required: ['input', 'output'],
      additionalProperties: false,
    },
    async run(args) {
      only(args, ['input', 'output', 'electrode', 'origin', 'angular_segments', 'sweep_degrees', 'extrusion_depth'])
      const argv = ['export', 'geometry', inputFile(reqString(args, 'input'), 'input'), prepareOutput(reqString(args, 'output'), 'output')]
      if (args.electrode !== undefined) flag(argv, '--electrode', finite(args.electrode, 'electrode'))
      pushSurface(argv, args)
      return simrep(argv)
    },
  },
  {
    name: 'export_grid',
    title: 'Export the potential grid',
    description:
      'Export potential, electrode mask, and electric field to VTI, or a revolved cylindrical VTS when revolve is true. no_field omits field vectors. Grid output must be .vti; revolved output must be .vts. origin is millimetres. Returns output_paths.',
    inputSchema: {
      type: 'object',
      properties: {
        input: { type: 'string' },
        output: { type: 'string' },
        origin: { type: 'array', items: { type: 'number' }, minItems: 3, maxItems: 3 },
        no_field: { type: 'boolean' },
        revolve: { type: 'boolean' },
        angular_segments: { type: 'integer', minimum: 1 },
        sweep_degrees: { type: 'number' },
      },
      required: ['input', 'output'],
      additionalProperties: false,
    },
    async run(args) {
      only(args, ['input', 'output', 'origin', 'no_field', 'revolve', 'angular_segments', 'sweep_degrees'])
      const argv = ['export', 'grid', inputFile(reqString(args, 'input'), 'input'), prepareOutput(reqString(args, 'output'), 'output')]
      if (args.origin !== undefined) argv.push('--origin', numbers(args.origin, 3, 'origin'))
      if (optBool(args, 'no_field')) argv.push('--no-field')
      if (optBool(args, 'revolve')) argv.push('--revolve')
      pushAngular(argv, args)
      return simrep(argv)
    },
  },
  {
    name: 'export_isosurface',
    title: 'Export a potential isosurface',
    description:
      'Export a potential isosurface (marching cubes, or a swept 2D contour) to a mesh. potential is volts. The output extension selects STL, OBJ, glTF, or GLB. origin is millimetres. Returns output_paths.',
    inputSchema: {
      type: 'object',
      properties: {
        input: { type: 'string' },
        output: { type: 'string' },
        potential: { type: 'number', description: 'Isosurface potential in volts.' },
        origin: { type: 'array', items: { type: 'number' }, minItems: 3, maxItems: 3 },
        angular_segments: { type: 'integer', minimum: 1 },
        sweep_degrees: { type: 'number' },
        extrusion_depth: { type: 'number' },
      },
      required: ['input', 'output', 'potential'],
      additionalProperties: false,
    },
    async run(args) {
      only(args, ['input', 'output', 'potential', 'origin', 'angular_segments', 'sweep_degrees', 'extrusion_depth'])
      const argv = [
        'export', 'isosurface',
        inputFile(reqString(args, 'input'), 'input'),
        prepareOutput(reqString(args, 'output'), 'output'),
        '--potential', String(finite(args.potential, 'potential')),
      ]
      pushSurface(argv, args)
      return simrep(argv)
    },
  },
  {
    name: 'export_trajectories',
    title: 'Export trajectory samples',
    description:
      'Convert a trajectory-sample CSV to VTP polylines or a normalized CSV. The input header must be trajectory_id,time_us,x_mm,y_mm,z_mm,vx_mm_per_us,vy_mm_per_us,vz_mm_per_us. This is not a fly result CSV; use read_csv_summary for fly --output files. Output extension must be .vtp or .csv. Returns output_paths.',
    inputSchema: {
      type: 'object',
      properties: {
        input: { type: 'string' },
        output: { type: 'string' },
      },
      required: ['input', 'output'],
      additionalProperties: false,
    },
    async run(args) {
      only(args, ['input', 'output'])
      return simrep([
        'export', 'trajectories',
        inputFile(reqString(args, 'input'), 'input'),
        prepareOutput(reqString(args, 'output'), 'output'),
      ])
    },
  },
  {
    name: 'read_csv_summary',
    title: 'Summarize a result CSV',
    description:
      'Summarize a workspace CSV, typically a fly --output file, without running simrep. Splits on commas (fly columns are unquoted). Returns row_count, columns, numeric min/max/mean/population-std, and reason_counts when a reason column exists. A short sample of the first rows is included. Files beyond max_rows are truncated and marked. Paths outside the workspace are refused.',
    inputSchema: {
      type: 'object',
      properties: {
        path: { type: 'string', description: 'Workspace-relative CSV.' },
        max_rows: { type: 'integer', minimum: 1, maximum: 5000000, description: 'Row cap. Default 2000000.' },
        sample_rows: { type: 'integer', minimum: 0, maximum: 20, description: 'Sample rows to return. Default 5.' },
      },
      required: ['path'],
      additionalProperties: false,
    },
    readOnly: true,
    async run(args) {
      only(args, ['path', 'max_rows', 'sample_rows'])
      const abs = inputFile(reqString(args, 'path'), 'path')
      const maxRows = args.max_rows ?? CSV_ROW_CAP
      const sampleN = args.sample_rows ?? 5
      if (!Number.isInteger(maxRows) || maxRows < 1 || maxRows > 5_000_000) throw new ToolError('max_rows must be an integer 1..5000000')
      if (!Number.isInteger(sampleN) || sampleN < 0 || sampleN > 20) throw new ToolError('sample_rows must be an integer 0..20')
      const summary = await summarizeCsv(abs, maxRows, sampleN)
      return { text: JSON.stringify(summary, null, 2), isError: false }
    },
  },
]

function pushAngular(argv, args) {
  if (args.angular_segments !== undefined) {
    if (!Number.isInteger(args.angular_segments) || args.angular_segments < 1) throw new ToolError('angular_segments must be an integer >= 1')
    flag(argv, '--angular-segments', args.angular_segments)
  }
  if (args.sweep_degrees !== undefined) flag(argv, '--sweep-degrees', finite(args.sweep_degrees, 'sweep_degrees'))
}

function pushSurface(argv, args) {
  if (args.origin !== undefined) argv.push('--origin', numbers(args.origin, 3, 'origin'))
  pushAngular(argv, args)
  if (args.extrusion_depth !== undefined) flag(argv, '--extrusion-depth', finite(args.extrusion_depth, 'extrusion_depth'))
}

function splitCsv(line) {
  return line.split(',').map((cell) => cell.trim())
}

async function summarizeCsv(abs, maxRows, sampleN) {
  const rl = readline.createInterface({ input: createReadStream(abs, { encoding: 'utf8' }), crlfDelay: Infinity })
  let columns = null
  let rows = 0
  let truncated = false
  const numeric = []
  const reasons = new Map()
  const sample = []
  let reasonIndex = -1
  for await (const line of rl) {
    if (!line.trim()) continue
    const cells = splitCsv(line)
    if (!columns) {
      columns = cells
      reasonIndex = columns.indexOf('reason')
      numeric.push(...columns.map(() => ({ count: 0, min: Infinity, max: -Infinity, sum: 0, sumsq: 0, numeric: true })))
      continue
    }
    if (rows >= maxRows) {
      truncated = true
      break
    }
    rows += 1
    if (sample.length < sampleN) sample.push(cells)
    cells.forEach((cell, i) => {
      const stat = numeric[i]
      if (!stat || cell === '') return
      const value = Number(cell)
      if (!Number.isFinite(value)) {
        stat.numeric = false
        return
      }
      stat.count += 1
      stat.min = Math.min(stat.min, value)
      stat.max = Math.max(stat.max, value)
      stat.sum += value
      stat.sumsq += value * value
    })
    if (reasonIndex >= 0 && cells[reasonIndex]) {
      reasons.set(cells[reasonIndex], (reasons.get(cells[reasonIndex]) ?? 0) + 1)
    }
  }
  rl.close()
  if (!columns) throw new ToolError('CSV is empty')
  const numericOut = {}
  columns.forEach((name, i) => {
    const stat = numeric[i]
    if (!stat.numeric || stat.count === 0) return
    const mean = stat.sum / stat.count
    numericOut[name] = {
      count: stat.count,
      min: stat.min,
      max: stat.max,
      mean,
      std: Math.sqrt(Math.max(0, stat.sumsq / stat.count - mean * mean)),
    }
  })
  const summary = {
    path: abs,
    row_count: rows,
    truncated,
    columns,
    numeric: numericOut,
    sample_rows: sample,
  }
  if (reasonIndex >= 0) summary.reason_counts = Object.fromEntries(reasons)
  return summary
}

const BY_NAME = new Map(TOOLS.map((tool) => [tool.name, tool]))

let writeChain = Promise.resolve()
function send(msg) {
  const line = JSON.stringify(msg) + '\n'
  writeChain = writeChain.then(() => new Promise((resolve, reject) => {
    process.stdout.write(line, (error) => (error ? reject(error) : resolve()))
  })).catch((error) => {
    console.error(`stdout write failed: ${error.message}`)
  })
}

function result(id, value) {
  send({ jsonrpc: '2.0', id, result: value })
}

function failure(id, code, message) {
  send({ jsonrpc: '2.0', id, error: { code, message } })
}

async function handle(msg) {
  const { id, method, params } = msg
  const isNotification = id === undefined || id === null
  switch (method) {
    case 'initialize':
      return result(id, {
        protocolVersion: PROTOCOL_VERSION,
        capabilities: { tools: { listChanged: false } },
        serverInfo: SERVER,
        instructions:
          `SimRep ion-optics CLI. Workspace: ${WORKSPACE}. Binary: ${SIMREP_BIN ?? 'SIMREP_BIN is not set'}.\n` +
          'Units: mm, microseconds, volts, gauss, amu, elementary charge, eV, pascals. Electrode numbers are 1..1000, not volts. PA files do not store an origin.\n' +
          'Use device "cpu" unless a hardware GPU is known. device "gpu" refuses a software adapter. Magnetic flight, exact-surface collisions, and direct Coulomb are CPU-only.\n' +
          'Every path is resolved inside the workspace and refused otherwise. There is no shell. Tool results are the CLI JSON summary (schema_version 1); failures include stderr on error.stderr.',
      })
    case 'tools/list':
      return result(id, {
        tools: TOOLS.map((tool) => ({
          name: tool.name,
          title: tool.title,
          description: tool.description,
          inputSchema: tool.inputSchema,
          annotations: { readOnlyHint: Boolean(tool.readOnly), destructiveHint: false, openWorldHint: false },
        })),
      })
    case 'tools/call': {
      const tool = BY_NAME.get(params?.name)
      if (!tool) return failure(id, -32602, `no such tool: ${params?.name}`)
      const args = params.arguments ?? {}
      if (typeof args !== 'object' || args === null || Array.isArray(args)) {
        return result(id, { content: [{ type: 'text', text: 'arguments must be an object' }], isError: true })
      }
      try {
        const ran = await tool.run(args)
        return result(id, { content: [{ type: 'text', text: ran.text }], isError: ran.isError })
      } catch (error) {
        const why = error instanceof ToolError ? error.message : `unexpected failure: ${error?.message ?? error}`
        return result(id, { content: [{ type: 'text', text: why }], isError: true })
      }
    }
    case 'ping':
      return result(id, {})
    default:
      if (isNotification) return
      return failure(id, -32601, `method not found: ${method}`)
  }
}

console.error(`simrep-mcp workspace=${WORKSPACE} bin=${SIMREP_BIN ?? '(unset)'}`)

const rl = readline.createInterface({ input: process.stdin })
rl.on('line', (line) => {
  const text = line.trim()
  if (!text) return
  let msg
  try {
    msg = JSON.parse(text)
  } catch {
    return send({ jsonrpc: '2.0', id: null, error: { code: -32700, message: 'parse error' } })
  }
  void handle(msg).catch((error) => {
    if (msg.id !== undefined && msg.id !== null) failure(msg.id, -32603, String(error?.message ?? error))
  })
})
rl.on('close', () => process.exit(0))
