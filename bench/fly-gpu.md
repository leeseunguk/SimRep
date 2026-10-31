# GPU flying: accuracy and compute time

Date: 2026-10-23. Windows release Rust, wgpu 30.0.1 WGSL, Vulkan,
NVIDIA GeForce RTX 3070 8 GB, Ryzen 9 5950X. WSL validation uses Vulkan
llvmpipe, not the NVIDIA adapter.

## Native dispatch safety (2026-10-25)

Strict real-einzel 100k CLI replay exposed a 2.3 s single-dispatch driver reset.
Native f64 now defaults to 32,768 ions per dispatch; hardware Preview retains
131,072. The fixed 100k CLI compute median is 2460.147 ms, with byte-identical
results and a measured maximum 32,768-ion dispatch of 829.175 ms. This fixes
watchdog exposure, not the unreproduced historical 637.090 ms throughput.
See `docs/dev/windows-rtx-20261025/EINZEL-INVESTIGATION.md` for controls,
history limitations, failure reporting, validation and cleanup.

## Cached-power Horner specialization (2026-10-24)

**The quadrupole 100k Preview compute target is met: 54.886 ms**, versus
99.750 ms with the general kernel in the same interleaved replay. Native
Precise also improves, from 558.633 to 490.028 ms. These are Vulkan compute
pass timestamps, excluding combination, transfer, host setup and compilation.

`precision::shader` now emits the six x-Horner coefficient iterations
explicitly when `effective.x == 6`, retaining the original loop otherwise.
For this specialization, the coefficient values, Horner operation order,
reconstruction, RK45 stages, error budgets, geometry, localization count and
termination conditions are unchanged. No input/output layout, public API,
CPU code or app code changed.
The static electric path already combines all four resident basis factors
once per voltage update; it does not sum three voltage bases at each RK stage.

### Retained before/after controls

The new opt-in `gpu::kernel_benchmark::kernel_experiments` uses the same
solved templates, beam, integration options and 300 sampled trajectories as
`live_refly_benchmark`, with fixed factors `[1, 1.05, 0, 0]`. It compiles an
independent general-loop control, alternates candidate order, warms each
candidate five times per population and takes three measured flights.
`nvidia-smi` reported 0% GPU and memory utilization before the bounded runs.
Early clock-ramp samples were replayed with the longer warmup.

| Precision | Template | Ions | General compute ms | Unrolled compute ms |
| --- | --- | ---: | ---: | ---: |
| native f64 | einzel | 10,000 | 24.577984 | 23.697152 |
| native f64 | einzel | 100,000 | 201.912416 | 194.637760 |
| native f64 | quadrupole | 10,000 | 72.957376 | 59.522912 |
| native f64 | quadrupole | 100,000 | 558.632704 | 490.027552 |
| f32 Preview | einzel | 10,000 | 3.470048 | 2.334016 |
| f32 Preview | einzel | 100,000 | 19.766240 | 13.209216 |
| f32 Preview | quadrupole | 10,000 | 19.192544 | 9.940032 |
| f32 Preview | quadrupole | 100,000 | 99.749952 | 54.886016 |

Every replay compares **all final states and TOFs, exact hit classes and
indices, accepted/rejected step counts, and every recorded trajectory value
bitwise** against the general kernel, in both precisions. Thus this retained
Preview change does not consume even the existing summation-order noise
allowance. A final independent phase-profile replay gives
99.371072 -> 54.553312 ms at quadrupole 100k, with optimized samples
54.134208 / 54.939200 / 54.553312 ms. Earlier independent Preview replays
gave 55.069440 and 55.127968 ms. The separate duration correction below
changes only Preview's final time-clipped integration, not native output.

### Phase replay and generated shader evidence

`SIMREP_GPU_PHASES=1` additionally replays the accepted states from the 300
sampled trajectories through separately timestamped coordinate, field,
RK-stage, controller and segment-termination kernels. Each invocation repeats
eight times with a small coordinate perturbation to prevent invariant-code
elimination. The controller uses a finite error proxy rather than the
original embedded error. These are throughput microbenchmarks, **not additive
percentages of the full flight**: they exclude rejected/probe-state divergence
and differ in cache residency and register lifetime.

Quadrupole 100k trace medians, nanoseconds per replay call:

| Precision | Phase | General | Unrolled |
| --- | --- | ---: | ---: |
| f32 | coordinate/reflection | 0.083985 | 0.082613 |
| f32 | field evaluation | 2.197612 | 1.229861 |
| f32 | complete RK trial, including fields | 29.314670 | 12.449019 |
| f32 | step-controller proxy | 0.055441 | 0.055441 |
| f32 | segment termination | 1.875120 | 1.875943 |
| f64 | coordinate/reflection | 0.611120 | 0.615633 |
| f64 | field evaluation | 4.331346 | 3.528767 |
| f64 | complete RK trial, including fields | 61.275316 | 33.829922 |
| f64 | step-controller proxy | 2.915068 | 2.916358 |
| f64 | segment termination | 15.705829 | 15.704969 |

The Preview beam takes 4,854,995 accepted and 437,012 rejected steps; native
takes 6,191,780 and 1,515,084. Preview adds 27 terminal-localization probes per
ion; native retains its existing adaptive 27-49 probe bound. A full 3D field
evaluation reads 216 powers, and an RK trial evaluates
seven fields. Neither those counts nor the localization policy changes.
The field/RK replay improvement, with unchanged step counts, localizes the
measured win to evaluation rather than cheaper trajectories or early exits.

The benchmark also parses the actual generated WGSL with Naga, emits the fly
SPIR-V entry and counts local arrays and approximate linear SSA live ranges.
The final emitted modules have 30,078 -> 30,765 words (f32) and
32,408 -> 33,095 words (f64), including debug names and the other module
functions. The field/RK/fly linear SSA peaks are 15/62/20 f32 scalar slots
or 30/124/40 32-bit slots for f64, unchanged by unrolling. This counts struct
results but excludes local-variable storage, control-flow liveness and
cross-function inlining; it is **not the driver's register allocation**.
There are no local arrays in `instance_electric` or `rk45`. The 216-scalar
local expansion array belongs to `expand_cell` in the combination path;
the 16-scalar private voltage array serves dynamic fields.

Hardware occupancy, allocated registers, spills, f64-pipe utilization and
memory-stall counters remain **unmeasured**. Nsight was absent under
`C:\Program Files\NVIDIA Corporation`. The authorized user-scope install
returned exit 16, "No applicable installer found". The standard installer
downloaded and verified Nsight Compute 2026.3.1, but opened a secure UAC
prompt; the Windows token is not elevated. The task-owned winget process
and its child were terminated. Windows AppInfo's secure consent prompt
remained; attempting to terminate that prompt returned "Access is denied".
The user must Cancel the Nsight installation consent dialog. No elevated
process or unrelated earlier consent dialog was killed. To install later,
an elevated Windows terminal must run:

```powershell
winget install --id Nvidia.Nsight.Compute --exact --source winget --silent --accept-package-agreements --accept-source-agreements --disable-interactivity
```

Nsight Compute's profiling guide describes CUDA API interception, not Vulkan
shader capture. Installing it alone does not establish wgpu Vulkan kernel
counters. Nsight Systems can trace Vulkan, while Nsight Graphics shader
profiling is the relevant route for Vulkan shader counters; these were not
installed or captured here. The fallback evidence above must not be described
as measured occupancy or f64-pipeline utilization. See NVIDIA's
[Compute profiling guide](https://docs.nvidia.com/nsight-compute/ProfilingGuide/index.html)
and [Systems user guide](https://docs.nvidia.com/nsight-systems/UserGuide/index.html).

### Preview duration correction

The strict Preview collision KS replay exposed an existing compensated-clock
bug independently of Horner specialization: subtracting a two-float clock
from the target rounds the remaining interval to one f32. Rounding upward
could integrate beyond the target by 1.862645149e-9 us. Disabling unrolling
reproduced both complete Preview CSVs byte-for-byte, proving the bug preceded
the optimization.

Only the f32 shader now checks the proposed compensated end time of a
duration-clipped step. If it exceeds the budget, the positive step is reduced
to its predecessor f32 value. The remaining interval is then integrated
on the next iteration. The state therefore advances through the corrected
physical duration; neither the terminal clock nor the result is relabeled.
Native source generation does not include this correction.

The deterministic regression replays absolute ion index 324 from the seeded
HS1 fixture, which failed before the fix, and checks an analytic sampled
zero-field drift for clock/state agreement. Afterward every one of the
4096 HS1 and SDS Preview TOFs is exactly 50 us. The largest HS1 coordinate
change is 5.960464478e-8 mm; energy is unchanged. The SDS CSV is unchanged.
Both native CSVs are byte-identical to their pre-correction versions.

`physics_compare.py` and `physics_compare.py --preview` both pass without
changing the 1e-10 us prescribed-TOF assertion or the 5% exact KS gate.
For both precisions the minimum HS1 p-value is 0.0987631828 and the minimum
SDS p-value is 0.4313524026. This explicitly fixes the duration bug rather
than loosening a test to accommodate it.

### Rejected and unnecessary experiments

| Experiment | Result | Decision |
| --- | --- | --- |
| Six-coefficient cached-power x-Horner unroll | Full before/after table above; both precisions bitwise identical | Retain |
| Constant-index x-unroll in magnetic `nodal_value` / `nodal_gradient` | Sector 3dp first exact replay changed accepted steps from 719 to 711 | Reject; experimental code removed |
| Magnetic solenoid timing for that nodal candidate | Not run after its required native-bitwise gate failed | No candidate timing or crossover claim |
| Prior physics-free entry, finite guards, 128/256-lane groups and direct ion-member loads | Approximately 108-112 ms Preview quadrupole 100k, versus the retained general kernel near 100 ms | Previously rejected; not repeated |
| Per-stage precombination of fixed voltages | Already performed once per request by the existing combination pass | No duplicate implementation |

No f16 packing, integrator substitution, ion sorting/compaction or persistent
thread machinery was needed to meet the target. No magnetic nodal kernel change
survived, and no new matched CPU/GPU crossover was measured, so `--device auto`
keeps its existing calibrated thresholds. In particular, these electric
measurements do not establish a magnetic GPU crossover.

### Cancellation limit and reproduction

`PendingFlight` already owns submitted queue work. A new `Queue::write_buffer`
abort flag would execute after the preceding flight dispatch, not interrupt
it; a one-dispatch 100k electric flight cannot be cheaply canceled this way.
Dropping still discards results and suppresses future physics continuations,
but does not cancel submitted work. No misleading cancellation API was added.
Avoiding obsolete queued flights needs admission/deferred submission or
host-driven smaller continuations, which would be a separate scheduling
contract rather than an atomic flag added to the current API.

Reproduce using the Windows hygiene below, replacing the test filter with
`kernel_experiments`, setting `SIMREP_GPU_KERNEL_BENCH` to
`%USERPROFILE%\simrep-gui-test` and forwarding it through `WSLENV`.
Leave `SIMREP_GPU_PREVIEW` unset for native, or set it to `1` for Preview.
Set and forward `SIMREP_GPU_PHASES=1` for phase replay. No diagnostic feature
flag is required; the benchmark is an opt-in test and adds no production
runtime switch.

### Validation and final Windows cleanup

Formatting and both strict lint commands exit 0:
`cargo fmt --all -- --check`,
`cargo clippy --workspace --all-targets -- -D warnings`, and
`cargo clippy -p simrep-py --all-targets -- -D warnings`.
`cargo test --workspace --release` also exits 0: **251 tests pass**
(249 baseline tests, the opt-in kernel benchmark and the Preview duration
regression).
Language-server diagnostic requests timed out; compiler and clippy
diagnostics are clean.

Windows Vulkan executes **21 GPU-module tests plus the electric integration
test**, all passing after the duration correction. The deterministic duration
regression also passes on llvmpipe. Both precision variants of the magnetic
PA/coil/Boris fixtures pass. All seven native reference cases retain exactly
the endpoint/TOF maxima in the magnetic acceptance table below. Both fresh
4096-ion HS1/SDS precision replays pass the unchanged exact KS gates, and the
native collision CSVs remain byte-identical across the Preview correction.

Every Windows invocation used a `/mnt/c` cwd, foreground executable,
`timeout`, closed stdin, piped output and explicit `WSLENV`. Final
`tools/win/winproc.ps1 cleanup` exits 0:

```text
cmd.exe: 4, conhost.exe: 22, node.exe: 4, powershell.exe: 1
orphan conhost: 0
busy conhost (>50% of a core): 2
stopped orphan conhost: 0
```

No task-owned Cargo, Rust compiler, winget or GPU test process remains.
The two pre-existing busy WSL consoles were not killed. The privilege-blocked
Windows Installer/AppInfo consent described above still requires the user
to Cancel the Nsight installation dialog; access to terminate it was denied.

## Queued re-flight and dispatch capacity (2026-10-24)

The four throughput APIs are implemented, but the **70 ms quadrupole 100k
compute target was not met before the Horner specialization above**. The app's
explicit f32 Preview path improves from 120.064 to 99.627 ms; native f64 improves
from 632.249 to 556.881 ms. These are GPU timestamps, not process/IPC times.
The earlier frontend scratch result of 89 -> 65 ms preceded the physics merge
and is not reproduced by this revision.

### Public APIs and ownership

- `PreparedField::max_ions_per_dispatch()` reports the effective partition.
  `set_max_ions_per_dispatch(n)` accepts a positive requested cap and clamps it
  to the device's `max_compute_workgroups_per_dimension`, storage-binding and
  maximum-buffer limits. Hardware defaults to 131,072; software adapters retain
  32,768. Setting 32,768 restores the old partition where supported. Allocation
  rounding cannot exceed the device capacity. Physics retains its 4,096-ion
  bound and 32-trial continuation policy.
- Ion validation, coordinate/proper-velocity conversion, SDS transport setup
  and f32 packing use indexed Rayon iteration. Input order and individual
  arithmetic expressions are unchanged. Buffers are pooled, but initial ion
  states are still uploaded for each flight; no persistent-particle API is
  required.
- `submit(...) -> PendingFlight` enqueues without an upload/completion wait;
  `pending.collect()` waits, drives physics continuations and decodes results.
  Indexed, sampled and electrode-emission submit variants mirror the existing
  blocking methods, which now just submit and collect. Handles own birth-time
  and sampling metadata, independent scratch/readback buffers, uniform and
  instance snapshots, and the selected pipeline. They do not borrow
  `PreparedField`, input slices or options. The original `Gpu` and
  `InstanceField` must outlive them. Voltage changes, new pipeline options and
  out-of-order collection cannot alter previously submitted work.
- Collecting or dropping a handle returns its scratch buffers for queue-ordered
  reuse. Dropping does not cancel already submitted GPU work. Bound outstanding
  requests to bound peak memory. Physics handles initially submit one bounded
  pass per batch; collect must run to make further progress.
- `PreparedField::gpu_memory_bytes()` sums current field-buffer allocations plus
  all pooled/outstanding scratch allocations, including readback and timestamp
  resolve buffers. CPU arrays, device-global resources and opaque driver,
  pipeline and query-set overhead are excluded. Re-query after power caching or
  changing particle, sampling or outstanding-request capacity. Old field
  buffers retained by already submitted work during a cache/pipeline rebuild
  are not a device-wide memory-usage measurement.

The README includes a two-request voltage-update example. `Timings::upload`
now measures host allocation and enqueueing rather than waiting for transfer
completion; `readback` measures mapped-result decoding and unmapping, not a
separate GPU-transfer interval. GPU timestamp compute/combine definitions are
unchanged. Without
timestamp support, delayed collection contributes to the fallback elapsed
flight interval and must not be interpreted as compute-only speed.

### Matched short Windows controls

Windows release Rust, RTX 3070 Vulkan, 5950X; three identical-workload warmups
then three measured flights per population, with median GPU compute below.
The new opt-in `gpu::benchmark::live_refly_benchmark` loads
`simrep-gui-test/einzel/einzel.pa0` and
`simrep-gui-test/phase7b/quad/quadrupole.pa0`, including their three sibling
unit-voltage bases, and calls `cache_basis_powers`. Both populations use the
same 100 amu, +1 charge, 100 eV golden-angle disk of radius 3 mm, launch x=1 mm,
relativistic RK45, 1e-8 tolerances, 0.001 us initial step, 0.1 us ceiling and
100 us duration. Factors advance through `[1, 1 + run*0.01, 0, 0]`.
Each flight records 300 selected trajectories, every accepted step, capacity
400. The baseline uses the original serial setup and four sequential 32,768-ion
dispatches at 100k; after uses parallel setup and one 100k dispatch.

| Precision | Template | Ions | Before compute ms | After compute ms | Before/after host flight ms |
| --- | --- | ---: | ---: | ---: | ---: |
| native f64 | einzel | 10,000 | 26.727 | 26.729 | 30.641 / 30.463 |
| native f64 | einzel | 100,000 | 224.626 | 201.756 | 246.372 / 214.902 |
| native f64 | quadrupole | 10,000 | 72.412 | 72.429 | 79.825 / 79.502 |
| native f64 | quadrupole | 100,000 | 632.249 | 556.881 | 658.543 / 574.108 |
| f32 Preview | einzel | 10,000 | 3.194 | 3.200 | 6.850 / 7.273 |
| f32 Preview | einzel | 100,000 | 23.248 | 19.716 | 44.039 / 33.493 |
| f32 Preview | quadrupole | 10,000 | 19.041 | 19.102 | 24.413 / 24.728 |
| f32 Preview | quadrupole | 100,000 | 120.064 | 99.627 | 142.637 / 115.592 |

Host columns exclude field preparation and shader compilation and are not the
project's speed criterion. The 100k ion-setup medians decrease from
10.655/9.884 to 3.541/3.998 ms for native einzel/quadrupole, and from
11.216/10.885 to 5.037/5.303 ms for Preview. Fresh three-run 100k quadrupole
compute samples are 556.490/557.523/556.881 ms native and
99.972/99.568/99.627 ms Preview. No app code or IPC measurement changed.

Each final population also replays the same particles and sampling with the
old 32,768 partition. Both precisions match bitwise in final state, TOF,
termination, step counts and every trajectory sample. The production WGSL,
precision conversion and physics formulas are unchanged. There is no new
inter-ion summation or f32 summation-order change; the separate compact-basis
caching order differences documented below still apply.

The electric-only regression investigation compared a separate physics-free
entry, compile-time exclusion of automatic-step code, direct ion-member loads,
the old floating finite guard and 128/256-lane workgroups. Output controls
agreed, but quadrupole 100k Preview stayed about 108-112 ms, slower than the
retained general kernel's 99.627 ms. Those experimental shaders were removed.
The experiments do not establish register/occupancy or instruction-stall
counters; a measured kernel profile remains needed for the 70 ms target.

Reproduce from the repository root, following `docs/dev/windows-gpu.md`:

```bash
manifest="$(wslpath -w "$PWD/Cargo.toml")"
export CARGO_TARGET_DIR='%USERPROFILE%\simrep-target'
export SIMREP_GPU_LIVE_BENCH='%USERPROFILE%\simrep-gui-test'
export WSLENV=CARGO_TARGET_DIR:SIMREP_GPU_LIVE_BENCH:SIMREP_GPU_PREVIEW
set -o pipefail
cd /mnt/c
timeout 900 /mnt/c/Users/<user>/.cargo/bin/cargo.exe test \
  --manifest-path "$manifest" -p simrep-fly --release --lib live_refly_benchmark \
  -- --nocapture --test-threads=1 < /dev/null 2>&1 | tr -d '\r'
# Repeat with SIMREP_GPU_PREVIEW=1 for the explicit app precision.
```

### Validation and final cleanup

`cargo fmt --all -- --check`, workspace/all-targets clippy with `-D warnings`,
separate `simrep-py`/all-targets clippy with `-D warnings`, and
`cargo test --workspace --release` all exit 0: **231 tests passed** (227 prior
tests plus four additions). Linux GPU checks identify llvmpipe as software;
unavailable DX12 checks retain their explicit skips.

Windows Vulkan executes 19 GPU-module tests plus the electric integration
test, all passing. This includes Precise/Preview magnetic PA, coil and Boris
fixtures, native SDS drift/diffusion, Philox replay, one-/32-trial equivalence
and the new pending-flight ownership tests. All seven native reference cases
retain exactly the endpoint/TOF maxima in the magnetic acceptance table below.
The fresh 4096-ion HS1/SDS CSVs also pass `physics_compare.py` exact KS tests:
minimum p-values 0.0987632 / 0.431352. No physics formula or CPU result changed.

The language-server daemon timed out on symbol/diagnostic requests; compiler
and clippy diagnostics are clean. Every Windows run used a `/mnt/c` cwd,
`timeout`, closed stdin, piped output and explicit `WSLENV`. Final
`tools/win/winproc.ps1 cleanup` exits 0:

```text
cmd.exe: 4, conhost.exe: 22, node.exe: 4, powershell.exe: 1
orphan conhost: 0
busy conhost (>50% of a core): 2
stopped orphan conhost: 0
```

No task-owned Cargo, Rust compiler or flight process remains. The two busy
conhosts already existed before validation, have live WSL parents and were
left untouched.

## Magnetic fields and neutral gas (2026-10-24)

`--device gpu` accepts magnetic scalar PAs, uniform B, analytic coils, HS1
and SDS. RK45 and Boris retain their CPU meanings. Magnetic scalar potentials
use the same polynomial reconstruction, header ng, placement scaling, ordinary
and anti-mirrors, cylindrical expansion and all-corner pole geometry.
Small coil groups use the finite-wire and AGM elliptic-hoop formulas, with
cooperative workgroup staging of up to 128 coil records. Coaxial groups of
16 or more hoops additionally use a resident, calibrated C2 vector-field map.
Analytic support PAs gate the field
without adding a material or workbench wall. The configured cyclotron guard
uses physical velocity and divides the rotation frequency by relativistic gamma.

Large 3D magnetic PAs retain 27 shared derivative jets per node and directly
contract the same tensor quintic Hermite basis at each stage. This avoids the
216-power-per-cell cache: sector 3dp's 4,421,088,000-byte power array exceeds
the adapter's 2,147,483,644-byte storage binding limit. The compact path does
not resample the field or change its reconstruction, mirrors or pole geometry.

`Gpu::prepare_physics(&electric, &bases, &magnetic, collision.as_ref(), seed)`
retains these fields and uploads the runtime SDS ICDFs. HS1 freezes the event
rate at each accepted step's start, limits optical-depth subdivisions and
samples the same relative-speed-weighted thermal gas distribution. SDS uses
the existing host `Sds::transport` constants per ion and exact damped E/B
rotation with midpoint sampling and step doubling. Neither rejected RK trials
nor deterministic boundary localization advances the RNG. A diffusion jump
intersecting geometry terminates at its segment intersection and accepted
step end time.

Philox4x32-10 keys streams by the 64-bit seed and absolute ion index, with a
64-bit draw counter. Native uniforms have 52 bits; preview uses 23 bits.
`PreparedField::fly_indexed` preserves results across caller-controlled
batching. Internal dispatches also retain absolute indices. Coil dispatch
capacity is 4096 ions, rather than being divided by coil count. Every dispatch
contributes to the compute timestamp sum.
The CPU generator and CPU results are unchanged.

Physics work is also split after 32 deterministic trial attempts, outside
active boundary localization. Per-ion phase, elapsed time, next step, optical
depth, draw counter and trajectory progress remain in GPU storage. Finished
ions are not advanced again. A four-byte completion flag controls resubmission;
its readback and all progress-copy time are excluded from GPU compute timestamps.
Tests compare one-trial and 32-trial dispatches and caller-controlled batches.
This bounds long solenoid flights without truncating duration or changing RNG
draw order. Coordinate-frame handedness is preserved for Lorentz rotation,
including accepted reflected placements.

`PreparedField::set_magnetic_expressions([bx, by, bz])` additionally accepts
workbench-gauss scalar expressions in absolute `position` (mm) and `time` (us).
Native scalar math includes arithmetic, square roots, sin/cos, atan2,
exp/log and powers, evaluated in f64. Rust closures are not uploaded.
Use an explicit step ceiling for analytic time scales unknown to the flyer.
Native f64 remains the acceptance mode; f32 preview retains a 2e-7 tolerance
floor and is checked separately on the small analytic/PA/coil fixtures.
Those fixtures require component-wise endpoint and velocity agreement within
2e-4 mm and 2e-4 mm/us, and TOF within 2e-4 us, over flights up to 2 us.
This is not a long-flight error guarantee or the native magnetic acceptance gate.
The distribution replay can also select `SIMREP_GPU_PREVIEW=1`; its separate
`*-gpu-preview.csv` files are checked with `physics_compare.py --preview`.

### Replaying accuracy and compute controls

The opt-in `gpu::physics_benchmark` tests load captured `START` states and
placements from `SIMREP_GPU_PHYSICS`, pointing at
`physics-validation-20261023`. They do not use the stale saved CLI ion files
for sector 2dc or solenoid. Set `SIMREP_GPU_MBM` to the installed
`collision_sds/mbmr.dat`; the table is never copied into the repository.
`gpu_physics_reference_agreement` compares all seven magnetic cases to the
same 0.0025 us native reference and reports maximum endpoint and TOF errors.
`gpu_physics_collision_distributions` writes 4096-ion HS1/SDS CPU and GPU CSVs
under the oracle tree's `gpu-physics` directory. Run
`python crates/simrep-fly/src/gpu/physics_compare.py <oracle-root>` for exact
CPU/GPU two-sample KS statistics and p-values, with a 5% rejection gate.

### Native magnetic acceptance, 2026-10-24

The RTX 3070 Vulkan replay uses the captured starts and the same 0.0025 us
reference as the CPU acceptance study. Maximum endpoint distance and absolute
TOF error must not exceed either retained old-API envelope. All seven cases
also beat their recorded SIMION maximum errors.

| Case | GPU endpoint mm | Old-API gate mm | GPU TOF us | Old-API gate us |
| --- | ---: | ---: | ---: | ---: |
| magnet1 | 2.16069e-8 | 5.47282e-7 | 6.24548e-9 | 4.01298e-7 |
| mag90 | 5.25852e-9 | 2.54520e-7 | 4.98641e-11 | 2.31283e-9 |
| sector 2dp | 3.30103e-9 | 1.85622e-7 | 7.55094e-11 | 7.11352e-9 |
| sector 2dc | 2.50026e-9 | 3.01237e-7 | 2.47002e-11 | 3.69093e-9 |
| sector 3dp | 8.77655e-8 | 9.77063e-7 | 4.63992e-8 | 4.95515e-8 |
| Helmholtz | 3.52165e-11 | 1.46092e-8 | 6.51666e-14 | 2.69845e-11 |
| solenoid, mapped | 4.31337e-7 | 1.22619e-6 | 3.95693e-10 | 1.30382e-9 |

The independent 4096-ion CPU/GPU collision replay passes every exact KS
quantity at 5%: the minimum HS1 p-value is 0.0987632 and the minimum SDS
p-value is 0.431352. The full component/TOF/KE table and CSV provenance are in
`docs/physics-magnetic-collision.md`.

Physics uploads now contract 2D nodal jets as well as 3D jets. Component
strides, loop bounds and Hermite coordinates respect inactive axes; translated
planar Z cannot create a fictitious field. Direct off-node planar/cylindrical
B and scalar-value regressions compare with the CPU at 2e-10 tolerance.
The prior 2dp/2dc gates used expanded powers, so they did not exercise the
formerly 3D-only nodal helper.

The solenoid map stores axial B and Br/r, preserving the finite even radial
limit. One inner tube and two smooth fringe rectangles avoid the filament
strip; uncovered points retain exact Biot-Savart evaluation. The selected
0.0625 mm resolution uses 3,386,571 nodes and about 488 MB of f64 jets.
Each rectangle has 512 deterministic off-node CPU calibration probes:

| Region | Sampled maximum absolute G | Sampled maximum relative error |
| --- | ---: | ---: |
| inner tube | 1.35617e-9 | 2.71637e-10 |
| entrance fringe | 4.56768e-12 | 9.93005e-12 |
| exit fringe | 2.46887e-12 | 7.54584e-12 |

These are sampled field residuals, not a global analytic bound. The independent
full-flight endpoint/TOF gate above determines the accepted error budget.
The rejected 0.25 mm map had 2.15719e-4 mm / 1.96833e-7 us maximum errors.
Map construction, calibration and upload occur once per prepared configuration
and remain outside compute timing.

The solenoid's per-ion profile is not nine times more integration work: on
100 captured starts, GPU/CPU average accepted steps are 3625.75/3625.47,
rejected steps 51.08/49.80, and maximum accepted steps 13334/13331. Mapping
removes most coil summations, but retains seven-stage RK and f64 C2 field
evaluation. RTX 3070 (compute capability 8.6) has native f64 add/multiply
throughput only 1/64 of f32. A 4096-ion cohort launches just 64 workgroups of
64 lanes; each declares 12 KiB of shared coil staging and carries substantial
RK state, while the long step-count tail leaves divergent lanes. Those source
resources and limited parallelism constrain occupancy; achieved occupancy,
register spills and instruction-stall counters were not measured. Thus the
step counts identify arithmetic/occupancy, rather than excess steps, as the
next profiling targets before choosing f32/df64 or another integrator.

Hardware references: NVIDIA CUDA GPU Compute Capability table
(`https://developer.nvidia.com/cuda/gpus`), CUDA C++ Programming Guide 12.6,
Table 4 (`https://docs.nvidia.com/cuda/archive/12.6.0/cuda-c-programming-guide/index.html#arithmetic-instructions`),
and Ampere Tuning Guide, section 1.4.1.1
(`https://docs.nvidia.com/cuda/ampere-tuning-guide/index.html#occupancy`).

### Compute timing protocol

`gpu_physics_compute_benchmark` additionally requires `SIMREP_GPU_TIMING=1`.
`SIMREP_GPU_CASE=sector_3dp`, `solenoid` or `hs1` selects a
workload. The default populations are `1000,10000,100000,1000000`; each has one
identical untimed GPU warmup followed by three actual GPU measurements.
Use `SIMREP_GPU_COUNTS=1000,10000` for the bounded solenoid controls and
`1000,10000,100000` for sector; HS1 retains all four actual GPU populations.
CPU uses a preconstructed 16-worker pool, with actual three-run medians at
1k and 10k and one actual 100k run. The table below reports its deviation from
the measured 10k per-ion rate. Large CPU table entries are explicitly linear
extrapolations from that 10k rate, with the measured controls and 100k check
shown alongside them; no overnight 1M CPU run is implied.
HS1 uses the default 0.53 Pa, 273 K, 4 amu gas and 200 amu ions at 0.1 eV,
with no E/B field, a 50 us duration, RK45, 0.001 us initial step and a
0.05 us maximum step.
GPU values are timestamps summed over bounded dispatches, excluding setup,
uploads, combination, readback and shader compilation. The SIMION column is
explicitly a per-ion extrapolation of its retained reported compute interval,
not a measured large-beam SIMION run.

The approved runtime exceptions are explicit: sector 1M GPU uses its measured
100k rate, and solenoid 100k/1M GPU use its measured 10k rate. These are
projections, not actual three-run medians. A separate
`SIMREP_GPU_CPU_ONLY=1 SIMREP_GPU_COUNTS=100000` native-adapter replay skips GPU
field preparation and compute and takes only the single 100k CPU control. The solenoid control
has a 600 s process budget; an unfinished control is reported as unavailable
and replaced by a labeled 10k-rate projection.

Run native Windows release tests from a `/mnt/c` cwd, forwarding all these
variables and `CARGO_TARGET_DIR` through `WSLENV`, with closed stdin and a
foreground timeout as in `docs/dev/windows-gpu.md`. An explicit C-drive
`--target-dir` also prevents an environment-forwarding error from building on
the UNC worktree. Do not calibrate llvmpipe or another CPU adapter. Inspect
per-sample upload/readback times, avoid competing GPU jobs, and repeat a timing
interval if transfer times indicate contention. Preserve final
`tools/win/winproc.ps1 cleanup` output.

### Fresh native compute controls

These controls were re-taken after the Windows CPU-load cleanup, serially on
the RTX 3070 Vulkan native-f64 path. GPU and CPU entries are three-run medians;
CPU uses 16 preconstructed Rayon workers. Only identical compute intervals
are compared.

| Workload | Ions | GPU actual median s | CPU actual median s | Auto decision |
| --- | ---: | ---: | ---: | --- |
| sector 3dp | 1,000 | 0.895151840 | 0.118082800 | CPU |
| sector 3dp | 10,000 | 4.031644832 | 1.144121800 | CPU |
| solenoid, mapped | 1,000 | 52.297237440 | 6.056711600 | CPU |
| solenoid, mapped | 10,000 | 188.427607616 | 59.597406400 | CPU |
| HS1, field-free | 1,000 | 0.129540192 | 0.053331800 | CPU |
| HS1, field-free | 10,000 | 0.436820416 | 0.546073900 | GPU |

No GPU-winning magnetic-PA crossover is established by these controls.
The electric-only 4,096-ion calibration below is not applied to this class.
HS1's first measured GPU win is 10,000 ions, bracketed by the tested 1,000
and 10,000 controls, not an exact universal crossover. On the measured RTX
3070 with native f64, auto uses 10,000 only for the field-free, default-HS1
RK45 profile: no PA, B field, bounds, detector or space charge; 50 us duration,
0.05 us maximum step,
default initial step and tolerances, and default HS1 gas/collision parameters.
Other HS1 configurations, SDS and magnetic classes retain CPU under auto.

Large-beam GPU values below are actual three-run medians. CPU values in this
table are **linear extrapolations from the measured 10k median**, not actual
large-beam medians. SIMION values are also explicitly per-ion extrapolations
from its retained oracle interval.

| Workload | Ions | GPU actual median s | CPU extrapolated s | SIMION extrapolated s |
| --- | ---: | ---: | ---: | ---: |
| sector 3dp | 100,000 | 35.634338944 | 11.441218000 | 266.666666667 |
| HS1, field-free | 100,000 | 3.841686720 | 5.460739000 | 243.164062500 |
| HS1, field-free | 1,000,000 | 38.037001696 | 54.607390000 | 2431.640625000 |

The independent actual 100k CPU check tests the 10k-rate extrapolation:

| Workload | CPU actual single s | CPU extrapolated s | Deviation |
| --- | ---: | ---: | ---: |
| sector 3dp | 10.914289400 | 11.441218000 | -4.605529% |
| solenoid, mapped | 588.296251100 | 595.974064000 | -1.288280% |
| HS1, field-free | 5.221651200 | 5.460739000 | -4.378305% |

The solenoid control completed inside its 600 s budget (588.56 s test elapsed).
It is one actual run, not a median; the large-beam comparison tables retain
their explicitly labeled 10k-rate CPU projections.

The sector 1M interval was interrupted under the requested runtime escape
hatch so solenoid and HS1 could proceed. It produced one actual GPU sample,
353.111426368 s (upload 0.050759900 s, readback 0.202616200 s), **not a
three-run median**. The following 1M comparison is explicitly extrapolated:

| Workload | Ions | GPU linear projection from 100k s | CPU linear projection from 10k s | SIMION per-ion projection s |
| --- | ---: | ---: | ---: | ---: |
| sector 3dp | 1,000,000 | 356.343389440 | 114.412180000 | 2666.666666667 |

The single actual GPU sample is 0.907% below its projection. Neither the
projection nor that single sample establishes an automatic-device crossover.

Solenoid large-beam GPU entries are the approved **linear projections from
the actual 10k GPU median**, not measured 100k/1M runs. CPU projections
independently use the actual 10k CPU median.

| Workload | Ions | GPU projection from 10k s | CPU projection from 10k s | SIMION per-ion projection s |
| --- | ---: | ---: | ---: | ---: |
| solenoid, mapped | 100,000 | 1884.276076160 | 595.974064000 | 7010.000000000 |
| solenoid, mapped | 1,000,000 | 18842.760761600 | 5959.740640000 | 70100.000000000 |

The mapped solenoid remains CPU-favoured at both actual control populations:
GPU/CPU ratios are 8.63 at 1k and 3.16 at 10k. Auto therefore retains CPU,
irrespective of the projected large-beam timings.

### Verification snapshot

The final native Windows `cargo test --workspace --release` run completed
with exit code 0: 205 tests passed, none failed or were ignored, across 43 test
summaries including doc tests. `tests/step_control.rs` explicitly exercises
configured one-cell and four-cell transit/acceleration ceilings and the manual
step override, instead of assuming the default cell span. All nine tests in
that file pass. The existing DX12 f64 capability case is skipped because
`SHADER_F64` is unavailable there; Vulkan f64 checks pass.

The opt-in seven-case magnetic replay and the independent exact KS replay above
also passed. The CLI was exercised on native Vulkan with uniform B plus HS1 and
with uniform B plus SDS. A separate 10k-ion auto-routing smoke selected native
f64 GPU for the calibrated 50 us HS1 profile and CPU for its uncalibrated 100 us
variant. Those route checks are not additional timing samples.

`cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`
and `cargo clippy -p simrep-py --all-targets -- -D warnings` each completed with
exit code 0. The language-server diagnostics service timed out; compiler and
Clippy diagnostics are the available source checks.

### Final Windows cleanup, 2026-10-24

After the final checks, `tools/win/winproc.ps1 cleanup` and a broader scan for
worktree/release-path process candidates produced the following output. The
live counts include unrelated user processes and the cleanup PowerShell itself.

```text
== process counts ==

Name           Count
----           -----
cmd.exe            4
conhost.exe       22
node.exe           4
powershell.exe     2

== conhost by parent ==

Name                              Count
----                              -----
cmd.exe                               4
node_repl.exe                         2
powershell.exe                        2
wsl.exe                               2
wslhost.exe                           2
lmgrd.exe                             2
dispatcher.exe                        1
rlm.exe                               1
wlanext.exe                           1
sunshine.exe                          1
GoogleDriveFS.exe                     1
matlab-mcp-server-windows-x64.exe     1
node.exe                              1
codex.exe                             1

orphan conhost: 0
stopped orphan conhost: 0
task-owned process candidates: 0
```

## Compact 3D fields, resident results and GPU trajectories

**Coarse 3D accuracy is not yet an acceptance result.** The RF/TOF precision
gates below still pass, but the quadrupole study in this section does not
demonstrate a matched-precision speedup against SIMION.

The copied app templates are a 201x41x1 cylindrical einzel and a 121x37x37
quadrupole, both with 0.5 mm spacing. These are not the historical 91x20
einzel array used later in this document. The deterministic template beams
have 100 amu, +1 e, 100 eV, x=1 mm and velocity along +X. Einzel uses a
14 mm uniform-y line at 0/80/0 V. Quadrupole uses a 6 mm disk at
+2.5/-2.5/0 V: radius `3*sqrt(i/(N-1))`, angle
`i*2.399963229728653`. Explicit speed settings are relativistic RK45,
1e-8 absolute/relative tolerances, 0.1 us maximum step and 100 us duration.
The PA edge terminates the flight; no workbench drift is added.

### Storage and preparation

The quadrupole has 155,520 cells. Original quintic powers require 216
coefficients/cell: **268,738,560 bytes per f64 basis**, or 134,369,280 f32.
Four bases plus the combined cache cost 1,343,692,800 / 671,846,400 bytes.
The new static 3D representation retains 27 boundary-aware derivative jets
at each of 165,649 nodes: **35,780,184 / 17,890,092 bytes per basis**.
Four bases plus the unchanged cell-power cache occupy
**411,859,296 / 205,929,648 bytes**, excluding masks and particle buffers.
Basis storage falls 7.51x; total field storage falls 3.26x.

`GridField::new_compact` retains the same nodal derivatives without eagerly
materializing every CPU cell polynomial. CPU samples construct only their
cell's powers. Default `GridField::new` remains precomputed. First/second
derivatives share their spline solve, full-vacuum intervals avoid a duplicate
solve, and short compact builds use four workers. Existing CPU accuracy
tolerances are unchanged. GPU preparation borrows existing 2D powers, exports
3D jets instead of cloning the full power array, and parallelizes the remaining
2D degree scan. There is no 3D per-basis coefficient-degree rescan.

The measured quadrupole base plus three normalized unit bases took
**606.81 ms** to reconstruct, prepare and upload with all-corner geometry:
GPU preparation setup 109.59 ms and transfer completion 46.79 ms are included
in that total. A single adjusted basis took 155.74 ms CPU reconstruction,
27.51 ms preparation and 20.24 ms upload. Loading files and creating the
device are excluded. The first GPU cache expansion adds **14.88 ms**,
separately timed as `Timings::combine`: about 622 ms in total.
Preview preparation/first expansion were **536.60 / 9.91 ms**.

Voltage changes combine the resident jets and expand the same tensor Hermite
quintics on the GPU once, before flying. The RK kernel still evaluates cached
cell powers in f64; it does not substitute trilinear fields or ordinary f32
coefficients. Dynamic 3D expressions and multi-instance workbenches retain
power bases; changing an already prepared compact field to expressions
materializes its power bases once. Buffer-limit errors remain explicit.

Compact bases trade memory for update work: the precise cache expansion is
14.88 ms for four bases, versus approximately 1.32 ms for the single-power-basis
control. Preview expansion was 9.91 ms. Repeated-voltage applications can
explicitly call `PreparedField::cache_basis_powers()` once to retain GPU-expanded
power bases and return to the light combination pass. This keeps compact CPU
construction/upload but spends the original power-basis VRAM; it is not the
default storage policy. `Timings::combine` is separate from compute in either mode.
The measured four-basis cached mode used 1,074,954,240 / 537,477,120 bytes
for precise/preview bases, reduced subsequent combination to
**3.17 / 1.67 ms**, and took **1.47 / 1.04 s** of one-time preparation.

Direct Hermite evaluation from jets at every stage was tested and rejected:
100k precise quadrupole compute increased from 0.706 s to **4.215 s**.
Its smaller upload did not compensate for repeated weight arithmetic and
indexing. The retained design keeps that work out of the flight kernel.

### Kernel profile and precision

Cell lookup is a clamped integer index with reflection, not a spatial search.
A full 3D stage reads 216 scalar coefficients; one seven-stage RK attempt
reads 1,512, or 12,096 bytes in f64 before cache reuse. Rejected trials and
boundary localization repeat those loads. A 64-thread workgroup assigns one
ion to each invocation, so tiny beams are dominated by the slowest ion's
sequential trials rather than bulk bandwidth.

The native path formerly used 49 RK trials to localize every terminal hit
regardless of requested tolerance. It now selects 27-49 from displacement
and position tolerance with four guard bits. That logarithmic decision, like
the existing adaptive power controller, uses f32. Fields, state, phase,
transformations and embedded error remain f64.
This removed measurable compute work without weakening the RF precision gate.
Nsight Compute is not installed; hardware occupancy, register allocation and
spill counters were not measured. The 216-scalar private expansion array is
confined to the combination kernel, not the flight kernel.

The strict preserved-reference replay after this change returned:

| Case | Endpoint RMS mm | TOF RMS us |
|---|---:|---:|
| TOF | 6.890138e-8 | 1.155349e-10 |
| cosine | 2.338648e-13 | prescribed duration |
| square | 1.314909e-13 | prescribed duration |

These preserve the existing TOF 9.493e-8 mm / 3.31e-7 us limits and RF
criteria. Evidence is `_rf_tof/gpu-precision-native-compact-final.json`.
Explicit `Precision::Preview` remains f32 with the documented tolerance floor;
it is not the precision acceptance mode.

### Matched warmed compute controls (all-corner geometry)

All entries are measured GPU timestamps in milliseconds, summed over every
bounded dispatch. Each timed population is preceded by one identical untimed
flight on the same resident field. The control forces original power bases
and 49 native localization trials; result buffers are shared infrastructure
in both columns. Preview always uses its existing 27-trial localization.
This isolates the kernel/field changes, not process startup or transfer.

| Template | Ions | Precise control ms | Precise new ms | Preview control ms | Preview new ms |
|---|---:|---:|---:|---:|---:|
| einzel | 1,000 | 35.353 | 29.912 | 5.311 | 5.311 |
| einzel | 10,000 | 62.777 | 57.089 | 5.270 | 5.272 |
| einzel | 100,000 | 582.590 | 524.654 | 32.261 | 32.237 |
| quadrupole | 1,000 | 66.446 | 60.501 | 23.136 | 21.368 |
| quadrupole | 10,000 | 88.847 | 79.617 | 25.341 | 24.199 |
| quadrupole | 100,000 | 745.608 | 669.091 | 129.710 | 128.276 |

Precise 100k compute decreased 9.94% / 10.26%; preview 100k quadrupole
improved only 1.11%. These are individual measurements, not confidence
intervals. Unfixed clocks/first-dispatch variability is material for tiny beams:
unwarmed new einzel samples of 127/161 ms were observed at 1k/10k. The warmed
table is the resident interactive case, not a cold-latency promise.
These rows explicitly select all-corner cells ahead of the CPU default change.
Earlier nearest-node 100k precise controls/new values were
629.476/562.076 ms for einzel and 700.722/611.650 ms for quadrupole; those
use a different collision convention and are not substituted into this table.
An attempted all-corner run during other jobs had readbacks above 100 ms and
multi-second construction; its unstable timings are not used here.

The opt-in unit benchmark `gpu::benchmark::template_compute_benchmark` writes
matching ION files into `SIMREP_GPU_BENCH`, which must contain copied
`einzel/einzel.pa0` and `quadrupole.pa0` with their sibling bases.
Run native Windows release tests from `/mnt/c`, with closed stdin and timeout,
using `--all-features --lib template_compute_benchmark -- --nocapture`.
Forward `SIMREP_GPU_BENCH` through `WSLENV`. New precise is the default;
`SIMREP_GPU_PREVIEW=1` selects preview. The matched precise control adds
`SIMREP_GPU_POWERS=1 SIMREP_FLY_ROUND=localization`; preview control needs only
`SIMREP_GPU_POWERS=1`. Forward those variables too.
This table additionally uses `SIMREP_GPU_NODE_CELLS=1` in all four runs.

### All-corner cell collision migration

GPU preparation reads the field's selected collision rule; the CPU migration's
all-corner default therefore becomes the GPU default too. The CPU
`with_nearest_node_collisions()` option selects the legacy rule. There is no
independent GPU policy that can silently disagree with the supplied field.
An extra bit in each existing degree word marks cells whose 4/8 nontransparent
corners are all electrode nodes. No extra storage binding is needed.
All-corner traversal uses integer cell faces and cylindrical shells rather than
half-integer nearest-node faces, and reports the lower-corner electrode index.
Exact mirrored integer faces are canonicalized before cell lookup.

Native f64 CPU/GPU agreement on the same 1,000-ion quadrupole disk:

| Collision rule | Endpoint RMS / max mm | TOF RMS / max us | Matching reasons, including electrode indices |
|---|---:|---:|---:|
| nearest-node | 4.395365e-8 / 1.076588e-6 | 4.753251e-9 / 9.701578e-8 | 1,000/1,000 |
| all-corner cell | 4.660299e-8 / 1.076588e-6 | 7.774918e-9 / 1.160223e-7 | 1,000/1,000 |

Both f64 and preview tests also cover planar solids, ignored isolated nodes,
mirrors at exact integer faces, cylindrical shells and transparent corner nodes.
The optional `quadrupole_collision_rule_agreement` test writes separate
`rule-{nearest-node,node-cell}-{cpu,gpu}.csv` and agreement text files under
`_quad_gpu`, without overwriting the earlier reference evidence.
Set and forward `SIMREP_GPU_QUAD_RULES` to that directory.
The timing harness accepts `SIMREP_GPU_NODE_CELLS=1` to select all-corner
geometry before the CPU default migration and prints the actual selected rule.
The legacy accuracy snapshot below does not classify the new collision model;
its independent convergence study remains separate.

### Compact readback, sampling and one shared device

`PreparedField` grows and retains ion, terminal, sample and mapped-staging
buffers across flights and bounded batches. Terminal transfer now contains
only cell coordinates, offsets, velocity, split TOF and outcomes:
**96 bytes/ion f64, 64 bytes/ion f32**, half the previous 192/128 bytes.
Complete `FlightResult` fields remain available, including final velocity for
energy summaries. Timestamp data is additional fixed-size output.

`fly_sampled` and `fly_from_electrode_sampled` take `TrajectorySampling` with
strictly increasing zero-based ion indices, an accepted-step interval, and
`max_samples_per_ion`. They record initial, every Nth accepted and final states
on the GPU, including velocity and absolute-time reconstruction. Only selected
results have trajectories. Capacity exhaustion returns an error after unmapping;
it does not silently truncate. Keep `FlightOptions::sample_every=None` for this
API. Selection and order are preserved across the 32,768-ion batch boundary.
The tests cover both precisions, a short tail batch, overflow recovery and
unchanged terminal states with/without sampling.

Both GPU crates expose `device()` / `queue()` and accept cloned handles:
flyer's async `Gpu::from_device(device, queue, info)` and solver's synchronous
constructor with the same arguments. Flyer additionally exposes
`from_device_with_precision(..., Precision::Preview)` so an f32 preview pipeline
can share the solver's F64-enabled device. No second adapter/device is created.
`new_with_precision(backends, precision)` remains available for independent
creation.

Unbounded maximum steps are clamped to a finite duration cap before f32
conversion. The GPU automatic path also caps a step to one active grid-cell
transit; explicit finite step caps bypass that guard. The pending CPU
`StepLimits` configurable-grid/RF-phase integration point is marked in the GPU
parameter setup. Magnetic/cyclotron stepping is not part of this electrostatic
GPU flyer.

### Quadrupole SIMION check: legacy nearest-node accuracy snapshot

The same 1,000-ion disk was flown in headless SIMION using copied template
PA0/PA1/PA2/PA3, identity placement and bounds
`[0,-18,-18]` to `[60,18,18]` mm. An adapted single-instance IOB records
the checked transform `(1,0,0) gu -> (0.5,0,0) mm`.
Compute calibration disables recording and retained trajectories. Separate
readout runs collect 17-digit terminal states and are not speed measurements.

| Backend / quality | Measured 1k compute ms | Backend time / GPU time | Endpoint RMS vs 4x mm | TOF RMS vs 4x us |
|---|---:|---:|---:|---:|
| CPU f64, coarse | 29.55 | 0.547x | 2.02234 | 0.145534 |
| GPU f64, coarse | 54.04 | 1.000x | 2.02234 | 0.145534 |
| SIMION T.Qual 3 | 100 | 1.851x | 0.82164 | 0.058874 |
| SIMION T.Qual 209 | 9,280 | 171.728x | 0.79025 | 0.056608 |

This accuracy/time calibration precedes localization tuning; the warmed table
above gives the subsequent throughput. The raw time ratios here use the matching
54.04 ms calibration and are **not matched-precision speedups**. Coarse native
endpoints are worse than both SIMION runs against this provisional reference.
CPU/GPU coarse endpoint RMS difference is only 4.395e-8 mm, so GPU rounding
does not explain the discrepancy.

The reference independently re-solves 2x/4x Laplace arrays to relative residual
1e-12, using finer nodes fixed only when all their nonzero-weight coarse
corners are fixed. Grid spacing is 0.25/0.125 mm. Vacuum interpolation is only
the initial guess, not the reference solution. At 4x, halving maximum timestep
0.05 -> 0.025 us changes endpoints by **6.029e-10 mm RMS** and TOF by
2.489e-10 us. However **2x -> 4x still changes endpoints by 1.14527 mm RMS**:
the spatial reference is not converged. Therefore coarse 3D currently has
**no demonstrated matched-precision speedup**. No 8x run was made because its
memory would compete with other workers; reconstruction/reference work is
tracked separately.

The 3D small-beam crossover also differs from a blanket GPU rule: measured
100-ion CPU/GPU compute was **6.74 / 33.68 ms**, and 1k was **29.55 / 54.04 ms**.
These bracket no crossover; CPU is preferable at both counts. The historical
4,096-ion einzel threshold must not be treated as a calibrated 3D threshold.

Raw evidence lives under `%USERPROFILE%\simrep-oracle\_quad_gpu`:
`quality-{3,209}-{compute,readout}.json`, coarse CPU/GPU CSVs and
`reference-{2,4}-*.csv`. Its `scripts/README.txt` contains exact bounded
foreground commands; scripts also remain in `crates/simrep-fly/src/gpu/`.
`quad_beam.py` generates the disk, `quad_oracle.py` drives SIMION,
the optional Rust `quadrupole_grid_time_reference` test builds references,
and `quad_compare.py` reports errors without calling the reference converged.

### Verification of this iteration

WSL formatting check, all-target/all-feature workspace Clippy with
`-D warnings`, and `cargo test --workspace --release` exited 0.
Native Windows release library/GPU/RF suites passed 10 + 1 + 3 tests on the
RTX 3070, including sampling, shared devices, cached bases and both collision
rules. Separate opt-in quadrupole agreement and timing runs also exited 0.
The system OpenCascade headers still emit their pre-existing C++ iterator
deprecation warning; Rust diagnostics are clean. The LSP daemon timed out.
Final `winproc.ps1 cleanup` reported zero orphan conhosts and stopped zero;
no cargo, rustc or SIMION process remained in its final counts.

## Native precision study: compute-only criterion

The subsequent user decision makes **actual compute time** the speed criterion.
Preparation, initialization, loading, upload, output and process wall are
reference information only; the historical process-wall gates below are no
longer acceptance criteria.

The native Vulkan flyer now uses f64 when `SHADER_F64` is available, with f32
adaptive-controller power and the original f32 fallback on other adapters.
GPU pass timestamps measure flight alone, excluding field combination and
transfer. GPU precision passes TOF/cosine/square; large-beam TOF and einzel
compute pass the revised speed gate. See `fly-rf-tof.md` for the error budget,
shared-constant correction and the explicitly extrapolated TOF SIMION baseline.
Small GPU beams are not speed gates; automatic selection uses CPU there.

The exact preserved 1,000-ion subset and 8x reference were reused:

| Comparison | Endpoint max / RMS mm | TOF max / RMS us | Same physical wall |
|---|---:|---:|---:|
| native f64 vs independent 8x | 1.324228 / **0.225702** | -- | -- |
| native f64 vs preserved CPU coarse | 0.0009264 / 0.0001540 | 4.021e-5 / 7.937e-6 | 1,000/1,000 |
| historical f32 vs independent 8x | 1.327079 / **0.224607** | -- | 1,000/1,000 |

The new RMS is 0.49% above the historical f32 RMS and remains below the existing
0.248278 mm acceptance bound. It is not numerically identical to 0.224607 mm:
the more accurate integration approaches the preserved CPU coarse result,
0.225707 mm. The reference SHA-256 remains
`386c299949479f2572ca42cea8633868ed007184c2b9a488d0f5f86b7f46f30b`.

All new speed rows use the same particles, PA, voltages, relativistic dynamics
and default flight settings as the historical einzel benchmark. The f32 control
was rerun with the same timestamp clock rather than comparing different clocks.

| Ions | f32 GPU compute s | Native f64 GPU compute s | Slowdown | SIMION 209 compute s, extrapolated | Estimated native speedup |
|---:|---:|---:|---:|---:|---:|
| 100,000 | 0.039438 | **0.637090** | 16.15x | 769.531 | **1,208x** |
| 1,000,000 | 0.306004 | **6.263721** | 20.47x | 7,695.313 | **1,229x** |

Both native runs physically splat every ion. Every GPU time is measured, not
extrapolated: the API sums all bounded 32,768-ion dispatches (four / 31 batches).
This avoids the observed Windows failure of a monolithic 1M TOF dispatch.
Fields remain resident and output order is preserved.

**SIMION's large strict times are extrapolated, not measured.** An
uninstrumented T.Qual 209 calibration of 1,024 uniform-y ions reported
**7.88 s compute**. It used the same IOB, PA and physical beam distribution,
with recording and trajectory retention disabled. The estimates assume linear
per-ion scaling: `N * 7.88 / 1024`. The raw record is
`_real_einzel/einzel-strict-compute-1024.json`; no inputs changed during it.
`einzel_compute_reference.py` reproduces this bounded foreground calibration.
The revised rule explicitly allows extrapolation for impractical large runs.

```sh
python3 tools/simion-oracle/generate_einzel_beam.py \
  /mnt/c/Users/<user>/simrep-oracle/_real_einzel/auto-1024.ion --ions 1024
p="$PWD/tools/simion-oracle/einzel_compute_reference.py"
(cd /mnt/c && timeout 45 python3 "$p" \
  --particles /mnt/c/Users/<user>/simrep-oracle/_real_einzel/auto-1024.ion \
  --ions 1024 \
  --output /mnt/c/Users/<user>/simrep-oracle/_real_einzel/einzel-strict-compute-1024.json \
  < /dev/null)
```

For a separate conservative historical comparison, the directly measured
default-quality 100k SIMION compute is 21.37 s, still **33.54x** the new GPU
time; linearly scaling that looser baseline to 1M gives 213.7 s / **34.12x**.
Those figures are not substituted for the strict-precision acceptance column.
Relative to the historical f32 submission-clock values 0.025889 / 0.215431 s,
the new path is 24.61x / 29.08x slower. Those mixed-clock ratios are reference
information; the same-clock slowdown comparison is the table's 16.15x / 20.47x.

Preparation is retained separately for reference: native 100k initialization /
CLI setup / readback were 3.427541 / 3.520684 / 0.030487 s (cold shader cache);
native 1M were 0.231430 / 1.268623 / 0.082296 s. Setup includes initialization
and must not be added to it. They do not affect the compute comparison.

Evidence is retained under `_real_einzel` as
`fly-batched-native64-{100000,1000000,subset}.csv`,
`fly-batched-native64-differences.csv`, and
`fly-batched-f32-{100000,1000000}.csv`.
The existing `compare_gpu_fly.py` comparison passed without modifying either
preserved CPU/reference CSV.

### Automatic device selection

`simrep fly` defaults to `--device auto`. For the calibrated electric-only
class, below **4,096 ions** it selects CPU without initializing a GPU; at or
above that count it selects a hardware f64 GPU. Magnetic/collision classes
use their separate measured controls above; an uncalibrated or CPU-favoured
class stays on CPU. No adapter, a software adapter, or unavailable native f64 selects CPU.
Explicit `--device cpu` and `--device gpu` retain their requested paths.

The threshold is the first tested GPU win in a compute-only Windows einzel
calibration, using identical deterministic beams and default accuracy settings:

| Ions | CPU compute ms | Native f64 GPU compute ms |
|---:|---:|---:|
| 1,024 | 13.970 | 43.720 |
| 2,048 | 27.800 | 44.475 |
| 4,096 | 58.734 | 51.278 |

Thus the measured crossover is bracketed by 2,048 and 4,096 ions, not claimed
as an exact universal count. Field complexity and adapter hardware can move it.
GPU initialization in these runs cost 0.233-0.287 s. It is excluded from the
compute-based policy under the revised decision; a fresh-process wall-time
policy would favor CPU for more ions. The CLI prints the selected device and
threshold so that choice is observable.

Default-command verification selected CPU for 1,024 ions (14.053 ms) and GPU
for 4,096 (51.336 ms); all ions physically splatted in both cases. The
`auto-{1024,2048,4096}.ion`, `auto-{cpu,gpu}-N.csv` and
`auto-default-{1024,4096}.csv` artifacts are retained under `_real_einzel`.

The independent TOF-workbench crossover also first favored GPU at 4,096 ions:
CPU/GPU were 43.464 / 84.389 ms at 1,024, 116.442 / 106.581 ms at 4,096,
and 536.671 / 247.362 ms at 16,384. Its original four-ion beam uses CPU
(0.589 ms versus 86.192 ms GPU) with accuracy better than strict SIMION.
WSL end-to-end auto verification selected CPU for a software Vulkan adapter.

### Final validation

WSL `cargo fmt --all -- --check`,
`cargo clippy --workspace --all-targets --all-features -- -D warnings`, and
`cargo test --workspace` exited 0. Windows
`cargo test --release -p simrep-fly --all-features --test gpu --test rf -- --nocapture`
exited 0 on the RTX 3070. The checks include native RF precision, absolute
stage times, narrow pulses, two different static workbench backgrounds,
resident voltage changes, geometry and an ordered 32,768-ion batch plus tail.
The LSP daemon timed out; compiler/Clippy diagnostics are clean.

## Historical f32 measurements

**Accuracy passes. The 100k speed gate passes; the 10k process-wall gate does
not.** GPU endpoint RMS is **0.224607 mm** against the preserved independent
8x reference, versus CPU coarse **0.225707 mm** and SIMION **1.106314 mm**.
The 10k/100k/1M process walls are **0.34 / 0.46 / 1.94 s**. The requested
10k limit is 0.22 s; the 100k limit is 2.15 s. Phase 2 must not be declared
complete on the strength of kernel-only throughput.

## Historical f32 implementation and memory trade

`simrep-fly::gpu` is additive. The existing CPU types/functions and the default
`simrep fly` behavior remain source-compatible. Select `--device gpu`;
`--backend vulkan` is its default. The initial GPU API supports static
electrostatic RK45 terminal states, including relativistic proper velocity.
It explicitly rejects Boris or trajectory recording instead of silently
changing those requests.

One invocation integrates one ion. Position remains an i32 cell index plus
an f32 fractional offset in each local PA axis. Stage displacements are
relative to the start of a step; accepted updates carry whole cells into
the integer index. Planar voxel cuts are also computed relative to that
index. Placement origins are removed on CPU in f64, and final physical
positions and absolute birth-time-plus-TOF are reconstructed in f64.

TOF is a two-float sum, with a split high/low duration. An ordinary floating
`two_sum` was experimentally optimized to a zero error term by llvmpipe.
The shipped version computes the close-operand difference through integer
mantissas, preventing that reassociation. The regression test accumulates
thousands of 0.003 us steps and requires the final 10.123456789 us duration
within 2e-10 us, separately from a 1e12 us birth time. This test also passes
on the real NVIDIA Vulkan driver.

The GPU uploads the **same CPU-precomputed cell power polynomials**, rounded
to f32. Boundary-aware vacuum-ended derivatives, mixed derivatives and C2
quintic patches are not replaced by trilinear interpolation or another field
solve. Global cubic B-splines are converted algebraically to cell powers.
Horner evaluation differentiates those powers analytically.

A boundary basis costs `4 * 6^d` bytes/cell in d active dimensions: 144 B in
2D and 864 B in 3D. The 91x20 einzel has 1,710 cells and 61,560 coefficients:
246,240 bytes per basis. Background plus three adjustable bases occupy
984,960 bytes; their combined field adds 246,240 bytes, and the u32 electrode
mask adds 7,280 bytes. This is more memory than nodal splines, especially in
3D, but avoids rebuilding a different reconstruction on GPU and eliminates
repeated basis summation at all seven RK stages. Binding-size preflight
rejects fields that do not fit; this is not a streaming implementation.

`Gpu::prepare` retains all basis buffers. `PreparedField::set_voltages`
writes only the small uniform on the next flight, then a GPU pass combines
resident basis powers once per voltage change. Subsequent RK stages read
only the combined polynomial. The CLI obtains unit bases through the
existing `read_fast_adjust` zero/unit adjustments, preserving its PA0
background and normalization rules. No basis is re-uploaded when the API's
voltage factors change.

The RK embedded difference is accumulated from stage derivatives, not by
subtracting large completed positions. The physical-velocity difference is
also cancellation-resistant for relativistic motion. Relative and absolute
position/velocity tolerances have an explicit **2e-7 floor**. Geometry uses
the CPU's nearest-node electrode voxels, face/mirror/cylindrical-shell chord
partitions, first-event selection, field-free workbench volume, and
shortened-step reintegration. Localization uses 26 bisections, appropriate
to f32 step resolution, rather than the CPU's 48.

## Historical f32 1,000-ion accuracy acceptance

Inputs and placement are exactly those in `bench/fly-real.md`. In particular,
`_real_einzel/subset.ion` is reused in its existing order; this is not a newly
selected subset. CPU coarse is the preserved
`target/fly-real/boundary-original.csv`; the reference is the preserved
`target/fly-real/grid-8.csv`. Neither file was overwritten or regenerated.
The reference SHA-256 is:

```text
386c299949479f2572ca42cea8633868ed007184c2b9a488d0f5f86b7f46f30b
```

The speed-settings GPU run uses default maximum step 0.1 us and requested
`1e-8` tolerances (effective GPU floor 2e-7). The preserved CPU comparison
uses its original 0.05 us / 1e-11 settings.

| Comparison, 1,000 ions | Endpoint max / RMS mm | TOF max / RMS us |
|---|---:|---:|
| GPU default vs preserved 8x | 1.327079 / **0.224607** | -- |
| CPU coarse vs preserved 8x | 1.323463 / **0.225707** | 0.066292 / 0.010981 |
| SIMION T.Qual 209 vs preserved 8x | 4.281588 / **1.106314** | 0.200515 / 0.055319 |
| GPU default vs CPU coarse | **0.0124364 / 0.00204923** | **0.000547116 / 0.0000842939** |
| GPU 0.05 us / requested 1e-11 vs preserved 8x | 1.322597 / 0.227353 | -- |
| GPU 0.05 us / requested 1e-11 vs CPU coarse | 0.0108493 / 0.00233162 | 0.000518209 / 0.000105661 |

Both GPU settings pass the 10% threshold, **0.248278 mm RMS**, and are well
below SIMION. The small nonmonotonic RMS difference between GPU settings
does not establish greater f32 accuracy; finite-grid error and near-boundary
trajectory sensitivity dominate this comparison.

All 1,000 GPU ions terminate physically at workbench walls. CPU/GPU
termination reasons and nearest **specific wall faces agree 1,000/1,000**.
Per-ion endpoint/TOF errors and face labels are retained in
`%USERPROFILE%\simrep-oracle\_real_einzel\fly-gpu-differences-default.csv`
and `fly-gpu-differences.csv` (the 0.05 us run).

Reproduce without touching the reference:

```sh
P=/mnt/c/Users/<user>/simrep-oracle
R=$REPO/target/fly-real
python3 tools/simion-oracle/compare_gpu_fly.py \
  --cpu "$R/boundary-original.csv" \
  --gpu "$P/_real_einzel/fly-gpu-subset-default.csv" \
  --reference "$R/grid-8.csv" \
  --output "$P/_real_einzel/fly-gpu-differences-default.csv"
```

The comparison prints max/RMS, hashes the reference, emits every per-ion
difference, and fails for a numerical termination, a mismatched ion order,
an RMS over the 10% gate, or a changed wall face.

## Historical f32 Windows Vulkan release timing

Runs were serial with no other benchmark running. The original
`_benchfly/einzel_10000.ion` and `einzel_100000.ion` files were used.
The 1M file extends the same deterministic uniform-y beam formula and
physical parameters; no SIMION 1M timing was measured or extrapolated.
All rows use the same PA, `1=0,2=110,3=0`, relativistic dynamics, bounds,
and default GPU settings whose accuracy is reported above.

Driver shader caches had been exercised, but **each row is a new process,
new Vulkan device, new pipelines, and new uploads**. Process wall is
`/usr/bin/time` around the foreground Windows invocation, including process
startup, setup and exit. CSV output is on C:. CLI total ends after flushing
CSV and precedes device/field destruction and process exit.

| Ions | CLI setup s | Device/pipeline init s | GPU host setup s | Upload s | Fly s | Readback s | CLI total including CSV s | Process wall s |
|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| 10,000 | 0.253142 | 0.227148 | 0.001262 | 0.001998 | 0.007983 | 0.003012 | 0.296897 | **0.34** |
| 100,000 | 0.309837 | 0.226787 | 0.004252 | 0.002899 | 0.025889 | 0.005957 | 0.422523 | **0.46** |
| 1,000,000 | 1.036930 | 0.222972 | 0.050711 | 0.017041 | 0.215431 | 0.029288 | 1.877936 | **1.94** |

The GPU-specific setup/upload rows include both field and particle buffers.
Fly is host submission-through-completion wall time, including coefficient
combination, not a timestamp-only kernel duration. Readback includes download
and conversion to public f64 results. CLI setup includes PA/ION reading,
CPU reconstruction, device initialization and initial field upload; it
overlaps the GPU-specific columns and **must not be summed with them**.
CSV formatting/flushing and teardown account for the remaining wall time.

Accepted/rejected steps are respectively:

* 10k: 1,657,181 / 138,340; 10,000 physical splats.
* 100k: 16,570,699 / 1,382,891; 100,000 physical splats.
* 1M: 165,707,366 / 13,828,983; 1,000,000 physical splats.

Against recorded SIMION 2.201 / 21.504 s walls, process-wall speedups are
**6.47x / 46.75x**. Thus the 100k <=2.15 s gate passes comfortably and the
10k <=0.22 s gate fails even though flying itself takes only 8 ms.
This is the same terminal-only-output vs SIMION-retained-trajectories
comparison caveat described in `bench/fly-real.md`.

Temporary startup instrumentation, removed from the shipped code, measured
0.140904 s in `Instance::new`, 0.002985 s requesting the adapter,
0.066496 s requesting the device and 0.010048 s creating both pipelines.
That run took 0.220639 s just to initialize the GPU. After simplifying the
pipeline to one RK45 call site, first-run development initialization fell
from about 4 s to about 1 s; the first final CLI run still reported
1.011575 s initialization and 1.32 s process wall. These cold shader-cache
costs are not hidden in the table. A persistent `Gpu`/`PreparedField`
amortizes startup for interactive voltage changes, but that does not satisfy
the requested fresh-process 10k acceptance.

### Reproduction

Build from WSL with the Windows process in the foreground, closed stdin,
a timeout and C-drive working directory:

```sh
manifest="$(wslpath -w "$PWD/Cargo.toml")"
export CARGO_TARGET_DIR='%USERPROFILE%\simrep-target'
export WSLENV="${WSLENV:+$WSLENV:}CARGO_TARGET_DIR"
set -o pipefail
(cd /mnt/c && timeout 55 /mnt/c/Users/<user>/.cargo/bin/cargo.exe build \
  --release -p simrep-cli --manifest-path "$manifest" \
  < /dev/null 2>&1 | tr -d '\r')
```

Generate only the new million-ion beam:

```sh
python3 tools/simion-oracle/generate_einzel_beam.py \
  /mnt/c/Users/<user>/simrep-oracle/_real_einzel/fly-gpu-1000000.ion \
  --ions 1000000
```

For 10k/100k, use the original ION files; for 1M use the generated file:

```sh
ions=%USERPROFILE%/simrep-oracle/_benchfly/einzel_10000.ion
out=%USERPROFILE%/simrep-oracle/_real_einzel/fly-gpu-10000.csv
(cd /mnt/c && /usr/bin/time -f 'process_wall_s: %e' timeout 45 \
  /mnt/c/Users/<user>/simrep-target/release/simrep.exe fly \
  --device gpu --backend vulkan \
  --pa %USERPROFILE%/simrep-oracle/_benchfly/einzel.pa0 \
  --particles "$ions" --voltage 1=0,2=110,3=0 \
  --bounds=-101,-19,-19,291,19,19 --output "$out" \
  < /dev/null 2>&1 | tr -d '\r')
```

For accuracy, substitute `_real_einzel/subset.ion` and an unused output
path. The default run uses no extra options; the tighter maximum-step
comparison adds `--max-step-us 0.05 --tolerance 1e-11`.

## Validation

WSL commands all exited 0:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
RAYON_NUM_THREADS=16 cargo test --workspace -- --nocapture
```

The new GPU test actually executed on `llvmpipe (LLVM 20.1.2, 256 bits)`,
DeviceType Cpu. It covers global cubic and boundary-aware patches,
nearest-node electrodes without tunneling, cylindrical shell/mirror
geometry, field-free workbench drift, oblique detector planes,
rotated/scaled 3D boundary patches, a 1e9 mm workbench origin,
resident voltage changes and guarded two-float TOF. Existing workspace CPU
accuracy, IO, GPU multigrid and residual-smoke checks also passed.

Windows `cargo.exe test --release -p simrep-fly --test gpu -- --nocapture`
exited 0 and executed the same checks on `NVIDIA GeForce RTX 3070`,
DiscreteGpu. The Rust LSP daemon timed out twice; compiler and strict
Clippy diagnostics were clean.

The final Windows invocation was the required cleanup:

```sh
s="$(wslpath -w tools/win/winproc.ps1)"
(cd /mnt/c && timeout 30 powershell.exe -NoProfile -NonInteractive \
  -ExecutionPolicy Bypass -File "$s" cleanup < /dev/null)
```

It exited 0. Actual output:

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

No cargo.exe, rustc.exe, simrep.exe or task-owned GPU test process remained.
