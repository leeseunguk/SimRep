# Verification paths

Run commands from the repository root unless a command changes directory.
The root Cargo workspace contains `crates/*`; it does not include the Tauri
app, Python extension behavior, or optional CAD features.

## Comparing results before and after a change

`tools/result_identity.py` compares two files or two run directories using only
Python's standard library. Exit status is **0** for equal results within the
selected tolerances, **1** for different results or missing artifacts, and **2**
for invalid inputs or an I/O error. Defaults use zero numeric tolerances.

```sh
python3 tools/result_identity.py before/lens.pa0 after/lens.pa0
python3 tools/result_identity.py before/ after/ --json
python3 tools/result_identity.py before/ after/ --require-byte-identity
python3 tools/result_identity.py before/lens.cuts.json after/lens.cuts.json --ulps 4 --tolerance 1e-12
python3 tools/result_identity.py before/flight.csv after/flight.csv \
  --tof-tolerance 1e-9 --position-tolerance 1e-8 --energy-tolerance 1e-8
python3 tools/result_identity.py before/summary.json after/summary.json --ignore-key output_paths
python3 tools/result_identity.py --selftest
```

Directory mode matches relative filenames recursively and checks PA, PA#, PA0,
PA1..N, PA_, `.cuts.json`, flight CSV, and JSON artifacts. Use the same output
stem in each directory. Other extensions, such as GEM source and log files,
are excluded. Compare whole directories to include every fast-adjust basis and
adaptive patch; comparing a manifest alone checks its contents, not referenced
files. CSV files must use the flight-output schema, not trajectory-export CSV.

PA reports include byte identity, header fields and PA0 trailer differences,
electrode-mask changes, the number of changed potentials and nodes outside
tolerance, maximum and RMS dV in volts, and the worst flat node index and
x/y/z grid coordinates. Physical offsets use the PA spacing: PA files do not
store `origin_mm`. The numeric report is still produced when bytes differ.
`--require-byte-identity` makes any byte difference fail, including signed zero
or JSON formatting differences that structural comparison would accept.

Cuts sidecars are compared structurally, including v2 sampled data and v3
source geometry. Run-encoded node electrodes and solver-fixed masks are
expanded; dense and sparse intersections are normalized to node/direction
records. JSON key order and sparse intersection order do not matter.
Fractions, normals and other floating fields use the selected tolerances;
electrode numbers, masks, array order and version remain significant. Source
geometry payloads are compared exactly rather than decoded. Adaptive manifests
are structural JSON comparisons: patch order, levels, factors, filenames,
bounds, origins, spacings and electrode voltages remain significant.

Flight CSV rows stay in order, with exact ion IDs, termination reasons and
electrode numbers. TOF, position, kinetic energy and velocity are numeric.
`--tolerance` (alias `--abs-tolerance`) applies in each field's stored units;
the three flight-specific options override its absolute tolerance for those
columns. `--relative-tolerance` and binary64 `--ulps` apply alongside absolute
tolerance: satisfying any one is sufficient. Integer-valued JSON counts and
indices always compare exactly. Non-finite values and malformed PA, CSV, JSON
or cuts encodings are errors, even if both inputs contain the same invalid data.

Summary JSON ignores only `compute_ms`, `prep_ms`, `setup_ms`, `total_ms`,
`elapsed_ms`, `end_to_end_ms`, `kernel_ms` and `submit_readback_ms`, at any
depth. Physics time fields such as `arrival_tof_us` are retained. Output paths
remain significant unless explicitly excluded with `--ignore-key output_paths`;
this repeatable option omits the named JSON key at every depth.
`--json` emits one report, with `identical`, `byte_identical`, per-file metrics
and bounded difference details; `--max-diffs` controls the detail limit.

The Rust CI job runs the synthetic self-tests and refines
`examples/data/einzel.gem` on CPU with one Rayon worker and with the default
worker count, requiring byte-identical PA# and fast-adjust families. Reproduce
the check with a built CLI:

```sh
work="$(mktemp -d)"
for run in single default; do
  mkdir "$work/$run"
  target/release/simrep gem2pa examples/data/einzel.gem "$work/$run/einzel.pa#"
done
RAYON_NUM_THREADS=1 target/release/simrep refine "$work/single/einzel.pa#" --device cpu
target/release/simrep refine "$work/default/einzel.pa#" --device cpu
python3 tools/result_identity.py "$work/single" "$work/default" --require-byte-identity
```

## GitHub Actions

`.github/workflows/ci.yml` runs only through manual `workflow_dispatch`, by
repository owner decision. Five Ubuntu jobs check Rust
formatting, both Clippy configurations and release tests; the built Python
extension and CLI parity; frontend types, build, unit tests and Chromium demo
E2E; published claims against committed docs/PDF; and native Tauri tests with
Linux GTK/WebKit dependencies. Cargo and npm dependencies are cached.
Rust 1.95.0 and Node 24.14.1 match the verified development toolchains; native
shell formatting is checked separately from the root workspace.
`AtomicU64::try_update` (stable since Rust 1.95.0) replaces the identically
behaving `fetch_update`, deprecated in Rust 1.99.0. See the
[Rust 1.95 atomic API](https://doc.rust-lang.org/1.95.0/std/sync/atomic/struct.AtomicU64.html#method.try_update).
Before the compatibility fix, Rust 1.99.1 Clippy rejected constant
`chunks_exact` calls in `simrep-gem` (`geometry.rs:945`, `raster.rs:130`,
`stl.rs:106,133`) with `chunks_exact_to_as_chunks`. Equivalent `as_chunks`
conversions fix these and the same lint elsewhere in the default workspace.
They keep complete chunks in order and ignore the remainder, exactly as
`chunks_exact` did; numerical operations and test assertions are unchanged.
Workspace and Python-feature Clippy with `-D warnings`, and the complete
release workspace tests, now pass on both Rust 1.95.0 and 1.99.1. The CI pin
remains 1.95.0 until a complete stable-toolchain CI run is verified; passing
local checks alone does not lift that gate.

The histogram-axis test checks the selected decade within one binary64 ULP
and verifies that it brackets half the smallest positive bin. Node 22.23.4
(V8 12.4.254.21-node.57) computes `10 ** -5` as
`0.0000099999999999999991240`, while Node 24.14.1 (V8
13.6.233.17-node.44) computes `0.000010000000000000000818`; the literal `1e-5`
has the latter value on both. `Math.log10(1e-4 / 2)` agrees on both, so this
is power-function rounding, not summation order or a different decade.
Both versions pass all 56 unit tests, typechecking, and all three Chromium
demo E2E tests after the assertion change; product arithmetic is unchanged.

Rust and Python install Mesa lavapipe and disable Gallivm AVX-512 with
`GALLIUM_OVERRIDE_CPU_CAPS=avx` and `LP_NATIVE_VECTOR_WIDTH=256`. Despite its
name, Mesa's `avx` tier retains AVX2/F16C/FMA and disables all AVX-512 features.
The first limits ISA selection; the second fixes the shader vector width.
Width alone does not disable AVX instructions, as explained in the
[LLVMpipe environment-variable documentation](https://docs.mesa3d.org/drivers/llvmpipe.html#environment-variables).
The ISA override values are documented in
[Mesa environment variables](https://docs.mesa3d.org/envvars.html#envvar-GALLIUM_OVERRIDE_CPU_CAPS).
Both jobs log the host CPU model and flags, the requested cap/width, and
`vulkaninfo --summary`, whose `deviceName` includes Mesa's chosen vector width.
`GALLIUM_DUMP_CPU=1` on the Vulkan probe command also audits the post-override
capability flags. It is not set for tests: the process-start dump otherwise
pollutes Python/CLI exact-output parity checks. The
[Mesa 26.0.8 CPU override](https://gitlab.freedesktop.org/mesa/mesa/-/blob/60e95b787857afbc9a00b693b91c0d9c8923a430/src/util/u_cpu_detect.c#L603-691)
applies before vector-width selection in
[`lp_bld_init_common.c`](https://gitlab.freedesktop.org/mesa/mesa/-/blob/60e95b787857afbc9a00b693b91c0d9c8923a430/src/gallium/auxiliary/gallivm/lp_bld_init_common.c#L68-80).
[`lp_build_fill_mattrs`](https://gitlab.freedesktop.org/mesa/mesa/-/blob/60e95b787857afbc9a00b693b91c0d9c8923a430/src/gallium/auxiliary/gallivm/lp_bld_misc.cpp#L316-363)
passes the cleared AVX-512 flags to LLVM explicitly. LLVM still detects the
host CPU for scheduling; this is an ISA/vector cap, not a CPU-model override.
`GALLIVM_PERF` has `no_avx` but no AVX-512-only switch, and
`GALLIVM_LLC_OPTIONS` is debug-build-only, so neither is used.
Rust also executes the Vulkan kernel smoke check. This verifies software
Vulkan, not physical GPU performance. The 15 external-fixture tests remain ignored, and Python hardware
GPU numeric parity tests report skips. CAD/OCCT and Windows installer/hardware
checks remain the explicit local paths below.

### CPU/ISA verification evidence

The uncapped control at `5a21e5a` is
[run 40222435247](https://github.com/leeseunguk/SimRep/actions/runs/40222435247).
Attempt 1 used Intel Xeon Platinum 8573C, with AVX-512 flags reported by
`lscpu`, and both magnetic tests failed. The imported-map result reported
`NonFinite` with zero accepted steps. Attempt 2 reran Rust on AMD EPYC 9V74
at the same SHA and both tests passed. Both adapters reported Mesa 26.0.8,
LLVM 21.1.8, and 256 bits, and both runners had two vCPUs: fixing vector
width alone cannot distinguish that control failure.

Configured jobs use the AVX2-tier cap and 256-bit width described above.
The tables record completed jobs separately because a job-only rerun does
not re-execute the other jobs.

| Rust run / attempt | Head | CPU | Imported maps | Uniform PA/coils/Boris |
| --- | --- | --- | --- | --- |
| [40245295332 / 1](https://github.com/leeseunguk/SimRep/actions/runs/40245295332) | `73a63b1` | AMD EPYC 7763 | pass | pass |
| [40246518664 / 1](https://github.com/leeseunguk/SimRep/actions/runs/40246518664) | `250ad0e` | AMD EPYC 7763 | pass | pass |
| [40246518664 / 2](https://github.com/leeseunguk/SimRep/actions/runs/40246518664) | `250ad0e` | AMD EPYC 9V74 | pass | pass |
| [40246550259 / 1](https://github.com/leeseunguk/SimRep/actions/runs/40246550259) | `250ad0e` | AMD EPYC 7763 | pass | pass |
| [40247877017 / 1](https://github.com/leeseunguk/SimRep/actions/runs/40247877017) | `e297a09` | AMD EPYC 7763 | pass | pass |
| [40250762926 / 1](https://github.com/leeseunguk/SimRep/actions/runs/40250762926) | `2a23bb9` | AMD EPYC 7763 | pass | pass |

| Python run / attempt | Head | CPU | Result |
| --- | --- | --- | --- |
| [40245295332 / 1](https://github.com/leeseunguk/SimRep/actions/runs/40245295332) | `73a63b1` | AMD EPYC 7763 | 80 pass, 1 output-parity failure, 6 hardware-only skips |
| [40246518664 / 1](https://github.com/leeseunguk/SimRep/actions/runs/40246518664) | `250ad0e` | AMD EPYC 7763 | 81 pass, 6 hardware-only skips |
| [40246550259 / 1](https://github.com/leeseunguk/SimRep/actions/runs/40246550259) | `250ad0e` | AMD EPYC 9V74 | 81 pass, 6 hardware-only skips |
| [40246550259 / 2](https://github.com/leeseunguk/SimRep/actions/runs/40246550259) | `250ad0e` | AMD EPYC 7763 | 81 pass, 6 hardware-only skips |
| [40246550259 / 3](https://github.com/leeseunguk/SimRep/actions/runs/40246550259) | `250ad0e` | AMD EPYC 7763 | 81 pass, 6 hardware-only skips |
| [40246550259 / 4](https://github.com/leeseunguk/SimRep/actions/runs/40246550259) | `250ad0e` | AMD EPYC 7763 | 81 pass, 6 hardware-only skips |
| [40246550259 / 5](https://github.com/leeseunguk/SimRep/actions/runs/40246550259) | `250ad0e` | AMD EPYC 7763 | 81 pass, 6 hardware-only skips |
| [40246550259 / 6](https://github.com/leeseunguk/SimRep/actions/runs/40246550259) | `250ad0e` | AMD EPYC 7763 | 81 pass, 6 hardware-only skips |
| [40246550259 / 7](https://github.com/leeseunguk/SimRep/actions/runs/40246550259) | `250ad0e` | AMD EPYC 9V45 | 81 pass, 6 hardware-only skips |
| [40246550259 / 8](https://github.com/leeseunguk/SimRep/actions/runs/40246550259) | `250ad0e` | AMD EPYC 7763 | 81 pass, 6 hardware-only skips |
| [40246550259 / 9](https://github.com/leeseunguk/SimRep/actions/runs/40246550259) | `250ad0e` | AMD EPYC 7763 | 81 pass, 6 hardware-only skips |
| [40246550259 / 10](https://github.com/leeseunguk/SimRep/actions/runs/40246550259) | `250ad0e` | AMD EPYC 9V74 | 81 pass, 6 hardware-only skips |
| [40246550259 / 11](https://github.com/leeseunguk/SimRep/actions/runs/40246550259) | `250ad0e` | AMD EPYC 9V45 | 81 pass, 6 hardware-only skips |
| [40246550259 / 12](https://github.com/leeseunguk/SimRep/actions/runs/40246550259) | `250ad0e` | AMD EPYC 7763 | 81 pass, 6 hardware-only skips |
| [40246550259 / 13](https://github.com/leeseunguk/SimRep/actions/runs/40246550259) | `250ad0e` | AMD EPYC 9V74 | 81 pass, 6 hardware-only skips |
| [40246550259 / 14](https://github.com/leeseunguk/SimRep/actions/runs/40246550259) | `250ad0e` | AMD EPYC 9V45 | 81 pass, 6 hardware-only skips |
| [40246550259 / 15](https://github.com/leeseunguk/SimRep/actions/runs/40246550259) | `250ad0e` | AMD EPYC 7763 | 81 pass, 6 hardware-only skips |
| [40246550259 / 16](https://github.com/leeseunguk/SimRep/actions/runs/40246550259) | `250ad0e` | AMD EPYC 9V74 | 81 pass, 6 hardware-only skips |
| [40246550259 / 17](https://github.com/leeseunguk/SimRep/actions/runs/40246550259) | `250ad0e` | AMD EPYC 9V74 | 81 pass, 6 hardware-only skips |
| [40246550259 / 18](https://github.com/leeseunguk/SimRep/actions/runs/40246550259) | `250ad0e` | AMD EPYC 7763 | 81 pass, 6 hardware-only skips |
| [40246550259 / 19](https://github.com/leeseunguk/SimRep/actions/runs/40246550259) | `250ad0e` | AMD EPYC 9V74 | 81 pass, 6 hardware-only skips |
| [40246550259 / 20](https://github.com/leeseunguk/SimRep/actions/runs/40246550259) | `250ad0e` | AMD EPYC 7763 | 81 pass, 6 hardware-only skips |
| [40247877017 / 1](https://github.com/leeseunguk/SimRep/actions/runs/40247877017) | `e297a09` | AMD EPYC 9V45 | 81 pass, 6 hardware-only skips |
| [40250762926 / 1](https://github.com/leeseunguk/SimRep/actions/runs/40250762926) | `2a23bb9` | AMD EPYC 9V74 | 81 pass, 6 hardware-only skips |
| [40253903430 / 1](https://github.com/leeseunguk/SimRep/actions/runs/40253903430) | `9906384` | AMD EPYC 9V74 | 81 pass, 6 hardware-only skips |

The first configured run's Python failure came from setting
`GALLIUM_DUMP_CPU` for the entire job: process-start diagnostics changed
the CLI-output oracle. Scoping it to the Vulkan probe fixed that failure
without changing the parity assertion. Run 40246550259 attempt 1 was green
across all five jobs.

No configured Intel job has been allocated yet. These AMD results do not
establish that the Intel failure is fixed. Sampling has included new push
and dispatch runs, job-only reruns, and a temporary explicit `ubuntu-26.04`
request. That historical workflow restored `ubuntu-latest`, per-ref concurrency,
and push/PR filters for `main`. Automatic triggers are now disabled; only
manual dispatch remains. The repository currently
has no self-hosted runner to target a particular CPU vendor.

## Default workspace

### Numerical uncertainty, MMS, and computed verdicts

`simrep_core::uncertainty` implements Celik et al. (2008), JFE 130, 078001,
DOI `10.1115/1.2960953`: positive fine-to-coarse grid sizes, unequal
refinement ratios, observed order, Richardson limit, and relative fine-grid
GCI with safety factor 1.25. The unequal-ratio equation is solved in a
bracket rather than a potentially divergent fixed-point iteration. Zero
finest values retain their order and extrapolation but have no relative
GCI. Oscillatory, divergent, or plateau triplets have no assigned order.
`crates/simrep-core/tests/uncertainty.rs` checks both monotone worked
examples in Celik Table 1 against its rounded published numbers.

For more than three levels or noisy data, `grid_study` follows Eca and
Hoekstra (2014), JCP 262, 104-130, DOI `10.1016/j.jcp.2014.01.006`,
Appendix A: unweighted and inverse-size weighted fits; power, first,
second, and first+second error models; residual degrees of freedom;
fit-selection rules; and the scatter-dependent safety/uncertainty branches.
The nonlinear power fit searches `0.05 <= p <= 8`. It is a bounded
single-power asymptotic model, not a general extrapolator for arbitrary
oscillations. Identical data on every grid do not identify an order or a
data-range uncertainty and are rejected explicitly. Grid spacing is a
positive characteristic length; do not supply node counts as sizes.

The Python `simrep.uncertainty` module exposes `observed_order(h, values)`
and `grid_study(h, values)` through the same Rust implementation.
`validation(S, D, u_num, u_input=0, u_datum=0)` returns `E=S-D` and
`u_val=RSS(u_num,u_input,u_datum)`, following ASME V&V 20. Terms must be
independent and use compatible coverage conventions; a safety-factored GCI
must not silently be presented as a calibrated standard deviation. A
single-grid tolerance or solver residual is not a numerical uncertainty.

```sh
cargo test --release -p simrep-core --test uncertainty -- --nocapture
cargo test --release -p simrep-solve --test mms -- --nocapture
cargo test --release -p simrep-fly --test mms -- --nocapture
python -m pytest crates/simrep-py/tests/test_uncertainty.py -q
python3 tools/validation_gates.py --check
# Intentionally larger CPU study, not part of the default gate snapshot:
cargo test --release -p simrep-solve --test mms larger_mms_study -- --ignored --nocapture
# After reviewing new measurements:
python3 tools/validation_gates.py --write
```

The default smooth-solver ladders have 9/17/33 nodes per active direction;
cut surfaces and field reconstruction use 17/33/65. Every ladder has
spacing ratio 2 and at least three levels. MMS sources are analytic nodal
values, not discrete sources generated by the operator under test. The
cut-cell study compares four subcell phases in the same translated frame,
and separates potential from the maximum field error within two cells of
the surface. The ignored 33/65/129 smooth and 65/129/257 cut-cell study
tests the approach to the asymptotic range rather than concealing a
coarse-grid order shortfall.

[`../validation/GATES.md`](../validation/GATES.md) is generated from records
emitted at the calculation by the default MMS tests and three existing
comparison tests (Poisson, coaxial dielectric, parallel-plate TOF). Their
original assertions are unchanged. Each gate names its reference, metric,
value, numerical uncertainty when estimated, and band. PASS means
`abs(E) <= u_val`, MISS means `abs(E) > u_val`, and an unestimated
uncertainty remains OPEN. These verdicts concern distinguishable
discrepancy, not proof of physical model validity. Order measurements have
no estimated uncertainty on order and remain OPEN; their bands explicitly
state whether the documented order was reached. Smooth centre-potential
comparisons use an actual three-grid GCI.

CI recomputes the table, checks the complete expected gate population,
and compares the committed four-significant-figure snapshot. Missing,
duplicate, new unregistered, or changed rows fail the check. Hardware GPU,
experimental data, external oracle fixtures, nonlinear materials, and
flight-time discretization uncertainty are not covered by these new gates.

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo clippy -p simrep-py --features python -- -D warnings
cargo test --workspace --release
```

### Numerical repeatability

`crates/simrep-cli/src/repeatability.rs` runs identical generated workloads
inside local Rayon pools requesting 1, 2, 8 and 32 threads, in one process.
The first run supplies the reference bytes; every other run must match.
Floating-point values use their complete little-endian binary64 encoding,
so even a signed-zero or one-bit difference fails. Complete PA families,
cuts sidecars and flight CSV files are compared byte for byte as well.
Only wall-clock measurements are excluded from JSON/report comparisons;
iteration counts, every recorded residual and terminal reasons are retained.

| Workload | Compared output |
| --- | --- |
| CPU multigrid refinement | Full potentials, absolute/relative residual histories, cycle and level counts |
| GEM and triangle-source `refine-geometry` | PA#, PA0, both unit-voltage bases, dense cuts-v3 sidecar and stable JSON summary |
| Mesh/CAD triangle preparation | Parallel BVH construction above its 32,768-triangle threshold, narrow-band SDF, voxel masks/assignments, exact segment roots, cut-node count and cut-aware gradients |
| Independent RK45 and Boris flight | Ordered final states, TOFs, termination reasons, accepted/rejected steps and saved trajectories |
| CLI flight statistics | Production population TOF mean/std/min/max, arrival/splat counts and complete CSV |
| Direct Coulomb and CIC PIC | Fields, PIC potentials/residual/cycles and coupled ensemble trajectories; populations exceed the direct parallel threshold |
| Line/ring beams | Coupled trajectories, global/rejected step counts and beam terminations |
| Seeded HS1 and SDS collisions | Full flight results and trajectories; another seed must change results; SDS uses generated monotone ICDFs, not external SIMION data |
| GPU versus the same GPU | Mixed and supported native-f64 multigrid/cut-cell solves, resident magnetic/HS1 flight and sampled trajectories, supported f64 PIC fields/potentials/coupled flight |

Small multigrid problems deliberately cap their internal pool at four
workers. The CPU solver test verifies that policy and also solves a grid
above the cap's size threshold using all 1, 2, 8 and 32 requested workers.
The triangle test exercises the native mesh preparation path also used by
tessellated CAD; optional OCCT tessellation and exact B-rep queries are not
covered by the default workspace.

The guarantee tested here is repeatability for unchanged ordered inputs,
settings and random seeds on the same build and machine, independent of
requested Rayon worker count. It is not equality between CPU and GPU,
different GPU adapters/drivers, CPU instruction sets or compiler versions.
GPU checks reuse one selected Vulkan device and accept software adapters.
No adapter is an explicit `SKIP`; missing `SHADER_F64` skips only native-f64
solve/PIC checks, leaving mixed solve and flight checks active. Device,
shader and execution failures are errors, not skips.

Run this matrix directly with adapter/skip diagnostics visible:

```sh
GALLIUM_OVERRIDE_CPU_CAPS=avx LP_NATIVE_VECTOR_WIDTH=256 \
  cargo test -p simrep-cli --release repeatability_ -- --nocapture
```

The CPU audit found no variation requiring a new summation algorithm.
GPU multigrid, cut-cell and resident HS1 flight also repeated exactly on
llvmpipe. The new coupled GPU PIC regression did expose variation: atomic
cell-list insertion made floating-point charge sums depend on dispatch
scheduling. Cell lists now use atomic ordered insertion in descending source
index, with links cleared before every binning pass. Deposition therefore
uses the same addition order for unchanged ordered inputs. This needs no
exact-sum dependency and leaves the CPU arithmetic unchanged.

On October 27, 2026, all seven regressions passed on the Ryzen 9 5950X with
llvmpipe (LLVM 20.1.2, 256 bits, native f64), using the Mesa caps above.
The existing `SIMREP_PIC_BENCH=1 cargo test -p simrep-fly --release --test
gpu_pic -- --nocapture` workload measures `EnsembleReport.compute_time`:
100,000 particles, a 33-cubed PIC grid, three global steps and three trials,
excluding mesh/device/shader preparation. Median GPU-path compute time was
1,386.083 ms before ordered insertion and 1,226.212 ms after it. No slowdown
was observed in this software-adapter run. The CPU control also changed from
220.921 to 200.947 ms, so the observed 11.5% GPU-path decrease is not an
isolated speedup claim or a hardware-GPU performance result.

`crates/simrep-cli/tests/json_cli.rs` runs tiny generated GEM, PA, ION and OBJ
fixtures through `--json`: `gem2pa`, `refine`, `refine-geometry`, `fly`,
`mesh-import`, `pa-info`, every `export` subcommand and CPU benchmarks.
It parses all stdout as one JSON object, checks version/key types, arrival TOF
statistics (including no arrivals), requested/executed settings, output files
and exit-1 error objects for invalid arguments and bad files. `gpu-info` and
`gpu-smoke` accept software Vulkan; the test reports an explicit skip when
no adapter is available. No hardware GPU is required.
Python CLI-parity tests compare the parsed diagnostic/export/benchmark results
and check that these helpers no longer print to stdout.

The 15 external-fixture/benchmark tests below are reported as **ignored** by
default, even when their environment variables are set. Run them with both
the variables and `--ignored`. An explicit run without a required variable
fails instead of counting an unexecuted check as passed. Other GPU harnesses
can report adapter-dependent skips separately; workspace success alone is
not evidence of validation on physical GPU hardware.

## Ignored installed SIMION examples

`installed_simion_examples_read_and_round_trip_without_copying_fixtures` in
`crates/simrep-io/tests/simion_examples.rs` requires an installed SIMION examples
directory. Set `SIMION_ROOT` to the installation root (a host-native path, such
as `/mnt/c/Program Files/SIMION-2024` under WSL), then opt in explicitly:

```sh
SIMION_ROOT=/path/to/SIMION-2024 cargo test --release -p simrep-io \
  --test simion_examples -- --ignored --nocapture
```

An explicit run fails if `SIMION_ROOT` or its `examples` directory is absent.
The installed examples passed when explicitly enabled on the validation
workstation. An explicit run with `SIMION_ROOT` unset failed with
`requires SIMION_ROOT`, rather than reporting an unexecuted pass.

## Local portability validation

The ISA-cap change was also checked locally with Rust 1.95.0, Mesa 25.2.8,
LLVM 20.1.2, `GALLIUM_OVERRIDE_CPU_CAPS=avx`, and
`LP_NATIVE_VECTOR_WIDTH=256`. Root and native-shell formatting checks,
workspace and Python-feature Clippy with `-D warnings`, the complete
release workspace suite, and the Vulkan kernel smoke check passed. Both
magnetic GPU tests executed and passed with native f64. The kernel smoke
check passed f32 and f64, with f64 maximum absolute error zero against the
CPU reference.

Rust 1.99.1 and both Node results are described above. Rust and TypeScript
language-server diagnostics were unavailable on the workstation; the
compiler, Clippy, and `npm run typecheck` were used instead.

## Ignored CPU oracle and sweep tests

Use existing, locally captured SIMION fixtures; these are not downloaded by
the tests. Replace the example absolute paths with your fixture locations.
`ORACLE` contains the magnetic case directories; `FIXTURES` contains
`_benchfly/einzel.pa0`, its fast-adjust siblings, and `_benchfly/einzel_10000.ion`.
`MBMR` is the installed SIMION diffusion-statistics file. Four Python oracle
tools - `tools/simion-oracle/check_fly2.py` (or `--root`),
`tools/simion-oracle/physics_validate.py` (or `--root`),
`tools/simion-oracle/quad_collision_probe.py` (or `--source`) and
`bench/default_flight.py` - read the same collection through
`SIMREP_ORACLE_CACHE` and exit with the expected layout when neither the
variable nor the explicit directory is given. The other scripts under
`tools/simion-oracle` still use their own historical default paths or
arguments; check each script's `--help` before running it.

```sh
export ORACLE=/path/to/physics-validation-20261023
export FIXTURES=/path/to/simrep-oracle
export MBMR=/path/to/collision_sds/mbmr.dat
export SIMREP_ORACLE_CACHE="$FIXTURES"

SIMREP_DEFAULT_SWEEP="$FIXTURES" cargo test --release -p simrep-fly \
  --test default_sweep einzel_step_control_sweep -- --ignored --nocapture
SIMREP_CPU_MAGNETIC_ORACLE="$ORACLE" cargo test --release -p simrep-fly \
  --test magnetic_performance magnetic_cpu_map_and_profile -- --ignored --nocapture
SIMREP_CPU_MAGNETIC_ORACLE="$ORACLE" cargo test --release -p simrep-fly \
  --test magnetic_performance magnetic_cpu_mag90_matched_precision -- --ignored --nocapture
SIMREP_PHYSICS_ORACLE="$ORACLE" cargo test --release -p simrep-fly \
  --test magnetic_performance magnetic_compute_and_convergence -- --ignored --nocapture
SIMREP_MBMR="$MBMR" cargo test --release -p simrep-fly \
  --test physics sds_runtime_icdf_transport_and_exact_drift -- --ignored --nocapture
```

## Ignored GPU oracle and benchmark tests

Use a physical Vulkan GPU. Native f64 and timestamp queries are required by
the tests whose ignore reasons name them. Copy fixtures into writable working
directories: some tests write reference CSVs or collision distributions.
Template layouts and capture recipes are in
[`bench/fly-gpu.md`](../../bench/fly-gpu.md) and
[`quad_README.txt`](../../crates/simrep-fly/src/gpu/quad_README.txt).
The latter's test invocations now also require `--ignored`.

```sh
export LIVE_TEMPLATES=/path/to/solved-app-templates
export BENCH_TEMPLATES=/path/to/solved-benchmark-templates
export QUAD=/path/to/quadrupole-reference
# ORACLE and MBMR are defined in the CPU oracle section above.

SIMREP_GPU_LIVE_BENCH="$LIVE_TEMPLATES" cargo test --release -p simrep-fly \
  --lib live_refly_benchmark -- --ignored --nocapture
SIMREP_GPU_BENCH="$BENCH_TEMPLATES" cargo test --release -p simrep-fly \
  --lib template_compute_benchmark -- --ignored --nocapture
SIMREP_GPU_KERNEL_BENCH="$LIVE_TEMPLATES" cargo test --release -p simrep-fly \
  --lib kernel_experiments -- --ignored --nocapture
SIMREP_GPU_PHYSICS="$ORACLE" cargo test --release -p simrep-fly \
  --lib gpu_physics_reference_agreement -- --ignored --nocapture
SIMREP_GPU_PHYSICS="$ORACLE" SIMREP_GPU_MBM="$MBMR" cargo test --release -p simrep-fly \
  --lib gpu_physics_collision_distributions -- --ignored --nocapture
SIMREP_GPU_PHYSICS="$ORACLE" SIMREP_GPU_TIMING=1 cargo test --release -p simrep-fly \
  --lib gpu_physics_compute_benchmark -- --ignored --nocapture
SIMREP_GPU_MBM="$MBMR" cargo test --release -p simrep-fly \
  --lib gpu_sds_exact_damped_rotation_and_diffusion -- --ignored --nocapture
SIMREP_GPU_QUAD_REFERENCE="$QUAD" cargo test --release -p simrep-fly \
  --lib quadrupole_grid_time_reference -- --ignored --nocapture
SIMREP_GPU_QUAD_RULES="$QUAD" cargo test --release -p simrep-fly \
  --lib quadrupole_collision_rule_agreement -- --ignored --nocapture
```

## Python extension and API

Python 3.10+, Rust, and venv support are required. This builds the actual PyO3
extension and CLI used by parity tests; the default empty Rust library does
not verify the Python API. See [`docs/python.md`](../python.md) for Windows
commands and `SIMREP_BIN` when the CLI uses a different output directory.

```sh
python3 -m venv /tmp/simrep-python
. /tmp/simrep-python/bin/activate
python -m pip install maturin numpy pytest scipy
maturin develop --release --manifest-path crates/simrep-py/Cargo.toml
cargo build --release -p simrep-cli
python -m pytest crates/simrep-py/tests -q
```

## CAD features

Install dynamically linked OCCT 7.6/7.7 development headers and libraries.
On Ubuntu, use the packages below. On Windows, use a matching MSVC x64 SDK,
set `OCCT_INCLUDE_DIR` and `OCCT_LIB_DIR`, and load the OCCT/third-party DLL
paths with the SDK's `env.bat` before testing. See
[`docs/step-import.md`](../step-import.md) and
[`docs/dev/windows-occt.md`](windows-occt.md) for the complete prerequisites.
Keep feature builds separate from default builds while CLI integration tests
are running so they cannot replace the same CLI executable.

```sh
sudo apt-get install --no-install-recommends \
  libocct-data-exchange-dev libocct-modeling-algorithms-dev
CARGO_TARGET_DIR=target/verify-cad cargo test --release -p simrep-cad --features occt
CARGO_TARGET_DIR=target/verify-cad cargo test --release -p simrep-cli --features cad
```

## Frontend and Tauri app

Node and npm are required for the frontend. Tauri additionally requires the
platform's native desktop build prerequisites (MSVC/WebView2 on Windows, or
GTK/WebKitGTK development libraries on Linux). The Tauri crate is a separate
workspace; see [`docs/dev/frontend.md`](frontend.md) for platform setup.

```sh
(cd app && npm ci && npm run typecheck && npm run build)
cargo test --release --manifest-path app/src-tauri/Cargo.toml
# With the OCCT SDK configured, also exercise the app's CAD feature:
cargo test --release --manifest-path app/src-tauri/Cargo.toml --features cad
```

## GPU on real hardware

Confirm `gpu-info` names a physical adapter, not `llvmpipe`/`Device type: Cpu`.
Run smoke checks and the relevant ignored GPU tests above on that adapter.
On a native Vulkan host:

```sh
cargo run --release -p simrep-cli -- gpu-info --backend vulkan
cargo run --release -p simrep-cli -- gpu-smoke --backend vulkan
cargo test --release -p simrep-fly --lib
cargo test --release -p simrep-solve
```

On this workstation, run Windows processes from WSL to reach the physical
GPU, keeping artifacts on a Windows drive and forwarding the Windows path
verbatim. Pick any Windows-visible directory for the build output and any
WSL path to a Windows `cargo.exe`:

```bash
export CARGO_TARGET_DIR='D:\simrep-target'   # any Windows path; /mnt/d/simrep-target in WSL
export CARGO_EXE='cargo.exe'                  # Windows PATH entry, or e.g. /mnt/c/Users/you/.cargo/bin/cargo.exe
export WSLENV="${WSLENV:+$WSLENV:}CARGO_TARGET_DIR"
manifest="$(wslpath -w "$PWD/Cargo.toml")"
target="$(wslpath "$CARGO_TARGET_DIR")"
(
  cd "$(dirname "$target")"
  set -eo pipefail
  timeout 900 "$CARGO_EXE" build --release \
    --manifest-path "$manifest" -p simrep-cli </dev/null 2>&1 | tr -d '\r'
  for backend in dx12 vulkan; do
    timeout 120 "$target/release/simrep.exe" \
      gpu-info --backend "$backend" </dev/null 2>&1 | tr -d '\r'
    timeout 120 "$target/release/simrep.exe" \
      gpu-smoke --backend "$backend" </dev/null 2>&1 | tr -d '\r'
  done
  timeout 1800 "$CARGO_EXE" test --release \
    --manifest-path "$manifest" -p simrep-fly -p simrep-solve </dev/null 2>&1 | tr -d '\r'
)
```

Follow [`docs/dev/windows-gpu.md`](windows-gpu.md) for `WSLENV` forwarding
of fixture variables when running the ignored tests and for process cleanup.
WSL software Vulkan success does not establish real GPU performance or
native f64 support on the target GPU.

## Published quantitative claims

```sh
python3 tools/check_claims.py
```

This standard-library-only check reads [`../claims.json`](../claims.json).
Its separate `readme` and `report_table13` lists register each published
number with an exact text fragment, source selector, derivation, and rounding
quantum. Report cells are selected from chapter 8's summary table in
`docs/report/manuscript.json`, not by JSON line number. The shared `sources`
map names the source file and section and extracts the underlying values
with Markdown regular expressions, CSV columns, or JSON selectors. The PDF
page count is counted from its uncompressed `/Type /Page` dictionaries.

The checker prints one PASS or FAIL per claim, with the derived value and
source, and exits nonzero on a mismatch, missing fragment/source, or
unregistered quantity. The evidence files those claims read (and every other
committed reference/fixture/answer-key file) are pinned by sha256 in
[`../validation/PROVENANCE.md`](../validation/PROVENANCE.md); run
`python3 tools/provenance_check.py` to verify them and
`python3 tools/provenance_check.py --add PATH` to register a new one. README prose is scanned for numbers with quantity
units (including both range endpoints); table 13 is scanned for every
numeric token. Code examples, links, scientific/product identifiers such as
`3D`, `4중극`, and `HS1`, list ordinals, and chapter/figure cross-references
are excluded. Additional bare counts, viewport widths, and the Python
version requirement are registered explicitly.

Rounding is decimal half-up at the registered power-of-ten quantum; a range
checks both endpoints. A literal upper-bound claim also checks the unrounded
source value. The earlier `6.4e-5 V 이내` wording failed this check because
the source maximum `6.423112427e-5 V` exceeded the literal bound. README and
table 13 now read `최대 6.42e-5 V 차이`, the rounded maximum, and the check
passes 58/58.

When a measurement or publication changes, update its source-backed claim
and rounding deliberately; do not copy the displayed number into a source
selector or widen rounding to hide a mismatch. This command verifies the
consistency of recorded evidence, not a fresh SIMION run, performance
measurement, installer build, or visual review.

## Frontend tests without a GPU

These run on any machine with Node 22+ (WSL here); no GPU, Rust toolchain or
native window is involved. Details and the demo-mode fixtures are in
[`docs/dev/frontend.md`](frontend.md) ("4. 검사").

```sh
cd app
npm ci
npm run typecheck                 # app + test/config sources (tsconfig.test.json)
npm test                          # vitest unit tests (src/**/*.test.ts)
npx playwright install chromium   # once per Playwright version
npm run e2e                       # Playwright smoke on demo mode (headless Chromium)
```

`npm test` covers the packet decoders (STR4 with its f64 end-state cases,
SPA1, SPQ1; `npm run check:flight` runs only that file, so it is not separate
evidence), analysis and CSV, plot helpers, refine progress and the
residual-chart model, the stores, i18n key/placeholder parity, the refusal
messages against every `Reason::new("…")` code in `app/src-tauri/src`, the
UI option lists against `src/scene/engineOptions.json`, and the demo fixtures.
`npm run e2e` serves the UI with `vite --mode demo` (mocked Tauri IPC
answering with packets recorded from the real native core) and clicks
through: start screen, einzel template, refine (live convergence chart),
fly, TOF histogram, end-state CSV export, dark/light theme
(`e2e/smoke.spec.ts`), plus refine completion and cancel-after-start
(`e2e/refine.spec.ts`). The tests step the recorded refine messages
themselves (`?replay=manual`) and await commands by count, so no assertion
depends on replay timing.

The option sync has a native half: `cargo test --manifest-path
app/src-tauri/Cargo.toml` (`options::tests`) proves every listed value is
accepted by the command that takes it and that `engineOptions.json` is
current. Both halves must pass; demo-mode tests do not exercise the Rust side.
