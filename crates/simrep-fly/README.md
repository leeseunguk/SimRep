# Particle flying

All public values use SIMION units: position and grid spacing in mm, absolute
time and TOF in microseconds, potential in V, E in V/mm, B in Gauss, mass in amu,
charge in elementary charges, and kinetic energy in eV. Positions, velocities,
fields and integration stages are f64. TOF is accumulated with Kahan summation
separately from the absolute birth time.

## Fields

`GridField::new(PotentialArray)` uses a **boundary-aware vacuum reconstruction**
when the array contains electrodes. Each axial line is split into vacuum runs,
including the first electrode node at either end as a Dirichlet value. Clamped
cubic splines estimate the shared nodal first and second derivatives. Their
boundary slopes use one-sided cubic fits on the vacuum side, not flat values
deeper inside the metal. Mixed derivatives average the directional orders at
corners. Tensor quintic Hermite patches join these shared nodal derivatives:
the potential is C2 and its analytic field `E = -grad V` is C1 across vacuum
cell faces. Polynomial coefficients are prepared once and evaluated by Horner's
rule. Derivatives use grid coordinates until the final physical-spacing map.

Electrode-free arrays retain the original **interpolating tensor cubic
B-spline**, including its high-order smooth-field behavior. Select
`GridField::with_reconstruction(array, Reconstruction::GlobalCubic)` to use that
method through metal as well, or `simrep fly --global-cubic` for comparisons.
Without a surface, both methods interpolate the input node potentials.
With an exact surface, `GridField::with_surface` or
`InstanceField::with_surface` first fills eight near-surface metal layers by
a conditioned cubic/quadratic/linear vacuum continuation, then uses the same
jets and Hermite assembly. Vacuum nodes and the original PA remain unchanged.
Constant electrode voltages come from flagged PA metal nodes; unavailable
voltages skip filling and are reported. Backgrounds and unit-voltage bases
are filled separately, preserving linearity. See
`docs/validation/CUT_GHOST_FILL.md` for band orders and flight measurements.

The global cubic needs four coefficients per active axis. Separable tridiagonal
prefiltering costs O(points) and stores `(nx+2)*(ny+2)*(nz+2)` coefficients
(single-point invariant axes are not padded). Open endpoints use slopes from
one-sided cubic sample fits (quadratic/linear for three/two points), giving exact
reproduction of cubics and third-order boundary slopes. Ordinary mirror planes
and the cylindrical axis have zero normal slope. Anti-mirror planes require
zero potential at the plane and use zero second derivative there, preserving C2
under odd reflection. Clamped/even and odd extension conditions are applied
during prefiltering, not by abruptly reversing a nonzero boundary field.

Planar `nz=1` arrays extend infinitely along z. Any single-point axis is
translationally invariant. Cylindrical arrays use SIMION's **x = axial,
y = radius** convention; the radial E component is rotated onto physical y/z,
with zero transverse field on the axis. Per-axis spacing, ordinary mirrors and
anti-mirrors come from the PA. Grid origins are zero; `InstanceField` adds a
workbench origin, orthonormal PA axis vectors and uniform instance scale.
Optional Cartesian workbench bounds permit zero-field drift outside the PA;
without bounds, exiting the PA terminates the flight. An optional oblique
detector uses `normal . position = offset`. Magnetic scalar potentials use
`physics::magnetic::MagneticPa` with the separately loaded PA-header `ng`.

`physics::magnetic::ImportedMagneticMap` reconstructs imported vector B on the
same C2 nodal jets as coil maps. Add maps to `MagneticField::maps` to superpose
them with PAs, coils and uniform B on CPU or GPU. `simrep fly --bfield-map`
reads regular-grid COMSOL, Opera, generic CSV/TXT and axisymmetric exports.
Units, placement, mirror parity, safe map-edge termination and convergence
controls are documented in `docs/bfield-import.md`. Unstructured point clouds
must be re-exported on a regular grid; there is no implicit resampling.

`AnalyticField(|position, absolute_time| FieldSample { ... })` supports arbitrary
analytic E and B, including constant B. `Superposition` sums
`voltage_i(absolute_time) * basis_i` for both V and E at every integrator stage.
Basis grids must have identical dimensions, spacings, symmetries and electrode
masks. Basis amplitudes are unit-voltage solutions: normalize external SIMION
solution arrays by their reference voltage before passing them in. A constant
voltage closure also represents a fixed background potential.
`Superposition::with_magnetic` supplies a uniform or position/time-dependent B.
The voltage and B closures must be thread safe for Rayon.

## Particles and integration

ION and executable FLY2 sources load through `simrep_io::read_particles`.
FLY2 sequences, distributions and custom Lua run in the shared bounded Lua 5.1
sandbox. PA-coordinate or field-dependent sources use
`read_particles_in(path, &Fly2Context)`; missing context is an explicit error,
never an implicit assumption that grid units are millimeters. The CLI supplies
its PA placement and initial fields. See `docs/fly2.md` for the source API,
constructor defaults, allowed read-only Lua includes and random seeding.

`Particle::new` accepts physical velocity in mm/us; `from_energy` normalizes the
direction and converts nonrelativistic eV to speed. For relativistic flights,
use `from_relativistic_energy` and `FlightOptions { relativistic: true, .. }`.
Both integrators evolve proper velocity gamma*v in that mode.

`fly_many_from_electrode` and GPU `fly_from_electrode` explicitly allow
surface emission from the initial contiguous metal run. After first reaching
vacuum, later electrode intersections absorb normally. Ordinary `fly` and
`fly_many` keep their original immediate start-in-metal splat.
`GridField::with_transparent_faces([xmin,xmax,ymin,ymax,zmin,zmax])` represents
ideal entrance/exit wire grids: their Dirichlet values still reconstruct the
field, but their face nodes do not absorb particles. This is needed for the
shipped TOF source exit and reflectron entrance, not its solid detector.

`WorkbenchField::new(instances,bounds)` supports separated placed arrays and
field-free drift. `Gpu::prepare_workbench` keeps all these arrays resident and
integrates the complete workbench in a single dispatch. The first containing
instance supplies the field in an overlap; collision semantics are intended
for non-overlapping instances.

`fly` integrates one particle. `fly_many` runs independent particles with Rayon
and preserves input order. The default is nonrelativistic Dormand-Prince 5(4)
RK45, with separate absolute position and physical-velocity tolerances and a
shared relative tolerance. Boris uses a fixed `initial_step_us`, clipped by
`max_step_us` and the final duration; its midpoint drift/rotate/drift update is
second order. It preserves speed in uniform B to roundoff, but its cyclotron
phase error remains second order: choose the step to resolve the gyroperiod.
`max_steps` bounds accepted plus rejected attempts.

RK45 no longer has a fixed default 0.1 us ceiling. `FlightOptions::default()`
and the CLI start with absolute/relative tolerances of 1e-12, and limit the trial step
to four smallest active-axis grid cells of travel, 0.2 radians of local
cyclotron rotation, and 0.2 radians of the fastest recognized voltage phase.
Grid spacing includes placement scale and the smallest grid in a workbench,
including field-free gaps. The transit estimate includes local electric
acceleration, so an ion starting at rest is also constrained. Relativistic
rotation uses the physical cyclotron frequency divided by gamma.

`StepLimits` exposes these three ceilings. `max_step_us = f64::MAX` selects
automatic control; setting another positive finite `max_step_us`, or passing
`--max-step-us`, selects the explicit ceiling instead. RK45 error control,
known time-event splits and collision/space-charge restrictions still apply.
Boris retains its fixed initial step and can only shorten it under these
ceilings. Parsed affine `sin`, `cos` and `exp` arguments provide smooth voltage
time scales; explicit `step`/`pulse` edges are split exactly. Arbitrary field
closures have no inferred spatial or voltage scale: custom `Field`
implementations can supply `spatial_scale_mm` and `time_scale_us`, or callers
can choose an explicit ceiling.

`automatic_tolerance` resolves a static electric grid's launch error budget
from the fourth forward difference of E along the initial direction over four
physical cells: `clamp(1e-3 * |delta4 E| / max|E|, 1e-12, 1e-8)`.
This bounded interpolation estimate avoids integrating a finite-grid field far
more precisely than its spatial reconstruction warrants. It vanishes for cubic
potentials. A zero stencil field, magnetic sample, known voltage timescale or
edge, missing grid scale, or launch from rest keeps 1e-12. Set
`automatic_tolerance: false`, any nondefault numeric tolerance, an explicit
`max_step_us`, or CLI `--tolerance` to retain exact configured tolerances.
`with_field_tolerance` exposes the resolved options for inspection.
When the grid estimate exceeds 1e-12, its absolute velocity budget is at least
that dimensionless estimate times the launch speed. This avoids forcing tiny
transverse velocity components to the same absolute numeric budget as positions
while retaining the position budget and the total launch-relative speed budget.

Dormand-Prince reuses its final derivative only when the next initial state and
absolute time match exactly. Rejected trials and geometry-event probes reuse
their common initial derivative; collision kicks and voltage-edge left limits
therefore cannot reuse a stale endpoint.
Endpoint stages and accepted states use the same compensated absolute-time sum
so floating-point association does not cause false cache misses.
The automatic ceiling shares that cached field sample rather than evaluating
the initial field again. Geometry and voltage-phase scales are cached per
flight; the next known voltage edge is refreshed only when reached.
Coupled space charge retains a separate one-cell
ceiling: its second-order push does not estimate Coulomb force error with RK45.

Staircase PA electrodes occupy cells whose corners are all electrode nodes:
four corners in planar/cylindrical 2D and eight in 3D. Single-point invariant
axes contribute only one corner. This matches the SIMION cell convention;
one isolated electrode node does not create a finite absorbing body.
Accepted step segments are partitioned at integer grid faces, mirror planes
and cylindrical shells, so a thin occupied cell cannot be skipped merely
because both endpoints are in vacuum. This geometry does not change the
input potentials or vacuum reconstruction.

For historical comparisons, `GridField::with_nearest_node_collisions()` and
`simrep fly --nearest-node-collisions` retain half-cell electrode voxels.
Apply the same convention to every voltage basis. Physical surface models
instead supply their own geometry; see `docs/pa-flight-geometry.md`.

`pub mod surface` supplies an additive continuous collision model. Load a v2
cuts sidecar using `SurfaceModel::from_reader`, or pass numbered in-memory
`SignedDistance` sources to `SurfaceModel::from_sdfs`; attach it using
`InstanceField::with_surface(Arc::new(model))`. Membership and first-segment hits
then replace PA conductor tests and prepare the surface-aware field and bases.
For in-memory sources, `.with_pa_domain(&array, source_origin_mm)` supplies
the PA clipping, mirror/revolution and source-origin rules without resampling.
V2 is **surface-accurate to the SDF sampling**, with conservative first-entry
tracing and bracketed root bisection on trilinear SDFs. V1 is reported as
**APPROXIMATE**: link distances and nodal signs reconstruct continuous
watertight cell interfaces, not uniquely recoverable source surfaces.
Mirrors, cylindrical revolution, source origins and instance placement are
applied before membership/roots; PA exits, bounds and detector events remain.
The existing flight event localizer re-integrates to the surface hit time.
`SurfaceElectrode { number }` is distinct from `Electrode { index }`, which
retains its original PA-node-index meaning.

The CLI loads a sibling `<stem>.cuts.json` automatically. `--cuts <file>`
overrides that path; `--no-cuts` opts out. Surface collisions force
`--device auto` to CPU, and explicit `--device gpu` errors. This does not
change the GPU staircase path. See `docs/geometry-pipeline.md` for the v2 schema,
sampling controls and accuracy measurements.

An optional Cartesian `StopPlane` detects a plane crossing in either direction.
Geometry events are localized by re-integrating shortened steps and bisecting
the first crossing, rather than linearly interpolating velocity or TOF.
Bisection stops at the requested absolute position tolerance divided by the
largest initial, final, or chord speed, or after 48 iterations.
Chord fractions first propose re-integrated times to narrow the bracket.
Every proposed bound is checked against the original step segment before use;
bisection remains the fallback.
Probes skip the unused embedded error estimate and retain the verified
high-bound state instead of integrating that same bound again.
Trial-stage fields extrapolate the endpoint polynomial beyond the array; only
accepted trajectories splat. A field's `sample` is therefore separate from its
`status` and `intersection` geometry methods.

Results contain final physical state, splat/termination reason, TOF, step counts,
and optional initial/every-N-steps/final trajectory samples. `TimeLimit`,
`StepLimit`, `StepUnderflow` and `NonFinite` are distinct from physical splats.

## Checks and benchmark

```sh
cargo test -p simrep-fly -- --nocapture
cargo run --release -p simrep-cli -- fly-bench --ions 10000
cargo run --release -p simrep-cli -- fly-bench --ions 100000
```

Tests cover cyclotron radius/period and 1000-orbit Boris energy, harmonic
frequency, parallel-plate TOF, relativistic acceleration, fast-adjust timing,
C1 grid fields, symmetry, electrode crossings and grid-flight convergence
against an analytic field. The convergence test reports RMS final-position
error across 17 launch phases to avoid a fortuitous cancellation at one spacing.
`fly-bench` uses a sampled synthetic cylindrical Gaussian einzel-like lens,
not the recorded SIMION einzel geometry; its timing ratio is not a claim of
matched-workload speedup or trajectory agreement with SIMION.

## Real SIMION arrays

The CLI loads SIMION-refined arrays without invoking our field solver:

```sh
target/release/simrep fly \
  --pa /path/to/einzel.pa0 --particles /path/to/einzel_10000.ion \
  --voltage 1=0,2=110,3=0 \
  --bounds=-101,-19,-19,291,19,19 --output splats.csv
```

`simrep-io::read_fast_adjust` applies voltage deltas using sibling `.paN`
solutions divided by the PA0 reference voltage, retaining unspecified voltages
and any background already in PA0. The static combined array is prefiltered
once rather than summing basis fields at each RK stage. `.ion` records retain
input order and use SIMION's azimuth toward -z and elevation toward +y.
The CLI also accepts the literal `standard_beam` FLY2 subset documented by
`simrep-io`; it does not execute Lua, binary legacy FLY, or binary IOB files.

Placement is explicit: `--origin x,y,z` gives the workbench location of grid
origin zero, `--axes xx,xy,xz,yx,yy,yz,zx,zy,zz` supplies PA unit axis vectors
column by column, and `--scale` multiplies all PA spacings. Defaults are zero
origin, identity axes, scale one. SIMION's nonzero `ox/oy/oz` working origin must
be folded into the supplied physical origin. `--plane 1,1,0,80` detects
`x+y=80 mm`. The CLI supports one electrostatic PA instance and does not
execute Lua programs. CPU magnetic PAs, Biot-Savart coils and
seeded HS1/SDS collisions are documented in
`docs/physics-magnetic-collision.md`; space charge has a separate API.
Additive CPU `--space-charge direct|pic --charge-mode factor|coulombic`
selects synchronized ensemble Boris integration rather than independent
flight. See `docs/physics-space-charge.md` for represented charges, CWFs,
PIC boundaries, event ordering, accuracy checks and Beam-mode limitations.

CSV records contain `ion,tof_us,x_mm,y_mm,z_mm,ke_ev,reason,electrode_number`.
The final column is populated for continuous-surface electrode splats and is
empty for staircase and other events. Relativistic
dynamics are enabled by default to match SIMION; `--nonrelativistic` opts out.
Timings on stderr separate setup, Rayon flight and total including CSV output.
Nonphysical terminations such as duration/step limits remain in the CSV and
make the command fail. See `bench/fly-real.md` for matched input measurements,
per-ion differences and the comparison's limits.

## Voltage expressions

`VoltageExpression::parse(source,&BTreeMap<String,f64>)` binds named parameters
once. The dependency-free parser accepts finite decimal/scientific constants,
`t` (absolute microseconds, including each ion's birth time), `pi`, `e`,
parentheses, unary `+ -`, binary `+ - * / ^`, and
`sin cos exp sqrt abs min max step pulse`. Power is right associative;
`-2^2` is `-(2^2)`. Trigonometric arguments are radians.
`step(t0)` is one for `t >= t0`, zero otherwise.
`pulse(t0,width)` is one on `[t0,t0+width)`. Their timing arguments must
be constant expressions or named parameters; width must be nonnegative.
Both integrators clip steps at these known edges, using the left limit for
an RK endpoint at an edge. Resolve smooth RF with a suitable maximum step.

Frequency in Hz needs `1e-6*t`, because `t` is in microseconds:

```sh
simrep fly --pa rf_mirror.pa0 --particles ions.ion \
  --voltage-expr '1=amplitude*cos(2*pi*rf_hz*1e-6*t)' \
  --voltage-expr '2=-amplitude*cos(2*pi*rf_hz*1e-6*t)' \
  --param amplitude=100 --param rf_hz=140000 \
  --origin=-1,-10,0 \
  --bounds=-1,-1,-1,401,81,1 --plane=1,0,0,20 --duration-us=300
```

`--voltage-expr` is repeatable and coexists with static `--voltage`.
An expression overrides a static setting for the same electrode. These
expressions set the electrode's absolute voltage, not a voltage delta.
`--run-file bench/fly-rf-run.json` reads the same expressions and parameters;
PA, particle and flight/placement options remain ordinary CLI arguments.
CLI expressions/parameters override their run-file counterparts. Unknown
run-file fields and unknown expression names fail rather than being ignored.

```json
{
  "parameters": {"rf_hz": 140000},
  "voltage_expr": {
    "1": "100*cos(2*pi*rf_hz*1e-6*t)",
    "2": "-100*cos(2*pi*rf_hz*1e-6*t)"
  }
}
```

For Rust, place unit bases with
`instance.with_voltage_expressions(vec![(unit_grid,expression)])`. CPU
evaluation is f64 at every RK stage, or at Boris's temporal/spatial midpoint.
GPU preparation compiles bound expressions into WGSL. It evaluates voltages
per ion at every stage, including birth time, and combines resident basis
coefficients there; a static field still uses the original once-per-change
combination pass. `PreparedField::set_voltage_expressions` changes selected
basis factors without uploading grids. `None` keeps the uniform static factor.
Native adapters exposing `SHADER_F64` use double field powers, stage states,
placement and expression time. Other adapters retain f32 expressions and a
separate two-float TOF accumulator; large absolute birth times can lose RF phase
on that fallback even though TOF remains accurate.
See `bench/fly-rf-tof.md` for waveform, real-array and timing evidence.

## Native GPU flyer

The Vulkan path selects native f64 when `SHADER_F64` is available.
`Gpu::has_f64()` reports the selected arithmetic. The f32 fallback remains
available without that feature. `Timings::fly` uses GPU pass timestamps when
`Gpu::has_timestamps()` is true, excluding field combination, uploads, driver
preparation and readback; otherwise it reports submission-through-completion.
The CPU API and GPU method signatures remain source-compatible. Precision and
large-beam compute acceptance results are recorded in `bench/fly-rf-tof.md`.

`gpu::Gpu::new(gpu::Backends::VULKAN)` creates a native compute context.
`gpu.prepare(&instance, &additional_bases)` uploads the instance's grid as the
first basis and any additional compatible `GridField`s. Keep the returned
`PreparedField` alive across flights. `set_voltages(&[1.0, v1, v2, ...])` updates
only the small voltage uniform; the resident arrays are combined on GPU before
the next flight. The first factor can instead be zero when no background is
wanted. Factors multiply the supplied arrays, so adjustable bases should be
unit-voltage solutions. At most 16 total bases are supported.

`PreparedField::fly` accepts the existing `Particle` and `FlightOptions` types
and returns ordered `FlightResult`s plus setup/upload/compute/collection timings.
`submit` returns a `PendingFlight` after enqueueing work; `pending.collect()`
waits and returns the same report. The blocking methods are thin submit/collect
wrappers. `submit_indexed`, `submit_sampled`, `submit_from_electrode` and
`submit_from_electrode_sampled` mirror their blocking counterparts. A pending
flight owns its birth-time/sampling metadata, parameter snapshot and scratch
buffers, so it does not borrow the input slices, options or `PreparedField`.
The original GPU and `InstanceField` must remain alive. Another request can
pack ions and submit changed voltages while an earlier flight is on the GPU:

```rust,ignore
prepared.set_voltages(&first_voltages)?;
let first = prepared.submit(&particles, &options)?;
prepared.set_voltages(&next_voltages)?;
let next = prepared.submit(&particles, &options)?;
let first_report = first.collect()?;
let next_report = next.collect()?;
```

Collection order does not change results. Physics submission enqueues the first
bounded trial pass for each batch; `collect` drives any remaining passes.
Dropping a pending flight discards its results, not already queued work.
Scratch buffers return to the prepared field's pool on collect or drop and are
reused in queue order. Limit outstanding requests to bound peak scratch memory.
Ion validation, coordinate conversion and per-species transport setup use Rayon
without reordering the input or changing the per-ion operations.

`PreparedField::gpu_memory_bytes()` reports actual allocated buffer bytes for
the field, including bases, combined powers, masks, uniforms, physics data and
pooled/outstanding flight scratch (also host-visible readback buffers). It
excludes CPU arrays, shared-device resources and opaque driver, pipeline and
query-set overhead. Query it again after caching powers or increasing flight,
trajectory or outstanding-request capacity; it is not an adapter VRAM-usage
query.

RK45 and Boris support electric and magnetic fields. GPU trajectories use
`fly_sampled` rather than the CPU `sample_every` option.
The public CPU API is unchanged. The CLI defaults to `--device auto`.
The electric-only calibration selects a hardware f64 GPU at 4,096 ions.
The measured field-free default-HS1 RK45 profile selects a native-f64 RTX 3070
at 10,000 ions:
50 us duration, 0.05 us maximum step, default initial step/tolerances, and no
PA, B, bounds, detector or space charge. Other magnetic/collision profiles
stay on CPU. `--device cpu` and `--device gpu` explicitly override the policy.
These thresholds use compute time, not initialization or process wall time.
See `bench/fly-gpu.md` for the controls and hardware limitations.

Each GPU invocation advances one ion. Physics flights keep position, velocity,
elapsed time, adaptive step, collision clock and RNG counter resident between
bounded 32-trial dispatches. Boundary localization finishes before yielding.
All dispatch timestamps contribute to compute time; progress readbacks do not.
Local position is an i32 cell index plus an
in-cell offset in the selected precision. Native stage state, velocity, time,
coordinate transforms and embedded error use f64. The adaptive controller's
power uses f32. The fallback retains compensated two-float TOF and split duration;
integer-mantissa differences protect compensation from reassociation. Workbench
origins are removed and final physical states reconstructed in f64.
Relativistic proper velocity is supported.

`gpu.prepare_physics(&electric, &bases, &magnetic, collision.as_ref(), seed)`
uploads the same magnetic scalar-PA reconstruction, placement, pole geometry,
uniform B and analytic Biot-Savart segments/hoops as the CPU models.
`PreparedField::set_magnetic_expressions([bx, by, bz])` adds analytic gauss
components as WGSL scalar expressions in `position` (absolute workbench mm)
and `time` (absolute us); for example `["0.0", "0.0", "100.0*cos(time)"]`.
Native mode promotes these expressions and their math to f64. Arbitrary Rust
closures must be expressed explicitly rather than copied to the device.
Automatic steps include the configured grid-transit, cyclotron and voltage-phase
ceilings; Boris keeps its requested fixed step and only shortens it.

Optional HS1 and SDS models run independently per ion. SDS uploads the runtime
`mbmr.dat` tables and per-species transport constants; no tables are embedded.
Its deterministic step retains exact damped rotation and midpoint step doubling.
HS1 retains the optical-depth clock and relative-speed-weighted gas sampling.
Philox4x32-10 uses the supplied seed, absolute input index and draw counter;
rejected trials and boundary localization consume no random draws. Internal
dispatch sizes do not change results. Use `fly_indexed(ions, options, first_index)`
when splitting one beam across API calls. CPU RNG sequences are unchanged;
CPU/GPU collision results agree statistically, not ion by ion.
Collisional flights remain nonrelativistic. See `docs/physics-magnetic-collision.md`
and `bench/fly-gpu.md` for the precision and compute acceptance evidence.

The GPU receives the CPU reconstruction's cell power coefficients in the
selected precision, rather than resampling the field. Boundary-aware quintics and global
cubics retain their CPU polynomial, mirror and cylindrical conventions.
GPU fast adjustment combines these coefficients once per voltage change,
not at every RK stage. This costs 6^d scalar coefficients per cell per boundary
basis, plus one combined array, in d active dimensions. Large fields must fit
the adapter's storage binding limit; oversized buffers return an error.
Electric flights default to at most 131,072 ions per dispatch on hardware,
clamped to the device's workgroup, storage-binding and buffer limits. Software
adapters retain the previous 32,768 default. Physics additionally retains its
4,096-ion watchdog bound. `max_ions_per_dispatch()` reports the effective limit;
`set_max_ions_per_dispatch(n)` selects a positive limit, clamped to device limits
and the physics bound. Use 32,768 to restore the previous partition on a device
that supports it. Larger populations retain input order across batches.
Flight compute time sums every batch's GPU timestamp; uploads and readback
remain separately reported. Submission upload timings cover host allocation
and enqueueing, without an upload-completion wait. On devices without timestamp
queries, the flight fallback includes elapsed time until collection, so a
delayed `collect` is not a compute-only measurement.
`readback` measures host mapped-result decoding and unmapping. GPU transfers
are queued with compute; overlapped wait time is not a separate additive timer.

The embedded RK error is formed directly from stage differences. Native relative
and absolute tolerances have a `1e-13` floor; the f32 fallback retains its `2e-7`
floor. The CLI prints the selected precision and compute clock. The same
nearest-node voxels, cylindrical shell/face partitions, accepted-step event
checks and shortened-step event localization are used as on CPU.

```sh
simrep fly --device gpu --backend vulkan \
  --pa /path/to/einzel.pa0 --particles /path/to/einzel_10000.ion \
  --voltage 1=0,2=110,3=0 --bounds=-101,-19,-19,291,19,19 \
  --output gpu.csv
cargo test -p simrep-fly --test gpu -- --nocapture
```

Vulkan is the GPU CLI default, including Windows. Tests identify software
adapters explicitly, skip only when no Vulkan adapter exists, and fail on
available-adapter errors. See `bench/fly-gpu.md` for measured accuracy, process
wall times, the unmet 10k startup budget, and Windows reproduction commands.
