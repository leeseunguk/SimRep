# Magnetic fields and neutral-gas collisions

The reference implementation is native Rust f64. It does not run Lua.
`physics::magnetic` supplies fields; `physics::collision` supplies independent
per-ion transport. Existing `fly`, `fly_many`, `Field`, `Particle` and
`FlightOptions` callers remain source-compatible. Space charge is separate.

Current flight defaults use automatic grid-transit, cyclotron and voltage-phase
ceilings with a 1e-12 magnetic tolerance, instead of a fixed 0.1 us ceiling.
Static electric grids can estimate a bounded 1e-12..1e-8 launch tolerance;
magnetic stencil samples retain 1e-12. Explicit
`--max-step-us` remains an override. Matched default-only reruns now beat
SIMION compute and the old API's maximum endpoint/TOF errors in all seven
magnetic cases; integrated-main sector 3dp takes 0.0080359 s versus SIMION's
0.04 s with the selected four-cell / 1e-8 static-electric cap.
`bench/default-flight.md` records the complete before/after tables, tolerance
selection, reference-input checks and unchanged HS1/SDS distributions.
`bench/magnetic-cpu.md` records the 2026-10-24 calibrated CPU coil-map path,
exact opt-outs, unchanged seven-case accuracy gates and fresh three/five-run
SIMION/CPU compute medians. All seven strict-default controls win after
removing repeated magnetic-PA coordinate and membership work.

## Native GPU transport

`simrep fly --device gpu --backend vulkan` accepts the magnetic PA, uniform-B,
coil, HS1 and SDS options described below, with either RK45 or Boris.
`Gpu::prepare_physics(electric, bases, magnetic, collision, seed)` uploads their
resident data. Magnetic PAs retain the CPU boundary-aware C2 scalar
reconstruction, ng/placement scaling, cylindrical expansion, scalar mirrors,
explicit polarity/sector switches and all-corner pole collisions. Separate
electric and magnetic placements and analytic support volumes are preserved.
Small analytic coil groups evaluate the finite-wire and AGM elliptic-hoop
formulas directly with shared workgroup records. Larger coaxial hoop groups
use a calibrated C2 axial/Br-over-r map and retain exact evaluation outside
its three filament-free rectangles. The selected 0.0625 mm solenoid map keeps
maximum endpoint/TOF error below the unchanged old-API envelopes; its sampled
field residuals and refinement rejection are recorded in `bench/fly-gpu.md`.

Additional analytic B components can be supplied through
`PreparedField::set_magnetic_expressions([bx, by, bz])`, as WGSL scalar
expressions using absolute workbench `position` in mm and absolute `time` in us.
The expressions return workbench gauss components. Native precision promotes
the expressions and transcendental math to f64. Rust closures are not device
programs; callers provide the corresponding expressions explicitly.
For unknown analytic time variation, choose an explicit maximum step.

HS1 uses the same collision rate, optical-depth subdivision, thermal rejection
sampling and COM scattering. SDS uploads the five validated mbmr ICDFs and
computes each ion's transport constants on the host using `Sds::transport`.
Only accepted deterministic steps consume RNG and apply kicks or diffusion.
Diffusion-segment intersections are localized at the accepted step's end time.
The exact damped E/B rotation and midpoint step doubling are retained, including
negative charge, gas drift and zero pressure.

GPU streams use Philox4x32-10 with a 64-bit seed key, a 64-bit absolute ion index
and a 64-bit draw counter. Uniform draws are open and use 52 bits in native f64,
23 bits in preview. Input order, rather than workgroup or internal batch number,
selects the stream. `PreparedField::fly_indexed(..., first_ion_index)` preserves
these streams across caller-controlled batching. CPU SplitMix64 and all CPU
physics results are unchanged. CPU/GPU acceptance therefore compares collision
distributions, not matching random trajectories.

Physics dispatches yield after 32 trial attempts, except while completing a
geometry localization. The resident ion state includes adaptive step, elapsed
time, optical depth and draw counter. Resumption therefore does not restart
transport or reinitialize an ion's random stream. One-trial/32-trial tests
also cover SDS transport constants and post-kick states across these boundaries.

Precise mode requires native f64 for the magnetic/reference acceptance runs;
the existing explicit `Precision::Preview` uses f32 and a 2e-7 tolerance floor.
Neither preview nor a software Vulkan adapter establishes RTX 3070 acceptance.
Compute timings exclude preparation, uploads, basis combination and readback,
and sum timestamps across every bounded dispatch. `bench/fly-gpu.md` records
the hardware replay procedure and measured results.
Automatic selection keeps magnetic and SDS profiles on CPU. The measured
field-free default-HS1 RK45 profile has a 10,000-ion native-f64 RTX 3070
threshold, restricted to 50 us duration, 0.05 us maximum step, default initial
step/tolerances and gas parameters, with no PA, B, bounds, detector or space
charge. Other HS1 configurations remain CPU under auto; explicit GPU remains
available.

### GPU distribution replay, 2026-10-24

Windows RTX 3070 Vulkan, native f64, 4096 CPU and 4096 GPU ions per model,
seed 813 on each implementation. Inputs match the 50 us HS1/SDS statistical
cases below. Exact two-sample KS tests do not reject a common distribution
at 5%. Prescribed 50 us TOF atoms are normalized only after checking every
reported TOF is within 1e-10 us of 50.

| Model | Quantity | CPU/GPU KS D | Exact p-value |
| --- | --- | ---: | ---: |
| HS1 | end X | 0.0270996 | 0.0987632 |
| HS1 | end Y | 0.0219727 | 0.276109 |
| HS1 | end Z | 0.0222168 | 0.264260 |
| HS1 | TOF | 0 | 1 |
| HS1 | KE | 0.0258789 | 0.128702 |
| SDS | end X | 0.0173340 | 0.569700 |
| SDS | end Y | 0.0131836 | 0.868785 |
| SDS | end Z | 0.0192871 | 0.431352 |
| SDS | TOF | 0 | 1 |
| SDS | KE | 0 | 1 |

CSV evidence is `physics-validation-20261023/gpu-physics/{hs1,sds}-{cpu,gpu}.csv`.
The `gpu_physics_collision_distributions` replay writes these files;
`crates/simrep-fly/src/gpu/physics_compare.py` checks and prints the full
statistics. Separate GPU tests cover the three Philox known-answer vectors,
seed/index reproducibility across batches, exact nondiffusive SDS E/B drift,
magnetic PA placement/poles, RK45/Boris and the cyclotron guard.

## Units and magnetic fields

External static B exports are supported by `simrep fly --bfield-map` and
`MagneticField::maps`. Regular Cartesian grids and axisymmetric z/r data use
the shared C2 component reconstruction on CPU and GPU, with explicit units,
placement and outside policies. See `docs/bfield-import.md` for COMSOL, Opera
and generic table layouts, scattered-data limitations and convergence tests.

Positions and PA spacings are mm, time is microseconds, E is V/mm, B is gauss,
current is A, mass is amu and charge is elementary charges. In these units
mu0/(4 pi) = 1 gauss mm/A.

`MagneticPa::new(array, ng)` requires magnetic PA metadata. `ng` comes from
`simrep_io::read_pa(path)?.ng`; it is not stored in `PotentialArray`. The field
is `B = -ng grad(phi)` in the PA's unscaled physical coordinates. Placement
rescales coordinates but does not rescale B. Axes rotate the field back to the
workbench. The electric and magnetic PA instances can have separate placements.
Magnetic pole bodies occupy grid cells whose corners are all pole nodes,
matching `simion.wb:find_at` probes and the electric flyer's staircase-PA
default. Their material boundary is at the pole node, not half a
cell into vacuum. A one-node-thick artificial Dirichlet mirror plane has no
material volume and does not splat ions; its values still constrain the field.

The scalar interpolation is the existing boundary-aware C2 reconstruction,
with ordinary scalar mirrors and explicit anti-mirrors, planar 2D/3D and
cylindrical x/r symmetry. Cylindrical radial B expands into workbench y/z.
Ordinary mirroring does not silently become magnetic anti-mirroring. The
explicit polarity switch in `sector_3dp.lua` is represented separately by
`with_polarity_flip_axis(0)`. The artificial sector aperture in
`sector_2dc.lua` is `with_angular_sector(0, 0, 90)`, whose angle is
`atan2(y,z)` modulo 360 about the workbench origin. Resolve these spatial
discontinuities with a suitable maximum step.

`Coil::segment` is the finite straight-wire Biot-Savart integral.
`Coil::hoop` uses complete elliptic integrals K/E evaluated by AGM, with the
analytic axial limit. `MagneticField::solenoid` sums N hoops at the centers of
N equal slices between `first` and `last`, matching `MField.solenoid_hoops`.
Currents follow the right-hand rule. The wires are infinitesimal filaments:
the field on a wire is nonfinite, not a finite conductor approximation.
An optional magnetic support PA gates analytic fields to the same volume
where a SIMION `mfield_adjust` override applies; leaving this volume is
field-free drift, not a workbench-wall collision.

`Electromagnetic { electric, magnetic }` sums fields and preserves geometry,
detectors and voltage edges. `AnalyticField` can supply any analytic E/B.
Both existing RK45 and Boris integrate E+B. Boris preserves speed in uniform
B but has second-order phase error; set `--step-us` to resolve gyration.

## HS1

`Hs1` implements the shipped `collision_hs1.lua` laws and its constants:

- Uniform background pressure, temperature, gas mass, cross section and bulk
  gas velocity; optional explicitly prescribed mean free path.
- Mean relative ion/gas speed from the Maxwell distribution, including a
  stable zero-speed limit. `nu = n sigma <|v-vgas|>` and
  `lambda = |v-ugas|/nu`; a stationary ion still has a nonzero thermal
  collision rate.
- Gaussian thermal gas velocities sampled with the HS1 relative-speed
  rejection weighting, including its approximate `v + 3 sqrt(3) sigma_v`
  rejection bound. This deliberately reproduces the example, rather than
  replacing it with an unweighted Maxwell sample.
- Elastic scattering in the center-of-mass frame, with a uniform outgoing
  relative direction. This is distribution-equivalent to HS1's
  `sqrt(U)` impact offset and uniform collision-plane azimuth.

The native event clock draws exponential optical depths `tau=-ln(U)`.
Each deterministic step is limited to
`min(tau, 1/steps_per_mean_free_time)/nu`, with 20 subdivisions by default.
Accepted steps subtract `nu*dt`; a zero remaining optical depth triggers a
collision and a fresh clock. The probability for a frozen rate over a step is
therefore exactly `1-exp(-nu*dt)`, consistent with HS1. Unlike the Lua endpoint
Bernoulli check, the native clock also localizes an event inside a longer
requested step. The rate is frozen at each accepted step's start; decrease
the maximum step or increase `--collision-steps` in strongly accelerating
fields. Rejected RK trials and geometry bisection consume no RNG draws.
The collision clock can shorten a step below the integrator's nominal
minimum; the usual floating-point time-underflow guard still applies.

`low_field_mobility` reports the first Chapman-Enskog/Mason-Schamp
hard-sphere limit:

```
K = 3 |q| / (16 n sigma) sqrt(2 pi / (mu kT))
vd = K E
```

K is m2/(V s), E in V/mm gives vd numerically in mm/us, and mu is reduced
ion/gas mass in kg. This is a low-field kinetic-theory approximation, not
an exact finite-field solution.

## SDS

SDS requires `DiffusionStatistics::from_path` / `--mbmr` pointing to the
installed example's `collision_sds/mbmr.dat`. The file is never embedded or
copied into this repository. It must contain five monotone 1002-point ICDFs,
including their zero origins. Semicolon comments are supported.

The implementation follows `collision_sds.lua`, not Gaussian diffusion:
choose a percentile over the first 1000 intervals, linearly interpolate each
of the neighboring mass-ratio ICDFs, then interpolate distance in log/log
mass-ratio space. The five ratios are 1, 10, 100, 1000, 10000; outside that
range the nearest pair extrapolates as in the Lua. Each accepted step adds an
isotropic jump with radius

```
r = L * ICDF(log10(mion/mgas), U) * sqrt((Vthermal/L)*dt / 100000)
```

Temperature/pressure scaling, thermal mean speed, mean free path, empirical
diameter/mobility estimates and optional supplied ion diameter/reduced
mobility use the example's constants and equations. `--mobility-stp` is in
1e-4 m2/(V s), at 273.15 K and 760 torr. This is independent of the HS1
cross-section option. Bulk gas velocity is in mm/us in the native API/CLI.
The example's optional `m_defs.dat` is not loaded implicitly: supply its
diameter/mobility values explicitly for a calibrated species, or use separate
Rust `Sds` models for species with different calibrated entries.

Deterministic Stokes drag is integrated with the exact constant-E/B damped
rotation and displacement. Field variation uses midpoint field sampling and
step doubling; the full/half-step comparison controls the local error. This
avoids stiffness restrictions at atmospheric pressure and converges to the
same continuous drag law as the example's finite-step acceleration damping.
The RNG jump is applied only after accepting that deterministic step; it is
not divided across trial steps. A jump crossing a pole, electrode or detector
is stopped at the segment intersection, at the accepted step's end time.
SDS models thermal diffusion in position, not Maxwell-randomized ion velocity;
its reported KE therefore must not be interpreted as HS1 thermal KE.

All collisional runs are nonrelativistic. The CLI selects that automatically;
the Rust collision entry points reject relativistic options. SDS uses the
absolute charge for positive drag also on negative ions (the shipped Lua
formula otherwise produces negative damping). Neutral ions are accepted by
HS1, but SDS mobility requires nonzero charge. Zero pressure disables both
models' effects.

SplitMix64 uses an avalanched per-ion seed keyed by input index, 52-bit open
uniforms, polar Box-Muller thermal velocities and isotropic directions.
Rayon scheduling does not change a result on one platform. Identical seeds are reproducible,
but are not SIMION's RNG sequence. Long ion streams do not reuse adjacent
ions' subsequences.

## CLI

An electric PA is optional for purely magnetic or field-free collision runs.
`--pa` accepts a normal PA when no voltage adjustment is requested; fast
adjustments still require PA0. Existing voltage run files remain valid.

```sh
simrep fly --particles ions.ion --magnetic-pa magnet.pa0 \
  --bounds=-200,-40,-40,430,40,40 --nonrelativistic

simrep fly --particles ions.ion --bfield=0,0,100 \
  --integrator boris --step-us 0.0001 --duration-us 10 --allow-time-limit

simrep fly --particles ions.ion --collision hs1 --seed 813 \
  --pressure-pa 0.53 --temperature-k 273 --gas-mass-amu 4 \
  --cross-section-m2 2.27e-18 --collision-steps 20 \
  --duration-us 50 --allow-time-limit --output hs1.csv

simrep fly --particles ions.ion --collision sds --seed 813 \
  --mbmr /mnt/c/Users/lsu10/simrep-oracle/collision_sds/mbmr.dat \
  --gas-diameter-nm 0.366 --temperature-k 273.15 \
  --duration-us 50 --allow-time-limit --output sds.csv
```

`--mean-free-path-mm`, `--gas-velocity=x,y,z`, `--ion-diameter-nm`,
`--mobility-stp` and `--no-diffusion` expose the remaining transport inputs.
HS1 defaults are 0.53 Pa, 273 K, helium 4 amu, sigma=2.27e-18 m2;
SDS defaults are 760 torr (converted to Pa), 298.15 K, air 28.94515 amu
and 0.366 nm. `--allow-time-limit` explicitly accepts fixed-duration
termination; other numerical failures still make the command fail.

`--bfield-coil coils.json` reads a JSON array of tagged objects:

```json
[
  {"kind":"segment","first_mm":[0,0,-10],"last_mm":[0,0,10],"current_a":2},
  {"kind":"loop","center_mm":[0,0,0],"normal":[0,0,1],"radius_mm":30,"current_a":100},
  {"kind":"solenoid","first_mm":[-50,0,0],"last_mm":[50,0,0],"radius_mm":10,"current_a":0.7958,"turns":100}
]
```

The same coil array can be `physics.coils` inside `--run-file`.
`physics.uniform_gauss` and `physics.magnetic_pas` are also supported.
Each PA entry has `path`, optional `origin_mm`, orthonormal `axes` (columns),
`scale`, optional `ng`, `polarity_flip_axis`, and
`angular_sector: [axis,start_deg,end_deg]`. `physics.magnetic_support` has the
same placement fields and gates only uniform/analytic sources.
CLI `--magnetic-pa` uses the CLI electric placement, while JSON entries
have independent placements. Relative paths are relative to the process
working directory. Unknown JSON fields and invalid model parameters fail.

## Validation protocol and artifacts

`tools/simion-oracle/physics_validate.py` creates dedicated working copies
under `C:\Users\lsu10\simrep-oracle\physics-validation-20261023`.
It uses the shipped Lua only inside SIMION as an oracle. Magnetic fields
are probed through the documented `instance:field_wc` interface; adjusted
PAs and workbench transforms are exported before native evaluation.
Original example files are not modified.

Fresh collision runs record each ion's end x/y/z, TOF and KE directly at
the fixed-time `other_actions` endpoint, before requesting a splat. SIMION
can rewind its terminate callback to the preceding trajectory point on
an artificial splat; recording there would incorrectly label a 50 us
endpoint as 49.95 us (HS1) or 49 us (SDS). The raw endpoint data and all
stdout remain in the external working directory.

The two-sample KS comparisons use SciPy's exact calculation with 4096
ions in each independent sample. TOF is a fixed-time atom, quantized only
below 1e-7 us to avoid treating floating-point summation noise as a
distribution difference; its unrounded range is reported separately.
SDS's zero-field, zero-initial-energy KE is also a degenerate atom.
Only the position distributions and HS1 KE test stochastic transport.
P-values are evidence against a distribution difference, not proof of
identity or a measure of numerical accuracy.

Imported adjustable values are set with SIMION's `--adjustable` arguments,
not assignments into the importing Lua file's environment. The SDS log
confirms 760 torr and 273.15 K. A direct documented-API probe
(`physics_seed_probe.lua`) found that `seed(237)` repeats a sequence within
one process but gives different sequences in separate SIMION processes.
Consequently the statistical fly and its silent timing replay run in the
same process, with one SIMION thread and reseeding at first ion initialization.
All 4096 reported splat records replay identically in both models. The raw
statistical sample is retained; another process can produce different KS
p-values. Native RNG streams do not have that process dependence.

### Statistical results, 2026-10-23

Each model uses 4096 SIMION ions and 4096 native ions, mass 200 amu,
charge +1, initially at the origin and terminated at 50 us. HS1 starts at
0.1 eV along +X, with 0.53 Pa helium at 273 K and sigma=2.27e-18 m2;
the oracle step is 0.05 us, and native steps also obey the 20-subdivision
optical-depth limit. SDS starts at zero velocity, with air at 760 torr,
273.15 K and one 50 us diffusion step. The native seed is 813 and the
same-process SIMION seed is 237.

| Model | Quantity | Two-sample KS D | Exact p-value |
| --- | --- | ---: | ---: |
| HS1 | end X | 0.0180664 | 0.515856 |
| HS1 | end Y | 0.0200195 | 0.384564 |
| HS1 | end Z | 0.0104980 | 0.977716 |
| HS1 | TOF | 0 | 1 |
| HS1 | KE | 0.0200195 | 0.384564 |
| SDS | end X | 0.0156250 | 0.699440 |
| SDS | end Y | 0.0129395 | 0.882848 |
| SDS | end Z | 0.0205078 | 0.355182 |
| SDS | TOF | 0 | 1 |
| SDS | KE | 0 | 1 |

None rejects a common distribution at 5%; TOF and SDS KE are the degenerate
atoms described above. The unrounded HS1 oracle TOF is 49.9999999999993 us
and SDS TOF is exactly 50 us.

The independent HS1 low-field check uses 16384 ions, mass 200 amu, helium
4 amu, 0.53 Pa, 273 K, E=0.0005 V/mm and 1000 us duration, with initial
mean velocity at the predicted drift to avoid an initial drift transient.
The Mason-Schamp prediction is 0.0238088528877 mm/us; measured displacement
per time is 0.0239279203405 mm/us, standard error 0.000340963234528 mm/us.
The difference is +0.5001%, within the 1.43% relative standard error.

### Magnetic field and trajectory results

Direct field probes compare the same inputs, not interpolated trajectories.
The coil comparisons include 21 off-axis points per source. Scalar-PA
differences include the intentional C2/C1 reconstruction versus SIMION's
coarser interpolation and must not be interpreted as error against an
analytic field.

| Source | Points | Maximum absolute B difference, G | Maximum relative difference |
| --- | ---: | ---: | ---: |
| magnet1 PA | 10 | 23.3368 | 0.0828170 |
| mag90 PA | 14 | 574.698 | 0.0285397 |
| sector 2dp PA | 26 | 2.30156 | 0.0149333 |
| sector 2dc PA | 9 | 0.00607401 | 6.07402e-6 |
| sector 3dp PA | 9 | 2.29585 | 0.0795626 |
| Circular loop / SIMION MField | 21 | 8.40109e-7 | 4.00641e-8 |
| Helmholtz pair / SIMION MField | 21 | 4.91069e-7 | 1.63840e-8 |
| 100-hoop solenoid / SIMION MField | 21 | 6.52309e-8 | 7.45018e-8 |

The solenoid axial center value is 9.806136450441315 G, agreeing within
2e-12 G with the retained high-precision `solenoid/out.csv`.
The uniform-B RK45/Boris half-orbit radius check is below 1e-8 relative;
the loop axial limit is within 1e-13 G. An independent 4096-segment polygon
integral checks the off-axis elliptic loop implementation to 4e-7 relative.
Exact constant-field SDS drift and damped gyration are checked separately,
including negative charge and nonzero B.

A separate source-free scalar PA, `phi=cos(pi*x/2)*cosh(pi*y/2)`, is
refined eightfold in space and compared to its own analytic B field and
three analytic-reference flights. This checks interpolation plus integration
without treating SIMION as ground truth:

| Grid | RMS endpoint error against own analytic field, mm |
| --- | ---: |
| 9x9 | 3.90778375e-7 |
| 17x17 | 5.51813278e-8 |
| 33x33 | 5.43587826e-9 |
| 65x65, eightfold refinement | 1.08579560e-9 |

The native trajectory reference uses an eightfold smaller maximum time
step (0.0025 versus 0.02 us) and tolerance 1e-12 versus 1e-10. Identical
captured initial states and workbench transforms are used in every pair.
Helmholtz's artificial termination is localized to a common 0.03 us;
physical workbench exits can occur earlier.

| Model | Ions | Max final-position difference vs SIMION, mm | Max TOF difference, us | Max difference vs tighter native reference, mm |
| --- | ---: | ---: | ---: | ---: |
| magnet1, E+B | 10 | 0.0465094 | 0.00369609 | 5.76413e-7 |
| mag90 | 10 | 0.0315245 | 0.000128330 | 7.02278e-7 |
| sector 2dp | 15 | 0.00914941 | 0.000290456 | 1.18257e-7 |
| sector 2dc | 50 | 0.00188868 | 0.0000773592 | 2.31864e-7 |
| sector 3dp | 15 | 0.107908 | 0.00693520 | 1.20506e-7 |
| Helmholtz | 27 | 0.212810 | 0.0000647327 | 3.02675e-9 |
| Solenoid | 100 | 2.01204 | 0.000971171 | 3.04818e-6 |

These are measured differences, not assertions that SIMION is exact.
The largest raw differences occur in the default-step solenoid example;
the much smaller native reference differences and analytic coil checks are
the native accuracy evidence. Scalar PA interpolation and magnetic material
surfaces are separate from integrator convergence. The initial electric
nearest-node geometry is retained for electrostatic PAs.

### Compute time

Only the reported native `fly_s` interval and SIMION's own
`Fly completed ... seconds` are compared. File loading, field reconstruction,
GPU initialization and CSV output are excluded. Magnetic runs use SIMION's
stored/default trajectory settings; collision timing uses the same-process,
one-thread replay with terminal printing disabled. Native runs use Rayon.
The 0.01 s resolution of short SIMION runs limits the useful precision of
their ratios.

| Model | SIMION reported compute, s | Native CPU f64 fly, s | SIMION/native |
| --- | ---: | ---: | ---: |
| magnet1 | 0.01 | 0.009007 | 1.11 |
| mag90 | 0.01 | 0.004663 | 2.14 |
| sector 2dp | 0.05 | 0.005698 | 8.78 |
| sector 2dc | 0.05 | 0.005601 | 8.93 |
| sector 3dp | 0.04 | 0.217283 | 0.184 |
| Helmholtz | 0.06 | 0.004202 | 14.3 |
| Solenoid | 5.07 | 0.577046 | 8.79 |
| HS1, 4096 ions | 9.96 | 0.303679 | 32.8 |
| SDS, 4096 ions | 1.23 | 0.006252 | 196.7 |

The original 0.02 us capped CPU 3D-sector run above is slower than SIMION.
The matched-reference CPU rerun below resolves that gap by tuning the RK45
step cap. These are configured-workload results, not a general GPU speed
claim.

### CPU magnetic profiling and matched-reference rerun

Fresh foreground SIMION runs on 2026-10-23 retained the same exported PAs,
workbench placements and model hooks. The native performance test reads the
captured `START` position/velocity directly, without a velocity/energy
round trip. It constructs fields and Rayon pools before timing and reports
the median of five flights. File input, dense C2 reconstruction, pool
construction and result output are excluded. SIMION times are its reported
`Fly completed` intervals; their 0.01 s resolution limits short-run ratios.

The root cause was step count, not rejected integration trials. At 0.02 us,
the 15 sector-3dp ions need 438898 accepted steps and zero rejections over
roughly 552--618 us flights. The 1 us cap needs 9526 accepted steps and 397
rejections at the same 1e-10 tolerances. RK45 still controls local error,
including the explicit polarity discontinuity. Its seven field evaluations
per trial, plus event localization, fall from about 3.08 million to 74606.
The paired 16-thread compute times are 148.65 ms and 10.06 ms: a 14.8x
reduction, and 3.98x faster than SIMION's 40 ms.

| RK45 maximum step, us | Accepted / rejected | 1 thread, ms | 8 threads, ms | 16 threads, ms | Max position error vs native reference, mm |
| ---: | ---: | ---: | ---: | ---: | ---: |
| 0.02 | 438898 / 0 | 1059.91 | 184.37 | 148.65 | 1.21034e-7 |
| 0.1 | 87865 / 53 | 363.42 | 55.12 | 47.74 | 9.77063e-7 |
| 0.2 | 44088 / 126 | 162.05 | 38.68 | 41.24 | 2.00474e-6 |
| 0.5 | 18013 / 265 | 77.79 | 10.42 | 11.61 | 3.61103e-6 |
| 1 | 9526 / 397 | 54.73 | 9.99 | 10.06 | 2.44417e-6 |
| 2 | 5474 / 480 | 35.21 | 6.00 | 7.66 | 2.13331e-6 |

Per-ion parallelism is already present; 15 ions limit available concurrency.
At the selected 1 us cap, eight and sixteen workers have comparable cost.
Repeated 16-worker sweeps measured 6.67--11.22 ms under varying host load,
all below SIMION's 40 ms. The 2 us/eight-worker point is faster still, but
the 1 us setting has smaller TOF error and is used consistently below.

A serial microbenchmark at 1571 active trajectory points measured 372 ns
per magnetic PA sample (other sweeps: 400--422 ns). Evaluation uses the
existing dense 216-coefficient tensor quintic, transforms and material
selection. A transform/material-lookup shortcut did not establish a
reproducible compute gain and was not retained. The interpolation and
integrator kernels are unchanged; the improvement is the measured RK45
configuration, not a lower-order field approximation.

Boris uses one field sample per fixed step, but needs many more steps for
the same accuracy. On 3dp its 0.02 us step took 40.79 ms with 2.78031e-6 mm
maximum position error. At 0.1, 0.2 and 0.5 us the errors increased to
1.96236e-4, 1.52283e-3 and 9.94628e-2 mm. The last setting took 7.79 ms
but loses almost the entire accuracy margin over SIMION. RK45 at 1 us
retains 2.45e-6 mm endpoint accuracy without that phase error.

Every row below uses RK45, 1 us maximum step, 1e-10 tolerances and sixteen
Rayon workers. Both endpoint and TOF errors are smaller than SIMION's
against the same native reference; the test asserts those maximum-error
comparisons, not agreement with SIMION.

| Model | SIMION compute, s | Native compute, s | SIMION/native | Native max position error, mm | SIMION max position error, mm |
| --- | ---: | ---: | ---: | ---: | ---: |
| magnet1 | 0.01 | 0.0022343 | 4.48 | 2.34507e-7 | 4.65061e-2 |
| mag90 | 0.01 | 0.0026643 | 3.75 | 4.92294e-7 | 3.15272e-2 |
| sector 2dp | 0.05 | 0.0046034 | 10.86 | 2.04451e-7 | 9.14382e-3 |
| sector 2dc | 0.04 | 0.0039928 | 10.02 | 2.74429e-7 | 2.08654e-3 |
| sector 3dp | 0.04 | 0.0100560 | 3.98 | 2.44417e-6 | 1.07902e-1 |
| Helmholtz | 0.12 | 0.0017843 | 67.25 | 1.46869e-8 | 2.12810e-1 |
| Solenoid | 7.01 | 0.7383168 | 9.49 | 1.22616e-6 | 2.33651 |

The reference uses a 0.0025 us cap and 1e-12 tolerances, as in the original
timestep study. An additional eightfold timestep refinement of 3dp to
0.0003125 us changes its endpoints by at most 8.53998e-9 mm and TOF by
2.14289e-8 us. The selected 1 us run differs from the 0.0025 us reference
by at most 3.95509e-7 us, versus SIMION's 6.92313e-3 us. The separate
9/17/33/65-node analytic scalar-PA spatial-convergence test still reaches
1.08580e-9 mm RMS. Imported example PAs are held fixed in the timing
comparison; the analytic grid study is spatial-validation evidence, not
an eightfold re-solve of each imported geometry.

The ordinary CLI was also exercised at 1 us: all 15 ions terminated at
the workbench wall, with 9524 accepted / 395 rejected steps and 6.862 ms
reported `fly_s`. Its maximum difference from the retained tight reference
was 3.01241e-6 mm and 3.86631e-7 us. Its 16.86 s PA reconstruction is setup,
not flight compute.

Reproduce the profiling and matched-reference gate after preparing the
foreground SIMION artifacts:

```sh
SIMREP_PHYSICS_ORACLE=/mnt/c/Users/lsu10/simrep-oracle/physics-validation-20261023 \
  cargo test --release -p simrep-fly --test magnetic_performance -- --nocapture
# Optional: limit the profiling test to sector_3dp.
SIMREP_MAGNETIC_CASE=sector_3dp \
SIMREP_PHYSICS_ORACLE=/mnt/c/Users/lsu10/simrep-oracle/physics-validation-20261023 \
  cargo test --release -p simrep-fly --test magnetic_performance -- --nocapture

RAYON_NUM_THREADS=16 target/release/simrep fly --device cpu \
  --particles /mnt/c/Users/lsu10/simrep-oracle/physics-validation-20261023/sector_3dp/ions.ion \
  --run-file /mnt/c/Users/lsu10/simrep-oracle/physics-validation-20261023/sector_3dp/run.json \
  --bounds=-11,-156,-156,11,156,156 --duration-us 10000 \
  --nonrelativistic --max-step-us 1 --tolerance 1e-10 --output /tmp/sector-3dp.csv
```

The optional external performance test is inert without
`SIMREP_PHYSICS_ORACLE`. Normal workspace tests retain their self-contained
analytic cases. The older `physics_validate.py --stage native` command
still reproduces the original 0.02 us baseline, not the tuned sweep.

Build and reproduce:

```sh
cargo build --release -p simrep-cli
cargo build --release -p simrep-fly --example physics_probe
python3 tools/simion-oracle/physics_validate.py --only magnetic --stage prepare
python3 tools/simion-oracle/physics_validate.py --only magnetic --stage native
python3 tools/simion-oracle/physics_validate.py --only collision --ions 4096
python3 tools/simion-oracle/physics_validate.py --check-replay
target/release/examples/physics_probe mobility
SIMREP_MBMR=/mnt/c/Users/lsu10/simrep-oracle/collision_sds/mbmr.dat \
  cargo test --workspace
```

SIMION processes run foreground with closed stdin, a /mnt/c working
directory and a 45 s per-process timeout. Run the repository's Windows
process cleanup command after the last oracle invocation.
`--stage prepare` records only foreground Windows results; `--stage native`
reuses them without launching Windows. This separates long native 3D PA
reconstruction from Windows process supervision. Boundary-aware 3D magnetic
reconstruction uses the same dense quintic coefficients as the electric
reference; setup and its memory cost are excluded from fly compute timing.
Collision timing uses a second SIMION run with terminal printing disabled,
while the independent statistical recording is retained in `flight.log`.

Verification completed with `cargo fmt --all -- --check`,
`cargo clippy --workspace --all-targets -- -D warnings`,
the full `cargo test --workspace --release` suite with `SIMREP_MBMR`
set to the installed dataset, the explicit external magnetic performance
test, and a release CLI build; all commands exited 0. All eleven physics
tests passed, including an added rotated/scaled planar/cylindrical magnetic
placement check. DX12 checks reported no adapter; Vulkan checks ran on
llvmpipe. The language-server daemon timed out; cargo's compiler and strict
clippy checks completed cleanly.

Fresh native HS1 and SDS runs reproduced all 4096 rows of each retained
native CSV byte for byte. The retained SIMION recording/timing replay also
matched all 4096 splat records for each model. Thus the exact KS comparisons
above are unchanged (minimum p-value 0.355182); no stochastic test was
relaxed or re-seeded.

After the last foreground SIMION call, `tools/win/winproc.ps1 cleanup`
exited 0 and reported:

```text
== process counts ==
cmd.exe            4
conhost.exe       44
node.exe           4
powershell.exe     1
orphan conhost: 0
stopped orphan conhost: 0
```

No `simion.exe` process remained in the process counts.

## GPU requirements

The GPU path supports magnetic scalar PAs, uniform B, analytic coils,
imported vector B maps, Lorentz RK45/Boris integration, and HS1/SDS
collisions. `--device gpu` requests that path explicitly; no magnetic
compute crossover is enabled for `--device auto`, which stays on CPU.
Vector potentials and magnetic material cells remain CPU-only and require
`--device cpu` or `auto`. See `bench/fly-gpu.md` and `bench/bfield-import.md`
for matched-precision RTX 3070 validation and compute measurements; the
electrostatic crossover is not used for magnetic flights.
