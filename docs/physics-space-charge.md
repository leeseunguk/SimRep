# Synchronized space charge

`simrep_fly::physics::fly_ensemble(field, particles, options, &mut space_charge)`
is the coupled equal-time API (CPU f64 or GPU f64 PIC). `fly`, `fly_many`, and GPU independent flight keep
their existing contracts. `EnsembleReport` preserves particle order and reports
global accepted/rejected steps and compute time separately from preparation.
The implementation is in `physics/ensemble.rs` and `physics/space_charge.rs`.

## Charges and units

All state units remain mm, microseconds, amu, elementary charges, V/mm and Gauss.
Physical `charge_e / mass_amu` determines acceleration; represented source
charge is a separate quantity:

* `ChargeMode::Factor(f)` assigns `Q_i = f * q_i * e * CWF_i`.
* `ChargeMode::Coulombic(Q)` assigns
  `Q_i = Q * q_i * CWF_i / sum_j(abs(q_j) * CWF_j)`.
  `Q` is the total **absolute** charge in coulombs; physical particle charge
  determines each sign. This also covers mixed charge magnitudes and neutrals.

Amounts and CWFs are nonnegative. Weights are fixed from the entire input
population, including future births; they are never renormalized when a
particle is born or lost. Neutral particles contribute no source and remain
test particles. An empty `charge_weights` vector means CWF=1.
ION and the supported literal FLY2 reader now preserve CWF; omitted CWF is one.
This does not add a Lua interpreter or dynamic FLY2 execution.

`epsilon_0 = 8.8541878128e-15 F/mm`, so a deposited `rho` in C/mm³ produces
`source = rho / epsilon_0` in V/mm². Direct Coulomb uses
`k = 1/(4*pi*epsilon_0)` in V mm/C. Source charge is not multiplied by the
represented factor a second time in the target's Lorentz force.

## Time staggering and events

Positions and physical velocities are collocated at global endpoints.
Every live particle drifts half a step using its endpoint velocity. All
source and target positions therefore share the force time `t + dt/2`.
The external field and the direct/PIC field are evaluated there, a Boris
electric half-kick / magnetic rotation / electric half-kick updates velocity,
and the second drift completes the step. Relativistic runs kick proper
velocity and return physical velocity, as independent Boris flight does.

The event order is:

1. Insert births before the next force evaluation. A birth inside a proposed
   interval splits that interval exactly; the newly born particle receives
   only the remaining partial first push, and all existing particles share
   the same subinterval. There is no pre-birth source or unequal-time sum.
2. Evaluate charge fields from the synchronized half-drift positions.
3. Push all live particles.
4. Detect the first geometry/stop-plane crossing on each accepted segment.
   Early crossings shorten and recompute the **global** interval and its
   forces before acceptance, so survivors do not receive a full-step impulse
   from a source already lost. Position, velocity and time at a crossing are
   interpolated by its exact segment fraction; the source is then removed.
   This is exact segment localization, not exact curved-orbit localization.
  A crossing less than one absolute-clock ULP away is localized to that
  representable time and removed before recomputing surviving particles.

Each particle's duration is measured from its own birth. Time limits, voltage
edges and birth times also clip the global step. Sampling uses accepted
global steps in each particle's lifetime. A global attempt budget produces
`StepLimit` for unfinished particles. Singular fields, solver nonconvergence
and unresolvable time steps are errors, not silently accepted trajectories.

The global external-field error estimate compares one second-order Boris
step with two half steps, divided by three. It uses the existing absolute
position/velocity and relative tolerances. Direct charge additionally limits
`dt` to `fraction * min(sqrt(r³/(k*|Q|*|q/m|*ELECTRIC_ACCELERATION)), r/|dv|)`
over interacting pairs (with the same softened radius if selected). PIC
limits it by the maximum local plasma frequency and mesh-cell transit time.
The default fraction is 0.1; `max_step_us` still controls temporal refinement.
This is a coupled Boris path regardless of `FlightOptions.integrator`.

## Direct Coulomb

`SpaceCharge::Direct(DirectCharge { softening_mm })` is an O(N²) deterministic
sum, serial below 256 live particles and target-parallel with Rayon at 256
and above. The closest-approach sum uses the same cutoff, avoiding two thread
dispatches per step for small bunches. Every target visits sources in input
order; changing Rayon worker count does not reorder a target's sum. It does
not allocate an N² interaction matrix. Self pairs are excluded. Zero softening means exact
point Coulomb, and coincident nonzero sources return an explicit singularity
error. Positive softening selects the documented Plummer kernel
`E = k Q r / (r² + epsilon²)^(3/2)`, not SIMION's undocumented automatic cloud
radius. Choose softening as a physical/modeling input, not a convergence fix.

Like SIMION's charge-repulsion method, direct sums superimpose free-space
particle fields and the external electrode field. They do not enforce the
electrode potential after adding charge and do not include induced images.

## PIC and Poisson

`SpaceCharge::Pic(Box<PicMesh>)` accepts a separate `mg::Grid`, physical origin,
Dirichlet mask and solver options. CIC deposits source charge on neighboring
nodes. Planar dual-cell volumes include half volumes at faces; axisymmetric
dual volumes are annular rings, including the finite axis disk, times axial
cell length. The public cylindrical mesh stores **r,z** and uses Cartesian z
as its physical axis through `origin_mm`. This differs from PA storage
(axial x, radial y). Planar nz=1 represents an extrusion of `spacing[2]` mm.

Each force evaluation warm-starts the potential from the previous solve of
`-L phi_sc = rho/epsilon_0`. The new additive CPU `mg::solve_poisson` applies
the physical RHS in both fine smoothing and fine residuals; transpose
restriction receives the correctly weighted residual, including cylindrical
row weights. Fixed source entries are ignored. Existing `mg::solve` remains
Laplace-only. Potentials are warm-started; the current CPU API rebuilds the
Galerkin hierarchy per solve, and that per-step computation is timed.

The correction is zero at conductor nodes; adding it to the external field
therefore retains the prescribed electrode voltage. Nodal central differences
give E; at unmasked mirror faces the normal E is exactly zero. CIC gathers E
with the same shape weights as deposition. Cloud fractions overlapping a
Dirichlet node are absorbed into that conductor, not renormalized into vacuum.
The mesh must cover all live particles, including their force-time drift.
Use a suitably resolved mask and perform mesh as well as time refinement.

The CLI builds a full 3D Cartesian mesh over explicit workbench `--bounds`.
All its outer faces are grounded **correction** boundaries, and electrode
membership is sampled from the placed PA. Thus transformed/mirrored and
cylindrical external PAs do not silently turn a bunch into a symmetry-repeated
charge distribution. Move open faces far enough away and refine the mesh to
bound their image-field effect. Direct/PIC agreement requires matching boundary
physics; electrode-image differences are intentional, not iterative error.

`gpu::Gpu::solve_poisson` and `solve_poisson_with_precision` are additive
physical-RHS APIs; existing GPU Laplace and cut-cell APIs retain their contract.
`gpu::Poisson` prepares a reusable Galerkin hierarchy. Both native f64 residuals
and the CPU-f64-residual fallback add `source / normalization` **before** row
weighting and restriction. Fixed-node sources are ignored. Planar, cylindrical
axis and offset-axis tests compare both paths against CPU Poisson.

`SpaceCharge::GpuPic(Box<GpuPic>)` uses the same mesh, volumes, mask and CIC
shapes as CPU PIC. GPU f64 first computes exact cell addresses; WGSL integer
atomic exchange builds per-cell linked lists. Each node gathers its neighboring
lists and accumulates charge in f64, avoiding unsupported floating-point atomics
and f32 charge quantization. Atomic list order can vary, so roundoff-level
differences are expected, not bitwise identity. Nodal differentiation, CIC field
gathering and the synchronized Boris push run on GPU. Planar 2D/3D and the
CPU mesh's cylindrical r,z convention are supported.

The existing ensemble loop still owns births, step selection, external-field
error estimates and geometry events. It supplies exact reconstructed external
E/B samples at the shared midpoint and reads endpoint states back for event
localization. This is a coupled GPU PIC path, not an entirely resident trajectory
loop. Physical mesh-source validation/readback, transfers, synchronization,
Poisson cycles, gather and push are included in `compute_time`. Shader compilation,
buffer allocation and reusable hierarchy construction happen before flight.
GPU PIC requires native `SHADER_F64`; the physical Poisson API independently
supports its CPU-residual fallback on adapters without f64.

## CLI

```sh
simrep fly --pa blank.pa --particles bunch.ion \
  --bounds=-10,-4,-4,10,4,4 --nonrelativistic \
  --space-charge direct --charge-mode factor --charge-amount 1000 \
  --max-step-us 0.0001 --output direct.csv

simrep fly --pa electrodes.pa0 --particles bunch.ion \
  --bounds=-10,-4,-4,10,4,4 --nonrelativistic \
  --space-charge pic --charge-mode coulombic --charge-amount 3.47e-13 \
  --pic-dims 65,33,33 --max-step-us 0.0001 --output pic.csv

simrep fly --pa electrodes.pa0 --particles bunch.ion --device gpu \
  --bounds=-10,-4,-4,10,4,4 --space-charge pic \
  --charge-mode coulombic --charge-amount 3.47e-13 --output gpu-pic.csv

simrep fly --pa blank.pa --particles beam.ion --device cpu \
  --bounds=-10,-4,-4,10,4,4 --charge-mode beam --beam-current-a 1e-5 \
  --nonrelativistic --max-step-us 0.00005 --output beam.csv
```

`--charge-softening-mm` specifies a direct charge or beam line-cloud radius.
`--space-charge-step-fraction` adjusts the shared stability criterion.
Direct Coulomb and beam integration use CPU. PIC accepts explicit `--device gpu`;
auto selects f64 GPU PIC at 100,000 particles and above (small studies stay CPU).
Space charge is never silently ignored. CSV adds final Cartesian velocity
columns; time/step limits remain visible and unfinished flights fail the CLI.

## SIMION-documented beam semantics

`physics::fly_beam(field, particles, &BeamOptions)` is a separate integration
contract. Particle #1 defines a plane through its position normal to its
velocity. Every surviving follower advances to that plane at **its own** time;
this is not `fly_ensemble` with a current multiplier. A drift/kick/drift
midpoint construction evaluates transverse charge fields on the intermediate
plane and solves each follower's forward plane crossing. Half-kick velocity
predictors provide the plane normal and line densities, while force-plane
positions drift without the old space-charge acceleration. This retains the
central line force's angular momentum rather than applying spurious pair
torque at acceleration-shifted positions. The leader's time
step and every follower's local step are bounded. TOB is removed internally and
added back only to recorded absolute times, so it cannot change the beam shape.

The label is deliberately **SIMION-documented beam semantics**, not a claim
that SIMION's private force/cloud appendix has been reverse engineered.
The physical assumptions are explicit:

* Total absolute current `I` is distributed once as
  `I_i = I*q_i*CWF_i / sum(abs(q_j)*CWF_j)`. Neutrals source no current but may
  be the leader. Removing a trajectory removes its current without renormalizing
  survivors. This extends the documented equal-charge CWF rule to mixed charges.
* A line's signed density is `lambda_i = 1e-6*I_i/(v_i dot n)` C/mm. Lines are
  locally parallel to the leader normal. Their standard 2D field is
  `E_ij = 2*k*lambda_j*r_perp/(r_perp² + softening²)`. Self lines are excluded.
  Zero softening is the exact line kernel; coincident charged lines are an error.
  Longitudinal charge force, magnetic self-fields, electrode images and cloud
  overlap rules from the private SIMION manual are not included.
* `BeamInteraction::Rings` azimuthally averages parallel lines into coaxial
  cylindrical shells about the leader. A ring contributes only to radii outside
  it; the equal-radius value is half the discontinuity. This is the symmetric
  thin-shell limit, intended for an axisymmetric transverse beam, not an
  arbitrary three-dimensional bent ring-current distribution. CLI uses lines.
* Static external fields are required. Known voltage waveforms/events are
  rejected. This single pass does not iterate trajectories and Poisson fields
  to a self-consistent steady state; that is a future, distinct model.
* A follower that cannot cross forward (including >90-degree divergence) gets
  `BeamTermination::Diverged`. The remaining beam continues with unchanged
  weights. If the leader stops, all surviving followers stop on its last plane,
  regardless of their unequal TOFs. `BeamReport.terminations` distinguishes
  `LeaderStopped`/`Diverged` from physical splats without changing independent
  flight's `SplatReason`. CSV uses `beam_leader_stopped`/`beam_diverged`.

The beam tests exercise unequal flight times, TOB invariance, rotating leader
planes, backward-follower loss, and the cold uniform-cylinder drift envelope
`R'' = C/R`, `C = (q/m)*ELECTRIC_ACCELERATION*2*k*I*1e-6/v`.
For a unit-radius beam with v=10 mm/us, q/m=1 e/amu, I=1e-8 A and duration
0.48 us (0.5182134834 eV for singly charged 1 amu ions), independent RK4
gives R=1.193692751585 mm. Ring-mode errors at
dt=0.04,0.02,0.01 us are 4.33276e-5, 1.08307e-5 and 2.70759e-6 mm:
ratios 4.00 and 4.00. This checks the continuum force and second-order
integration, separately from finite particle sampling and SIMION cloud details.

`tools/simion-oracle/beam_readout.lua` records each ion's initial state, CWF,
final position/velocity and absolute TOF. `beam_compare.py` runs the repulsion1
and mixed-charge beam examples in separate working copies, replays the actual
drawn initial states, and reports endpoint/velocity/TOF statistics and time-step
convergence. The shipped `repulsion1.lua` records potentials only and is not
used as an endpoint reference. See `bench/space-charge.md` for measured results.

The line-mode step limiter resolves both pairwise acceleration and transverse
relative transit, using each follower's clock rate relative to the leader.
It does not rely only on net acceleration: oppositely signed line fields can
cancel while a close encounter remains unresolved. The initial line-field and
encounter-limit calculation share one source-ordered pair loop; targets are
parallel from 64 live trajectories. A localized loss restores the preceding
trial step only after the trajectory is actually removed. Restoring it while
the accepted endpoint is still just inside a boundary can cause repeated
near-zero advances; the loss and grazing-line regressions cover these cases.
Beam uses the normal four-cell automatic spatial ceiling, external-field
error control and explicit maximum leader/follower dt.
An endpoint encounter-scale check bisects a trial before accepting an
unresolved near-coincidence or a scale decrease beyond the controller's
factor-two step growth. Invalid midpoint force predictions are also rejected,
not softened. Fields and encounter limits are cached across retries and
accepted no-loss planes, and invalidated whenever the source population
changes. Genuine coincident charged input lines remain undefined and fail.
The beam stability fraction defaults to 1.0; the synchronized ensemble keeps
its separate 0.1 default. `--space-charge-step-fraction` overrides either.
This default targets cheaper equal-or-better precision than the two recorded
SIMION cases, not individually converged mixed-beam scattering. Tighten the
fraction as well as dt when individual trajectories matter; the convergence
and timing tables in `bench/space-charge.md` state that distinction.

`tools/simion-oracle/beam_reference.py` is an independent DOP853 reference for
the captured examples' straight, neutral leader and exactly zero external PA.
It integrates transverse position and velocity in plane position x, with
`dy/dx = vy/vx` and `dvy/dx = (q/m)*ELECTRIC_ACCELERATION*Ey/vx`, restarting
at every side loss and ending survivors on x=10 mm. It shares no production
force, push or geometry code. Maximum plane-spacing halving and independent
tolerance tightening are both required: the mixed beam's near-focus encounters
make a fixed-tolerance spacing plateau an insufficient accuracy check.
The final mixed reference changes by 1.75293e-7 mm RMS / 2.16912e-6 mm maximum
when its relative tolerance tightens from 3e-13 to 3e-14. Its exit classes do
not change. The external field is identically zero at every resolution;
refining that PA tests numerical invariance, not an unknown field solution.

`beam_simion_refine.py` replays the same measured launches, with their original
SIMION KE, and halves `beam_max_dt` through the readout's optional
`tstep_adjust` hook. It preserves the originals and bounds every foreground
Windows invocation. A common terminal follower plane can slightly trail the
leader in SIMION output, so exit classification uses that shared plane rather
than interpreting its micrometre-scale ordering offset as a side splat.
SIMION-to-reference discrepancies include model differences, whereas native
errors against this reference measure numerical error in the stated line
model. The last SIMION dt-to-dt change is reported separately; subtracting
RMS errors would not isolate an undocumented cloud kernel.

The two captured beam cases now have independently converged references.
Against those specified line-model references, default numerical controls
have lower endpoint RMS/max, TOF RMS/max and exit-class error than SIMION
and take 0.269405/2.116568 s versus the faster measured SIMION medians
0.86/4.21 s: 3.19x/1.99x faster compute for repulsion1/mixed. Each median
uses three fresh runs. This comparison does not establish private-cloud
equivalence or individually converged native mixed scattering; the separate
model residual, resolution tables and much tighter/slower native contexts
are explicit in `bench/space-charge.md`.

## Numerical evidence

`crates/simrep-fly/tests/space_charge.rs` checks:

* exact Coulomb units, signed/multiply charged CWF accounting and singularities;
* births, exact segment stop times, lifetime removal and absence of pre-birth force;
* direct two-body cold expansion against its implicit analytic solution, with
  dt=0.04/0.02/0.01 us errors `2.348765e-4`, `5.785171e-5`, `1.445999e-5` mm
  (error ratios 4.06 and 4.00);
* CIC conservation including the cylindrical axis volume, symmetric self-force,
  and PIC dt halving against its own tightly time-converged mesh reference:
  errors `2.815777e-6`, `7.017689e-7`, `1.734399e-7` mm at
  dt=0.04/0.02/0.01 us (ratios 4.01 and 4.05);
* uniform sphere expansion: `R''=k Q (q/m) ELECTRIC_ACCELERATION/R²`.
  At 0.3 us with 257 lattice samples, analytic scale=1.35018239,
  direct=1.35458036, PIC=1.35422336. Direct/PIC RMS-radius scales differ by
  0.027%; the remaining continuum difference includes finite-particle sampling.
  Debug `cargo test` flies 0.15 us instead of 0.3 us with the same time
  step, particles and 33^3 PIC mesh; the analytic envelope is evaluated at
  the same duration, so all tolerances apply unchanged;
* uniform, axially Neumann cylindrical beam expansion:
  `R''=lambda (q/m) ELECTRIC_ACCELERATION/(2*pi*epsilon_0*R)`.
  At 0.2 us with 576 ring/axial quadrature samples, the independent analytic
  envelope integration gives 1.16878562 and PIC gives 1.16858552 (0.0171%).

`crates/simrep-solve/tests/poisson.rs` independently checks exact manufactured
planar/cylindrical quadratic sources on anisotropic grids, the axis stencil,
source sign, fixed-node preservation, source validation and zero-cycle warm starts.
These are our accuracy references; SIMION agreement is a compatibility
measurement rather than the definition of correctness.

Oracle endpoint and compute-time reproduction is in
`tools/simion-oracle/space_charge_compare.py`; the independent DOP853 reference
and external-grid refinement are in `bench/space_charge_reference.py`.
See `bench/space-charge.md` for the point-Coulomb accuracy verdict and
small-N compute speedup.
The archived phase-5 `.oracle_recording.txt` files primarily contain the
example Lua's kQ/r potential diagnostics, not endpoint tables. The harness
makes a separate `space-charge/{factor,coulombic}` working copy, instruments
the original repulsion1 particle source, and replays every actual initial
state/CWF in the native code. It does not mistake independent random sources
for matched inputs.
The harness subtracts SIMION TOB from its absolute `ion_time_of_flight`
before comparing the native lifetime TOF, and converts the measured initial
velocity to native KE using the native constants rather than assuming the
two programs use the same mass/charge conversion constant.

Sources for the model distinctions are the locally extracted SIMION
`charge_repulsion.html`, `workbench_program_extensions.html`, and the
SIMION manual's Flying Ions "Effective Weighting Factor" definition.
