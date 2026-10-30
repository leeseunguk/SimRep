# AGENTS.md

Guidance for AI coding agents working in or driving this repository.

## What SimRep is

SimRep is a native Rust ion optics simulator, compatible with SIMION file
formats. It solves electrostatic and magnetostatic potentials (Laplace /
Poisson, multigrid, CPU and wgpu GPU), then integrates ion trajectories through
the fields. It reads SIMION PA/PA#/PA0, GEM, ION, and FLY2 files directly.
Three front ends share one core:

- `simrep`: batch CLI (`crates/simrep-cli`)
- `import simrep`: Python/NumPy package (`crates/simrep-py`), mirrors the CLI
- a Tauri 2 desktop app (`app/`)

For a step-by-step lens/analyzer (ESA) recipe with gates, failure diagnosis
and pitfalls, read `.claude/skills/simrep-ion-optics/SKILL.md` first.

## Build and install

```sh
# CLI
cargo build --release -p simrep-cli
./target/release/simrep --help

# Python package (Python 3.10+, stable abi3 wheel)
python3 -m venv .venv && . .venv/bin/activate
python -m pip install maturin numpy
maturin develop --release --manifest-path crates/simrep-py/Cargo.toml

# Optional STEP/IGES CAD import, needs OpenCascade (see docs/step-import.md)
cargo build --release -p simrep-cli --features cad
```

The workspace `cargo build`/`test` compiles an empty stub for `simrep-py`;
only maturin builds the real extension. Building a Python wheel with CAD:
`maturin build ... --features cad`.

MCP `optimize_design` runs a bounded, resumable GEM design loop; see `mcp/README.md`. Budget exhaustion yields a candidate, not an optimum.

## Core workflow

Geometry -> potential array -> refined field -> fly particles -> export.
Every CLI command has a same-named Python function; the full mapping table
lives in `docs/python.md`. Examples that exercise this workflow end to end
live in `examples/`.

| Task | CLI | Python |
|---|---|---|
| GEM file to PA# | `simrep gem2pa lens.gem [lens.pa#]` | `simrep.gem2pa("lens.gem")` |
| Triangle mesh to PA# | `simrep mesh-import in.stl out.pa# --grid nx,ny,nz` | `simrep.mesh_import(...)` |
| STEP/IGES to PA# | `simrep refine-geometry part.step ...` (feature `cad`) | `simrep.refine_geometry("part.step", ...)` |
| Measure STEP/IGES gaps | `simrep cad-measure part.step --gap 0 1` / `--all-pairs` (feature `cad`) | Rust `MeasurementModel::gap` / MCP `cad_measure`; see `docs/step-import.md` |
| Solve PA/PA# | `simrep refine file.pa#` | `field = simrep.refine("file.pa#")` |
| Solve exact surfaces | `simrep refine-geometry geo.gem --electrode SRC=N:V --out stem` | `simrep.refine_geometry("geo.gem", electrodes={src: (n, v)}, out="stem")` |
| Fly particles | `simrep fly --pa f.pa0 --particles ions.ion --voltage 1=0,2=110` | `simrep.fly(field, ions)` |
| Merge flight shards | `simrep merge-flights out.csv shard1.csv shard2.csv ...` | `simrep.merge_flights("out.csv", ["shard1.csv", "shard2.csv"])` |
| Change voltages, no re-solve | `--voltage N=V` on a PA0 with `.paN` siblings | `field.fast_adjust({2: 110})` |
| Time-varying voltages | `--voltage-expr N=EXPR --param name=value` or `--run-file run.json` | `voltage_expr={2: "..."}, parameters={...}` |
| Uniform B field | `fly --bfield Bx,By,Bz` (gauss) | `fly(..., bfield=[bx, by, bz])` |
| Magnetic PA | `fly --magnetic-pa map.pa` | `fly(..., magnetic_pa=["map.pa"])` |
| Coils (Biot-Savart) | `fly --bfield-coil coils.json` | `fly(..., coils=[{"kind": "loop", ...}])` |
| External B map | `fly --bfield-map export.txt` (docs/bfield-import.md) | `fly(..., run_file=...)` or bfield-map options |
| Neutral gas collisions | `fly --collision hs1|sds --seed 1` | `fly(..., collision="hs1", seed=1)` |
| Space charge | `fly --space-charge direct|pic` | `fly(..., space_charge="pic")` |
| Export field/surfaces | `simrep export grid|geometry|isosurface|trajectories ...` | `simrep.export_grid(...)` etc. |
| Compare aperture widths | `simrep aperture-check --pa lens.pa# --line x0,y0,z0,x1,y1,z1` | `simrep.aperture_check("lens.pa#", [line])` |
| Inspect a PA | `simrep pa-info file.pa` | `simrep.pa_info(...)` |

Particles come from `.ion` files (CSV rows: birth_us, mass_amu, charge_e,
x, y, z, azimuth_deg, elevation_deg, energy_ev, optional CWF, color), from
`.fly2` definitions, or from NumPy via `simrep.particles_from_energy(...)`.
Useful `fly` options: `--plane nx,ny,nz,d` detector, `--bounds` for drift
outside the PA, `--duration-us`, `--allow-time-limit`, `--integrator
rk45|boris`, `--tolerance`, `--output results.csv`.
`--surface-backend sdf` flies sampled collision surfaces on CPU or explicit GPU;
`--surface-sdf-resolution-divisor N` sets the sampling refinement. The reported
certified interpolation bound describes an approximation; CPU exact is the reference.
`--surface-backend auto` (`surface_backend="auto"` in Python) explicitly opts in
to measured GPU SDF selection; the default remains `exact`. The policy requires
device auto/gpu, a hardware Vulkan adapter, supported GPU SDF physics, and the
precision/divisor crossover. The f64/f32, N=2/4 table currently contains only
`None`, so it chooses CPU exact. `SIMREP_SDF_GPU_CROSSOVER=<ions>` overrides the
threshold for experiments, never the adapter or physics checks. Record any
future measured threshold in `docs/dev/sdf-auto-crossover.md` before enabling it.

### Portable flight shards

Run the same `fly` command on each process or PC, changing only `--shard i/N`,
`--output`, optional `--trajectories`, and `--gpu-adapter`. No MPI, scheduler,
or additional runtime installation is needed, including on Windows. `i` is
one-based and `1 <= i <= N`. A shard flies zero-based source indices
`k % N == i-1`; the CSV `ion` column remains the original one-based index.
The complete ION/FLY2 source is expanded before filtering, preserving birth
order and source sampling. CPU HS1/SDS and GPU Philox streams retain global
ion identities. Auto device selection uses the original total ion count.
Direct Coulomb, PIC, and beam/leader mode refuse sharding, even `--shard 1/1`.

```sh
simrep fly --pa lens.pa0 --particles ions.ion --device cpu --shard 1/2 --output a.csv
simrep fly --pa lens.pa0 --particles ions.ion --device cpu --shard 2/2 --output b.csv
simrep --json merge-flights out.csv a.csv b.csv
```

For different hardware GPUs use `--device gpu --gpu-adapter INDEX_OR_NAME`:
indices are zero-based within the requested backend; names are unambiguous,
case-insensitive substrings. Software adapters are still refused. Use the same
SimRep version, input bytes, physical settings, device/backend, and precision
on all workers. Adapter names and output paths do not enter the run hash;
CPU and GPU or f32 and f64 results are not interchangeable reference runs.

Every file-based CLI flight writes `OUT.csv.json`, containing a SHA256 run
hash (PA and basis bytes, surfaces, source, external physics files, settings,
and executed precision), CSV checksum, original total count, full-precision
TOFs, and the summary. Copy sidecars along with the CSVs. `merge-flights`
requires all partitions, including empty ones, checks identical run hashes,
shard layout, checksums, and exactly one of every global ion index, then
copies rows in global order without reformatting. Its CSV is byte-identical
to an unsharded run on the same numerical execution path. TOF statistics are
recomputed in global order from the full-precision values, not rounded CSV.

`--trajectories PATH --sample-every N` exports initial/final states and every
Nth accepted step with global indices. Keep trajectory files beside their
flight CSVs when copying between machines. GPU sampling also accepts
`--max-samples-per-ion`; capacity overflow is an error. A merge with trajectory
exports requires them on every shard and writes `OUT.trajectories.csv`, sorted
by global ion index while preserving sample order. It also writes `OUT.csv.json`.

## Device and precision rules

- `auto` (default for `fly`) picks by measured compute-only crossovers and
  stays on CPU below them. Magnetic flight is CPU under `auto` always.
- `--device gpu` refuses software adapters; on WSL/CI there is usually no
  hardware Vulkan/DX12 device. Use `--device cpu` (`device="cpu"` in Python)
  anywhere GPU support is uncertain.
- CPU is f64 everywhere. GPU solves run native f64 ("Precise") when the
  adapter supports `SHADER_F64`, otherwise f32 corrections with an f64
  iterate and residual (reported as precision `mixed-f32-f64`). GPU f32 "Preview" exists for live display only; it is a
  Rust API selection, not a promised accuracy.
- Exact-surface flying supports explicit GPU with `--surface-backend sdf`,
  including `--no-cuts --surface-collisions insulators|all`. The default exact
  backend, direct Coulomb, beam mode and `transparent_faces` remain CPU paths.
  Opt-in `--surface-backend auto` selects GPU SDF only above its measured
  precision/divisor crossover on hardware Vulkan; otherwise it selects CPU
  exact, even with `--device gpu`. SDF flight needs 10 storage bindings;
  native f64 uses f64 samples and the portable path uses f32 samples.

## Units and conventions

mm, microseconds (us), volts, gauss, amu, elementary charge e, energies in
eV, currents in amperes, pressure in Pa. All public APIs use these units.

- `.pa#`: unrefined PA, electrode nodes hold electrode numbers 1..1000.
- `.pa0` + `.pa1`..`.paN`: refined PA0 plus unit-voltage basis arrays; this
  family is what fast adjust scales. `.pa_` holds the scalable (nonlinear)
  basis when present.
- `Field.write("x.pa0")` emits the whole family, including `.cuts.json`
  when the field carries exact surfaces.
- `.cuts.json`: exact electrode surfaces written by `refine-geometry`;
  `fly` picks up a sibling automatically, `--no-cuts` (`no_cuts=True`)
  falls back to staircase voxel collisions. `refine-geometry --insulator PART`
  assigns a collision-only surface without a potential boundary. With `--no-cuts`,
  `--surface-collisions insulators|all` adds collision surfaces; GPU requires
  `--surface-backend sdf`, while CPU exact remains the reference.
- Electrode numbers are 1..1000 and map to the `e(n)`/`electrode(n)` values
  in GEM, not to stored volts.
- PA nodes sit at the origin; grid node (0,0,0) is at `origin_mm`. PA files
  do not store an origin; pass `--origin`/`origin_mm` consistently between
  geometry creation and flight.

## Machine-readable output

All commands accept a global `--json` flag. With it, the command prints one
JSON summary object to stdout and sends human-readable output (and flight
CSV records) to stderr, so `simrep fly --json ... | jq` sees only the
summary. The `simrep` Python module uses the same mechanism internally.

Every object has `schema_version` (integer, currently 1) and
`simrep_version` (string). On failure the object is
`{"error": {"message": string, "kind": string}}` and the exit status is 1.

| Command | Keys (besides the two above) |
|---|---|
| `fly` | `ion_count`, `arrivals` (detector-plane hits): int; `splat_counts`: {reason: int} over `electrode`, `outside`, `plane`, `time_limit`, `step_limit`, `step_underflow`, `nonfinite`, `beam_diverged`, `beam_leader_stopped`; `arrival_tof_us`: {`mean`,`std` (population),`min`,`max`: number or null}; `compute_ms`, `prep_ms`: number; `requested`, `executed`: selection (executed adds `cuts_path`: string or null and `cuts_source`: `explicit`, `sibling`, `disabled`, or `none`, `surface_collisions`: null, `insulators`, or `all`, `surface_backend`, `surface_sdf_resolution_mm`, `surface_sdf_lipschitz`, `surface_sdf_interp_error_mm`, and `surface_sdf_error_bound_mm`); `reason`: string; `fallback_reason`: string or null; `output_paths`: string[] |
| `merge-flights` | Flight count/statistics keys recomputed globally; `run_hash`, `total_ions`, `merged_shards`; `shard`: null; `compute_ms` and `compute_ms_sum`: sum of shard compute times; `compute_ms_max`: longest shard compute time; `prep_ms`: sum; `output_paths`: merged CSV, JSON sidecar, optional trajectory CSV |
| `refine`, `refine-geometry` | `requested`, `executed`: selection; `reason`; `fallback_reason`; `cycles`: int; `final_residual`, `tolerance`, `compute_ms`, `prep_ms`: number; `converged`: bool; `output_paths`: string[] |
| `gem2pa` | `output_paths`; `dims`: int[3]; `spacing_mm`: number[3]; `electrode_numbers`: int[]; `compute_ms` |
| `mesh-import` | `output_paths`; `dims`; `spacing_mm`; `electrode_numbers` |
| `export *` | `output_paths` |
| `aperture-check` | `apertures`: [{`line`, `staircase_mm`, `effective_mm`, `exact_mm`, `diff_mm`, `diff_cells`, `vacuum_intervals`, `warnings`}] |
| `pa-info` | `file`, `symmetry`, `field_type`: string; `mode`, `header_bytes`, `ng`, `points`, `electrode_points`, `non_electrode_points`: int; `dims`; `spacing_mm`; `mirrors`, `anti_mirrors`: bool[3]; `refinable`, `simion_compatible`: bool; `simion_issues`: string[]; `max_voltage`, `potential_min`, `potential_max`, `potential_mean`: number; `adjustable_electrodes`: int or null; `reference_voltage`: number or null |
| `gpu-info` | `adapter_name`, `adapter_type`, `backend`, `driver`, `driver_info`: string; `f64_support`, `software_adapter`: bool |
| `gpu-smoke` | `gpu-info` keys plus `checks`: [{`precision`, `kernel_ms` (or null), `submit_readback_ms`, `max_abs_error`, `tolerance`, `cells_checked`}] |

Flight summaries also include `run_hash`, `total_ions`, and `shard` (null for an
unsharded flight, otherwise `{index, count, ions}` with one-based `index` and
local `ions`). Requested/executed selections add `gpu_adapter` (selector/name,
or null). File-based CLI flights include their provenance sidecar in `output_paths`.

A selection is `{device, precision, backend}` (`backend` is null for CPU);
flight selections add `surface_mode` and `surface_backend`. Compare `requested`
with `executed` to detect a device or precision change, and read
`fallback_reason` for why. Benchmarks (`fly-bench`, `refine-bench`, `refine-sdf`)
also emit JSON; see `crates/simrep-cli/src/json.rs` and
`crates/simrep-cli/tests/json_cli.rs`.

`--surface-backend auto` reports `requested.surface_backend: "auto"`,
`executed.surface_backend: "exact"` or `"sdf"`, and `fallback_reason: null`.
Its `reason` describes the policy decision, for example
`auto: CPU exact (no measured GPU SDF crossover)`; a GPU decision includes
the ion count, crossover, precision and resolution divisor. This is a selected
execution path, not an error fallback.

Example:

```json
{"compute_ms":6.32,"converged":true,"cycles":29,
 "executed":{"backend":null,"device":"cpu","precision":"f64"},
 "fallback_reason":null,"final_residual":7.49e-11,
 "output_paths":["out.pa1","out.pa2","out.pa3","out.pa0"],"prep_ms":3.81,
 "reason":"explicit --device cpu",
 "requested":{"backend":"vulkan","device":"cpu","precision":"f64"},
 "schema_version":1,"simrep_version":"0.1.0","tolerance":1e-10}
```

## Common errors and fixes

Quoted strings are the real messages emitted by the code.

- `--shard requires i/N with 1 <= i <= N`: supply a valid one-based partition.
- `--shard cannot be used with coupled physics: direct Coulomb, PIC space charge, or beam/leader mode`: fly the complete coupled ensemble without sharding.
- `--gpu-adapter requires --device gpu`: explicit adapter selection requires explicit GPU flight.
- `GPU adapter index {index} not found for {backends:?}` / `GPU adapter name {selector:?} not found for {backends:?}`: choose an adapter available to that backend; ambiguous name matches require an index.
- `merge-flights run hash or shard layout mismatch`: use identical inputs, settings, execution precision, and shard count.
- `merge-flights duplicate shard` / `merge-flights missing shard or ion index`: supply every partition exactly once, including empty partitions.
- `merge-flights CSV checksum mismatch` / `merge-flights trajectory checksum mismatch`: copy the original CSV and matching sidecar/trajectory files without editing them.
- `--device gpu requires a hardware GPU; adapter <name> (<backend>) is a
  CPU/software adapter; use --device cpu`: WSL, CI, or a system without a
  hardware adapter. Pass `--device cpu` / `device="cpu"`.
- `--surface-collisions requires --no-cuts`: use the flag with staircase flight.
- `surface collisions require CPU; use --device cpu or auto, or --no-cuts
  for staircase GPU flying`: the default exact backend is CPU-only. Select
  `--surface-backend sdf` for approximate GPU surface flight.
- `SDF volumes exceed GPU storage limit (... MB > ... MB); use a smaller --surface-sdf-resolution-divisor or --device cpu`: reduce surface sampling refinement or use CPU.
- `refinement did not converge after {cycles} cycles` /
  `electrode {number} did not converge after {cycles} cycles; increase
  max_cycles`: raise `max_cycles`/`--max-cycles`, loosen `--tolerance`, or
  check for floating/overlapping electrode constraints.
- `part {i} {name:?} requires exactly one --electrode or --insulator assignment`: every
  geometry part needs either `--electrode part=number:voltage` (GEM: `SRC=N:V`,
  mesh/CAD: name or index) or `--insulator part`, but not both. Unused
  selectors also fail.
- `electrode {number} has no inside/on-surface node; refine the grid`:
  the electrode's surface contains no grid node; use a finer `--grid` or
  thicker part. Analytic-shape input needs a node on the surface.
- `gap check failed: fewer than 2.5 cells across an electrode gap or an unresolved GEM gap`:
  reduce `--spacing` (10 cells across is recommended), or explicitly select
  `--gap-check warn|off`; GEM reports certified brackets, not exact distances.
- `refine input must have a .pa or .pa# extension` /
  `refine-geometry input must be .gem, .step, .stp, .iges, .igs, .stl,
  .obj, .ply, .3mf, .gltf or .glb`: wrong file kind for the command.
- `STEP/IGES import requires cargo build -p simrep-cli --features cad;
  see docs/step-import.md`: rebuild with the `cad` feature (needs OCCT).
- `{n} particles did not reach an allowed termination; inspect CSV
  reasons`: ions hit time/step limits. Add a detector `--plane`, extend
  `--bounds`, raise `--duration-us`, or opt in with `--allow-time-limit`.
- `PA is marked non-refinable (field map)`: the PA is a fixed field map;
  refine the source `.pa#` instead.
- `electrode voltages require --pa`: `--voltage` only applies to a PA0
  fast-adjust family supplied via `--pa`.
- `nonlinear permittivity cannot use PA# fast-adjust bases; refine an
  ordinary PA at its actual voltages`: dielectric nonlinear solves do not
  superpose.
- `leader-plane beam integration requires --device cpu or auto` /
  `direct Coulomb physics requires --device cpu or auto`: CPU-only physics.
- `SDS requires --mbmr /path/to/mbmr.dat`: the SDS statistics file is not
  bundled; supply SIMION's mbmr.dat.
- Python `ValueError: electrode {n} has no fast-adjust basis`: adjusting a
  number the field was not solved for; use the numbers reported by
  `field.voltages.keys()`.

## Performance measurement rule

Published speed numbers compare compute time only. Use `compute_ms` (file
reading, parsing, GPU init, basis loading, and export are excluded);
`setup_ms`/`total_ms` in Python reports are for context. This mirrors the
policy in `docs/dev/verification.md` and `bench/`.

## Testing and verification

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo clippy -p simrep-py --features python -- -D warnings
cargo test --workspace --release
. /tmp/simrep-python/bin/activate  # or your venv
python -m pytest crates/simrep-py/tests -q
SIMREP_BIN=./target/release/simrep node mcp/smoke.mjs
python3 tools/result_identity.py --selftest  # compare before/after run directories; see docs/dev/verification.md
```

Oracle/fixture-gated tests are ignored by default and need env vars plus
`--ignored` (see `docs/dev/verification.md`, which is the authoritative
verification guide). `tools/check_claims.py` validates quantitative claims
in docs and the report against `docs/claims.json`; run it after changing
documented numbers. `tools/provenance_check.py` pins every committed
reference/fixture/answer-key file listed in `docs/validation/PROVENANCE.md`
by sha256; run it after adding or editing any such file (`--add PATH`
registers a new row).

## Repository layout

- `crates/simrep-core`: grid, potential array, geometry and shared types
- `crates/simrep-solve`: potential solvers: multigrid (CPU/GPU), cut-cell,
  residual correction
- `crates/simrep-fly`: field interpolation, integrators and particle
  flying (CPU/GPU)
- `crates/simrep-io`: SIMION file formats (PA/PA#/PA0, fast adjust, ION,
  FLY2) and imported magnetic field maps
- `crates/simrep-gem`: SIMION GEM constructive geometry parser and PA
  rasterizer
- `crates/simrep-geom`: signed-distance electrode geometry
- `crates/simrep-mesh`: native triangle mesh import, voxelization, SDF and
  scientific export (VTI/VTS fields, VTP trajectories, USDA mesh/curve scenes)
- `crates/simrep-cad`: optional OpenCascade B-rep electrode import
- `crates/simrep-lua`: bounded Lua 5.1 execution shared by SIMION input
  formats
- `crates/simrep-cli`: batch command-line runner
- `crates/simrep-py`: NumPy and Python parameter scans for SIMION_Rep
- `app/`: Tauri 2 + React desktop app (separate npm workspace)
- `docs/`: design and validation docs; start at `docs/PLAN.md`
- `examples/`: runnable end-to-end Python/CLI examples
- `mcp/`: stdio MCP server over the CLI (`node mcp/smoke.mjs`; see `mcp/README.md`)
- `ai/`: subscription-authenticated AI sidecar; read `ai/PROTOCOL.md` before UI changes and run `cd ai && npm test && npm run typecheck`. Never log or move provider credentials into the WebView or repository.
- `ai/harness/`: provider-neutral SimRep context, lazy ion-optics recipes, result verification and bounded outputs; see `ai/harness/README.md`. Run `node ai/harness/evals/run.mjs` for deterministic mock-model workflow evals.
- `bench/`, `tools/`: benchmark records and dev/oracle utilities

## Dev pitfalls

- Windows binaries launched from WSL hang unless stdin is closed
  (`< /dev/null`), they get a `timeout`, and the cwd is under `/mnt/c`.
  Cross-boundary env vars need `WSLENV`. Details: `docs/python.md`
  ("Windows build with cargo.exe") and `tools/win/winproc.ps1`.
- GPU checks under Mesa/lavapipe need `GALLIUM_OVERRIDE_CPU_CAPS=avx` and
  `LP_NATIVE_VECTOR_WIDTH=256`; see `docs/dev/verification.md`.
- `simrep.py`'s shared commands run the CLI frontend in-process; their
  stdout/stderr text is part of the Python API surface, so do not "clean
  up" CLI output without checking `test_cli_parity.py`.
