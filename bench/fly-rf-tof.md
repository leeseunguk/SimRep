# RF expressions and the three-instance TOF workbench

Date: 2026-10-23. Ryzen 9 5950X, WSL2 CPU f64; native Windows
RTX 3070 Vulkan, wgpu 30.0.1; SIMION 2024-09-26-8.2.1.0.

Current CPU defaults use automatic grid/cyclotron/voltage-phase ceilings and
1e-12 tolerances. Default TOF endpoint/TOF RMS are 6.88959e-8 mm /
2.63312e-10 us at 0.0029872 s compute; cosine/square are 1.21090e-12 /
1.73137e-12 mm at 0.0009842 / 0.0008078 s. All improve the old default
accuracy and beat SIMION's compute. See `bench/default-flight.md` for the
measured default-only tables and the separate GPU integration boundary.

**Final acceptance uses GPU precision on all three cases and compute speed on
large beams, per the subsequent user decisions.**
Device creation, file loading, reconstruction, upload, output and process wall
are reference information, not speed gates. Native f64 passes all three accuracy
gates. The measured 100k/1M TOF GPU runs exceed 10x against the explicitly
extrapolated strict SIMION compute baseline below. Tiny beams use CPU under the
default automatic policy; there is no four-ion GPU speed requirement.

## Native precision study: compute-only acceptance

The Vulkan path requests `SHADER_F64` when available. The same shader template
and CPU cell polynomials are evaluated with native double state, field powers,
placement, stage time and embedded error. The adaptive controller's power is
still f32. Native double transcendental instructions are not supplied by the
SPIR-V GLSL instruction set, so the native path uses double arithmetic for
the RF sine/cosine reduction and polynomial evaluation. Unsupported adapters
retain the original f32 path and its explicit floor.

`Timings::fly` now uses GPU pass timestamps when available, excluding coefficient
combination, driver preparation, uploads and readback. A device without timestamp
queries explicitly reports submission-through-completion instead. All new
Windows rows below used timestamp queries on the RTX 3070.

An additional error source was a stale hardcoded electric-acceleration constant:
GPU `96.48533215665327`, CPU `96.48533202185007`. Shader generation now inserts
the shared CPU constant. It accounted for 3.75e-8 / 3.70e-8 mm of cosine/square
endpoint RMS and 9.85e-9 us of TOF RMS even after the other double conversions.

An unbatched precision-study snapshot at the original maximum step 0.02 us and
tolerance 1e-12 is retained below. Final RF validation reproduced the same
endpoint errors; its compute times were 0.043889 / 0.033424 s. These tiny-beam
GPU times are diagnostic, not acceptance gates.

| Case | Native f64 endpoint RMS mm | Native f64 TOF RMS us | GPU compute s | SIMION 209 compute s |
|---|---:|---:|---:|---:|
| TOF | 6.890e-8 | 1.155e-10 | 0.480848 | 0.12 |
| cosine | 2.339e-13 | prescribed duration | 0.043856 | 0.12 |
| square | 1.315e-13 | prescribed duration | 0.033384 | 0.07 |

Matching physical inputs and reference accuracy does not require matching the
integrators' maximum step. A 0.2 us cap gives the following accepted accuracy
and measured small-beam compute results. This historical tuning table is
reference information, not the revised large-beam acceptance test.

| Case | Requested tolerance | Endpoint RMS mm | TOF RMS us | GPU compute s | SIMION 209 compute s | Compute speedup |
|---|---:|---:|---:|---:|---:|---:|
| TOF | 1e-11 | 8.075e-8 | 2.674e-9 | 0.097329 | 0.12 | 1.23x |
| cosine | 1e-8 | 3.113e-9 | prescribed duration | 0.007003 | 0.12 | **17.14x** |
| square | 1e-8 | 2.843e-10 | prescribed duration | 0.005455 | 0.07 | **12.83x** |

The TOF endpoint/TOF targets remain 9.493e-8 mm / 3.31e-7 us.
Increasing its cap to 2 us gave 0.102467 s, 8.481e-8 mm and 2.486e-9 us,
not a speed improvement. No workgroup-per-ion cooperative kernel was built.

### Large-beam TOF acceptance

The larger beams repeat the original four physical particle templates, in
order, 25,000 / 250,000 times. They use the same placed arrays, surface emission,
transparent faces, relativistic dynamics, 0.2 us maximum step and 1e-11
tolerances. No new phase-space distribution is substituted.

| Ions | Measured GPU compute s | SIMION 209 compute s, extrapolated | Estimated compute speedup |
|---:|---:|---:|---:|
| 100,000 | **1.633020** | 3,000 | **1,837x** |
| 1,000,000 | **15.598810** | 30,000 | **1,923x** |

**The SIMION entries are estimates, not large-beam SIMION runs.** They assume
linear per-ion scaling from its measured 0.12 s for the original four ions:
`N * 0.12 / 4`. Running those strict SIMION populations was impractical; this
extrapolation is explicitly allowed by the revised acceptance rule.

Endpoint RMS remains **8.075e-8 mm** and TOF RMS **2.674e-9 us**, both below
SIMION 209. The benchmark verifies every copy against its first template:
100k and 1M both have zero RMS endpoint and TOF differences between replicas,
and numerical terminations make the run fail. Accepted/rejected totals are
16,850,000 / 9,900,000 and 168,500,000 / 99,000,000 respectively.

A monolithic 1M Windows dispatch failed with
`WrongSubmissionIndex(7, 6)`. The API now splits independent ions into at most
32,768 per dispatch, retains resident fields, and concatenates ordered results.
The reported compute is the sum of **all four / 31** GPU timestamp intervals,
not a per-particle extrapolation or CPU substitution. The batch/order regression
also covers the short final batch.

For reference only, 100k/1M process walls were 2.363 / 16.498 s; device
initialization 0.252 / 0.249 s; resident preparation 0.0195 / 0.0142 s;
upload 0.0030 / 0.0439 s; readback 0.0238 / 0.1538 s.

### TOF CPU/GPU crossover

Both backends use the same 0.2 us / 1e-11 accuracy settings:

| Ions | CPU compute ms | GPU compute ms |
|---:|---:|---:|
| 4 | 0.589 | 86.192 |
| 64 | 2.180 | 91.864 |
| 1,024 | 43.464 | 84.389 |
| 4,096 | 116.442 | 106.581 |
| 16,384 | 536.671 | 247.362 |

The first tested GPU win is 4,096 ions, agreeing with the CLI's einzel
calibration. Below that count the default auto policy selects CPU at full
precision. CPU endpoint RMS in this study is 7.686e-8 mm, also below SIMION 209.

### Measured error sources

Each entry below is RMS of the *difference of terminal states*, not a subtraction
of two RMS errors. These are controlled perturbations, not an additive variance
decomposition: field transitions change accepted/rejected steps. Position,
velocity, phase, evaluation and transform experiments use a matching 2e-7-floor
control; otherwise the f32 perturbations can prevent a 1e-12 flight from advancing.
Coefficient and compensated-TOF experiments use the unfloored native control.
The rounding studies shared the historical force constant, isolating it in the
last row. All comparisons retain the same particles and independent references.

| Perturbed source | TOF endpoint mm | TOF time us | Cosine endpoint mm | Square endpoint mm |
|---|---:|---:|---:|---:|
| tolerance floor 2e-7 | 4.800e-4 | 5.420e-5 | 0 | 0 |
| accepted fractional position rounded to f32 | 1.714e-6 | 4.078e-8 | 1.646e-6 | 1.010e-6 |
| accepted velocity rounded to f32 | 8.822e-5 | 2.624e-5 | 1.282e-6 | 7.272e-6 |
| stored polynomial coefficients rounded to f32 | 2.995e-7 | 1.523e-8 | 3.689e-7 | 3.622e-7 |
| f32 Horner / rounded combined coefficients | 1.308e-7 | 2.597e-7 | 7.275e-8 | 3.133e-8 |
| original compensated TOF and f32 step width, static field | 6.463e-9 | 4.800e-10 | not run | not run |
| absolute expression time rounded to f32 | 0 | 0 | 1.547e-8 | 1.563e-6 |
| cross-instance transforms in f32 | 1.067e-5 | 2.742e-7 | not applicable | not applicable |
| historical force constant | 2.535e-10 | 9.853e-9 | 3.746e-8 | 3.698e-8 |

The TOF tolerance floor dominates this experiment. Velocity accumulation is
material independently, particularly for square RF. The two-float TOF sum is
not the principal cause of the historical 6.33e-5 us TOF error. f32 coefficients
alone still exceed the TOF endpoint target, so double state with ordinary f32
field powers is insufficient. These studies do not separately isolate initial
charge/mass rounding or the original f32 RF transcendental implementation.

Two further speed experiments did not close TOF's gap: per-cell effective
polynomial degrees with coefficients below 1e-10 V discarded, and replacing
whole private parameter-block copies by per-vector resident loads. The trimmed
experiment retained TOF accuracy but still took 0.242541 s at the original
0.02 us cap; RF took 0.023667 / 0.013768 s there. Trimming is enabled only by
the diagnostic feature, not by normal native flight.

### Study reproduction and evidence

Build the Windows example using the foreground recipe later in this document,
adding `--features simrep-fly/precision-study`. Then run from `/mnt/c`:

```sh
p="$PWD/tools/simion-oracle/gpu_precision_study.py"
(cd /mnt/c && timeout 55 python3 "$p" \
  --native /mnt/c/Users/<user>/simrep-target/release/examples/rf_tof.exe \
  --mode native --mode constant < /dev/null)
(cd /mnt/c && timeout 55 python3 "$p" \
  --native /mnt/c/Users/<user>/simrep-target/release/examples/rf_tof.exe \
  --mode native --max-step 0.2 --tolerance 1e-11 --tag=-step02 < /dev/null)
(cd /mnt/c && timeout 55 python3 "$p" \
  --native /mnt/c/Users/<user>/simrep-target/release/examples/rf_tof.exe \
  --mode native --case rf_mirror --case rf_square_wave \
  --max-step 0.2 --tolerance 1e-8 --tag=-rf-speed < /dev/null)
```

`gpu-precision-{native,constant,all,tolerance,position,velocity,coefficients,phase,evaluation,transforms}.json`
under the existing `_rf_tof` directory retains argv, exit codes, stdout/stderr,
independent-reference differences and timestamp compute times. The static
compensated-clock study is `--mode tof --case tof`; mixing its f32 clock with
double RF edge coordinates is deliberately rejected rather than advertised as
a valid RF precision experiment. The full old f32 control reproduces the
historical 4.282e-4 / 2.573e-6 / 9.803e-6 mm results.

The native-double path was retained because it meets both the precision and
revised large-beam speed gates without compensated-arithmetic or cooperative
kernel complexity. The CLI now defaults to compute-calibrated automatic
selection: CPU below 4,096 ions, hardware f64 GPU at or above it, with CPU
fallback for unavailable/software/non-f64 adapters. See `fly-gpu.md` for the
measured crossover and the separate initialization-cost note.

Large-beam reproduction adds `--copies` to the study command:

```sh
(cd /mnt/c && timeout 45 python3 "$p" \
  --native /mnt/c/Users/<user>/simrep-target/release/examples/rf_tof.exe \
  --mode native --case tof --copies 250000 \
  --max-step 0.2 --tolerance 1e-11 --tag=-batched-1000000 < /dev/null)
```

The authoritative records are `gpu-precision-native-batched-{100000,1000000}.json`,
`{cpu,gpu}-precision-native-crossover-N.json`, and
`gpu-precision-native-final-rf.json` under `_rf_tof`. They preserve argv,
exit codes, raw output, reference errors, all replica checks and actual timings.

## Historical f32 baseline

**The DSL and validation are implemented. Phase 2's overall GPU
precision/speed gate is not satisfied by these measurements.** CPU accuracy
passes against our independent references. GPU TOF errors exceed SIMION's,
and none of these four/seven-ion GPU workloads establishes a matched-precision
10x speedup. The table below includes those failures, not just kernel timing.

## Implementation and interfaces

The handwritten Rust parser has no expression-engine dependency. It binds
finite named parameters, recognizes absolute `t` in **microseconds**, constants
`pi/e`, arithmetic `+ - * / ^`, parentheses, and
`sin/cos/exp/sqrt/abs/min/max/step/pulse`. Power is right associative and binds
more tightly than unary minus. `step(t0)` turns on at t0;
`pulse(t0,width)` is on over `[t0,t0+width)`. Timing arguments are constant
expressions/parameters and width is nonnegative, so both backends can clip
exactly at explicit edges. Unknown names, arities and invalid timing arguments
are errors.

`VoltageExpression` evaluates f64 on CPU and generates f32 WGSL expressions.
`InstanceField::with_voltage_expressions` adds unit-voltage bases to a fixed
background. GPU voltages are evaluated **per ion at every RK stage**, including
each ion's birth time, instead of being combined once for an entire flight.
CPU RK45 already used `t+c_i*h`; Boris already sampled at `t+h/2`.
Explicit step/pulse edges now clip integration steps and RK endpoint stages
use the left limit. Static GPU flying retains its once-per-change combination
pass; compile-time shader specialization removes unused workbench/dynamic
paths from that pipeline.

The CLI accepts repeated `--voltage-expr '2=...'`, `--param name=value`,
and `--run-file bench/fly-rf-run.json`. The JSON file contains `parameters`
and `voltage_expr` maps; PA, particles and placement remain CLI options.
CLI maps override file maps, and an expression overrides a static `--voltage`
for the same electrode. Voltages are absolute electrode values, not deltas.
JSON is buffered to avoid a syscall per byte on the Windows-mounted filesystem.
No Lua is executed by native flying. Existing public functions remain present.

`WorkbenchField` and `Gpu::prepare_workbench` support the separated TOF
instances, with zero-field drift between arrays. One GPU invocation integrates
an ion through the entire workbench; this is not a CPU-segmented GPU benchmark.
Overlapping-array collision priority is not validated here. GPU Boris and
internal trajectory recording remain unsupported; GPU trajectory validation
below uses separate terminal-state flights to the recorded sample times.

## Oracle setup and physical inputs

Installed examples remain read-only. `rf_tof_reference.py` makes working
copies under `%USERPROFILE%\simrep-oracle\_rf_tof`. It refines `.pa#` inputs
with SIMION's strict minimum `--convergence 1e-7`. Existing normal
`mirror.pa` and `detect.pa` are loaded unchanged.

`workbench_metadata.lua` reads the IOB through `simion.wb:load` and
`pa_to_wb_coords`. Loaded, fast-adjusted arrays are saved as separate
`loaded-N.pa` snapshots. This matters for the TOF source's actual 800 V
adjustment. Native inputs use those snapshots, not a guessed IOB binary layout
or our own coarse refine output.

TOF workbench bounds are `[-197,-22,-19]` to `[110,32,19]` mm.
The measured placements are:

| Instance | Nodes | Origin mm | Scale | PA x-axis / PA y-axis in workbench |
|---|---|---|---:|---|
| mirror | 100x20 cylindrical | approximately (0,0,0) | 1 | (1,0,0) / (0,1,0) |
| source | 25x20 cylindrical | (-40,-12.478,0) | 0.34327 | (cos 3 deg,sin 3 deg,0) / (-sin 3 deg,cos 3 deg,0) |
| detector | 3x20 cylindrical | (-191.7347940667,15.7058949717,0) | 0.65915 | (cos 3 deg,-sin 3 deg,0) / (sin 3 deg,cos 3 deg,0) |

All four original ions start at
`(-39.65685764004503,-12.460016670936735,0)` mm. Masses are
100, 100, 110, 110 amu; energies are 1, 51, 1, 51 eV.
The reference stores the actual SIMION physical velocities in f64, rather
than reconstructing the source's coordinate mode approximately.

Two geometry conventions are essential. These ions start on their source
electrode. Explicit `fly_many_from_electrode` / GPU `fly_from_electrode`
permit departure from the initial contiguous metal run, then absorb later
hits normally; ordinary `fly` behavior is unchanged. The source's xmax face
and mirror's xmin face are ideal transmitting grids, as described by the
example README. `with_transparent_faces` retains their Dirichlet field
values while removing absorption on those faces. The solid detector remains
absorbing. Omitting either convention produces a spurious immediate/source
exit splat.

### Original RF programs, not the unattached quad Lua

The shipped `8_rf_tests/rf_mirror.prg` computes
`V1=100*cos(2*pi*140000*t/1e6)`, `V2=-V1`.
`rf_square_wave.prg` uses +/-80 V according to the sign of the same cosine.
Frequency is 140 kHz, or **0.14 cycles/us**, not 140000 cycles/us.
The plates are 80 mm apart; the PA origin is `(-1,-10,0)` mm.
Both original seven-ion beams and their velocities are exported from SIMION.

The default legacy programs stop just after detecting the first peak, at
approximately half a period. Over that tested interval the exact DSL
square wave is `80*(1-2*step(1/(4*frequency)))`; the opposite electrode
has the negative expression. This one-edge expression is deliberately
limited to the shipped short test, not a claim of a periodic square wave
for `Long_Simulation_if_1=1`.

`readout.iob` has a different basename from the original PA, so the attached
readout-only workbench Lua does **not replace** the original per-PA legacy
voltage/termination programs. It records initial/final states and sparse
positions/velocities, without changing timesteps or forces. Separate,
uninstrumented flies of the original IOBs provide timings.

Native RF flights stop at the prescribed half-period,
3.571428571428571 us. SIMION's peak-detection termination differs by up to
1.85e-8 us at T.Qual 209. Thus RF TOF agreement is a **termination-protocol
comparison**, not an independently predicted detector TOF. RF position and
trajectory errors provide the integration accuracy check.

## Independent accuracy references

TOF surfaces are reconstructed at their original physical node coordinates.
A fine node is metal only when all corners of its containing original
rectangle are metal, preserving thin sheets and the node-defined geometry.
Fixed voltages are interpolated on those electrode surfaces; vacuum values
are **re-solved**, not resampled from an interpolated coarse solution.
Cylindrical solves transpose PA axial/radial order into the solver's r/z order.
References use relative residual 1e-13 and f64 flight tolerance 1e-13.

| TOF grid | Mirror / source / detector nodes | Endpoint RMS vs final 8x, mm |
|---:|---|---:|
| original, speed settings | 100x20 / 25x20 / 3x20 | 6.923e-8 |
| 1x, finer integration | 100x20 / 25x20 / 3x20 | 6.790e-8 |
| 2x | 199x39 / 49x39 / 5x39 | 1.572e-9 |
| 4x | 397x77 / 97x77 / 9x77 | 5.929e-10 |
| 8x, max step 0.00125 us | 793x153 / 193x153 / 17x153 | 3.479e-10 |
| final 8x, max step 0.000625 us | same | reference |

Halving the final 8x timestep changes TOF RMS by 7.10e-11 us.
These examples have nearly ideal linear fields, so their finite-grid
errors are much smaller than the einzel case in `fly-real.md`.

RF references use the same basis fields with four times smaller maximum
steps (0.005 vs 0.02 us), separately retaining each reference trajectory.
The endpoint RMS differences are 2.16e-13 mm for cosine and 1.31e-13 mm
for square. Independent analytic 80-mm plate closures differ from the
grid-field results by 2.94e-8 and 3.15e-8 mm RMS respectively.

| Case / backend | Endpoint RMS vs our reference, mm | TOF RMS, us | Sparse trajectory RMS, mm |
|---|---:|---:|---:|
| TOF CPU | 6.923e-8 | 3.50e-10 | 4.224e-8 |
| TOF GPU | 4.282e-4 | 6.33e-5 | 3.522e-3 |
| TOF SIMION 209 | 9.493e-8 | 3.31e-7 | 6.838e-6 |
| cosine CPU | 2.16e-13 | prescribed duration | 1.87e-13 |
| cosine GPU | 2.573e-6 | prescribed duration | 1.101e-6 |
| cosine SIMION 209 | 1.234e-6 | 1.16e-8 | 7.785e-7 |
| square CPU | 1.31e-13 | prescribed duration | 6.10e-14 |
| square GPU | 9.803e-6 | prescribed duration | 6.780e-6 |
| square SIMION 209 | 1.219e-6 | 1.37e-8 | 7.765e-7 |

There are 228 TOF and 98 samples per RF example, at the actual SIMION
accepted-step times near 0.25-us intervals. GPU max TOF-workbench endpoint
error is 8.249e-4 mm; maximum sparse trajectory error is 9.144e-3 mm.
All native TOF ions hit the detector; no duration or numerical failure was
counted as a detector hit. The GPU result is **not accuracy parity with
SIMION**, even though all terminal event types agree.

Terminal kinetic-energy RMS differences from SIMION 209 are
6.00e-6 / 0.0366 eV (CPU / GPU TOF),
9.22e-8 / 1.11e-7 eV (cosine), and
7.47e-7 / 8.38e-7 eV (square). These are end-state consistency checks,
not an assertion of kinetic-energy conservation in a time-dependent field.

For context, SIMION T.Qual 3 endpoint RMS values are 3.507e-7 mm (TOF),
1.991e-6 mm (cosine), and 1.203e-6 mm (square). Therefore switching the
speed denominator to SIMION's default does not repair the GPU precision gate.

### Analytic quadrupole and Mathieu checks

The analytic closure uses a Laplace quadrupole proportional to
`(a-2*q*cos(Omega*t))*(x*x-y*y)`, with normalization giving
`x''+(Omega^2/4)*(a-2*q*cos(Omega*t))*x=0`.
Two independent initial conditions construct the one-period monodromy
matrix. Its trace gives the Floquet exponent; stability requires
`abs(trace)<=2`. This avoids estimating frequency from a coarse FFT.

At a=0, q=0.01, measured beta is **0.007071205926312**, within 1.96e-5
relative of the small-q secular approximation `q/sqrt(2)`.
The a=0 first stability boundary is **q=0.9080463337433**, within 2e-8
of the Mathieu boundary value 0.9080463337. A second point gives
**b1(q=0.05)=0.9496894489508**, within 1e-8 of
`1-q-q^2/8+q^3/64`. Halving the integration step changes the checked
near-boundary trace by less than 1e-10. These tests pass on CPU f64
in both WSL and Windows.

## Serial matched-workload timings

Speed settings: original arrays, original 4/7/7 ions, relativistic RK45,
max step 0.02 us, requested tolerance 1e-12. GPU has its documented f32
2e-7 tolerance floor. Reference refinements and trajectory-validation
replays are excluded from timed rows. Measurements were serial.

GPU compute is host submission through completion, after the device,
pipeline and resident fields are initialized; it excludes upload/readback.
Process wall includes initialization, reconstruction, upload, output and
teardown. Each process-wall row creates a **new** device; driver caches
have been exercised. CPU compute includes its first Rayon launch.

| Case (ions) | CPU compute s | CPU process wall s | GPU compute s | GPU process wall s | SIMION 209 compute / wall s |
|---|---:|---:|---:|---:|---:|
| TOF (4) | 0.002617 | 0.017210 | 0.088485 | 0.400163 | 0.12 / 0.187 |
| cosine (7) | 0.003197 | 0.193605 | 0.026036 | 0.403682 | 0.12 / 0.188 |
| square (7) | 0.002447 | 0.192632 | 0.026197 | 0.394639 | 0.07 / 0.156 |

CPU is at least as accurate as the measured strict SIMION settings, with
about 46x / 38x / 29x compute speedups. Whole-process TOF is 10.9x faster;
whole-process RF is not faster because field loading/reconstruction dominates.
These are individual measured runs, not confidence intervals or a search for
the fastest intermediate SIMION T.Qual meeting each accuracy target.

Uninstrumented T.Qual 3 walls were 0.070 / 0.071 / 0.073 s; its reported
compute time rounds to 0.00 s. We do not divide by that rounded zero.
GPU timings are provided for diagnosis, **not advertised as matched-precision
speedups**, since their reference errors are worse than both SIMION settings.
Neither GPU 10x compute nor process-wall completion is established here.
No result is extrapolated to a larger ion population.

Warm GPU initialization is 0.200-0.218 s. Resident preparation is
0.0155 / 0.0301 / 0.0167 s; fly-specific upload is 0.00024-0.00031 s and
readback is 0.00284-0.00352 s. Cold specialized-shader process walls were
4.689 / 1.988 / 1.981 s; cold TOF device/pipeline initialization was
1.563 s and resident workbench preparation 2.457 s. Cold costs are retained
separately rather than hidden behind the warm table.

The remaining GPU issues are multi-instance/spatial field-transition
accuracy, low occupancy for tiny beams, and basis evaluation/setup cost.
This report does not establish which dominates a large-beam workload.
Absolute expression time is f32 on GPU; large birth times can additionally
lose RF phase even though the two-float TOF accumulator remains accurate.

## Reproduction and evidence

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo build --release -p simrep-cli --example rf_tof --bin simrep

# Foreground Windows oracle calls, copied examples only:
p="$PWD/tools/simion-oracle/rf_tof_reference.py"
(cd /mnt/c && timeout 55 python3 "$p" < /dev/null)

python3 tools/simion-oracle/rf_tof_measure.py \
  --native "$PWD/target/release/examples/rf_tof" --device cpu --grid-study

# Build using the Windows recipe in docs/dev/windows-gpu.md, then:
p="$PWD/tools/simion-oracle/rf_tof_measure.py"
(cd /mnt/c && timeout 55 python3 "$p" \
  --native /mnt/c/Users/<user>/simrep-target/release/examples/rf_tof.exe \
  --device gpu --output gpu-cold-specialized.json < /dev/null)
(cd /mnt/c && timeout 55 python3 "$p" \
  --native /mnt/c/Users/<user>/simrep-target/release/examples/rf_tof.exe \
  --device gpu --grid-study < /dev/null)

python3 tools/simion-oracle/rf_tof_report.py
```

`%USERPROFILE%\simrep-oracle\_rf_tof\` retains:

* `reference.json`: exact SIMION arguments, exit codes, process walls,
  metadata, high-precision readout and uninstrumented T.Qual 3/209 logs.
* Each case's `case.json`, `loaded-N.pa`, original `.prg`, strict basis
  solutions and separate readout IOB/Lua.
* `cpu-measurements.json`, `gpu-measurements.json`,
  `gpu-cold-specialized.json`: native argv, exit codes, stdout/stderr,
  per-ion endpoints and all native/reference sparse positions.
* `comparison.json`: max/RMS differences and the grid/timestep study.
* `cli-cpu.csv` / `cli-gpu.csv`: actual JSON-run-file CLI smoke flights
  to x=20 mm. Both terminate at the plane. CPU/GPU TOFs are
  3.219358588719 / 3.219358554459 us; y is
  60.86394146604 / 60.86394798756 mm.

WSL fmt, strict Clippy and the entire workspace test suite pass.
Windows release `cargo test -p simrep-fly --test rf` passes all three test
groups, including actual Vulkan expression evaluation, per-ion birth times,
narrow pulses, source launches, ideal grids and placed multiple instances.
WSL GPU tests use llvmpipe and are not counted as RTX 3070 performance.
The language-server daemon timed out twice; compiler and strict Clippy
diagnostics are clean, but no successful LSP result is claimed.

Final Windows command, run in the foreground with closed stdin:

```sh
s="$(wslpath -w tools/win/winproc.ps1)"; (cd /mnt/c && timeout 30 powershell.exe -NoProfile -NonInteractive -ExecutionPolicy Bypass -File "$s" cleanup < /dev/null)
```

Output (exit 0; no SIMION, Rust or simrep compute processes remain):

```text
== process counts ==

Name           Count
----           -----
cmd.exe            4
conhost.exe       33
node.exe           4
powershell.exe     1

== conhost by parent ==

Name                              Count
----                              -----
wsl.exe                              12
wslhost.exe                           4
cmd.exe                               4
node_repl.exe                         2
lmgrd.exe                             2
rlm.exe                               1
node.exe                              1
powershell.exe                        1
codex.exe                             1
sunshine.exe                          1
dispatcher.exe                        1
GoogleDriveFS.exe                     1
wlanext.exe                           1
matlab-mcp-server-windows-x64.exe     1

orphan conhost: 0
stopped orphan conhost: 0
```
