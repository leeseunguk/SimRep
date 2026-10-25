# Magnetostatics

Coordinates are mm, magnetization is A/mm, current density is A/mm^2, and B
is gauss. Permeability is finite, positive, scalar relative permeability.
There is no nonlinear B-H constitutive model.

## Equations and flight contract

Scalar magnetic PAs store p, with the PA header's positive scale ng:

```text
-div(mu_r grad p) = (4*pi/ng) * (-div M)
B = -ng*mu_r*grad p + 4*pi*M
```

The output remains an ordinary magnetic PA. Its sibling `.mag.json` stores
the material cell mask. Cells with mu_r != 1 or nonzero M are solid for
ions. `MagneticPa` uses the existing `Electrode { index }` terminal reason
at the first occupied cell face, including thin cells crossed in one step.
The index is the lower PA node of the occupied cell. Vacuum sampling and
flight use the existing scalar reconstruction: there mu_r=1 and M=0, so
`B=-ng grad p` is the full B field. Sampling this scalar field inside solid
material is **not** an evaluation of the full material B.

Current fields use a different CPU field type, `VectorPotential`:

```text
planar:       -div((1/mu_r) grad Az) = (4*pi/ng) Jz
axisymmetric: -div((1/(mu_r*r)) grad psi) = (4*pi/ng) Jphi
              psi = r*Aphi
Cartesian:    -(1/mu_r) Laplacian A_i = (4*pi/ng) J_i
```

The psi equation uses a **planar** divergence, not the cylindrical scalar
Laplacian. Psi is zero on the regular axis. Cartesian components require a
spatially uniform mu_r; variable-mu 3D vector equations are coupled and are
not implemented as three independent scalar equations.

The flyer takes the curl of the reconstructed C2 potential, rather than
interpolating B components. For psi it reconstructs the regular even
function `f=psi/r^2`; its axis value is `(4*f(h)-f(2h))/3`. Then, in local
PA coordinates (axial x, radial y/z):

```text
Bx = ng * (2*f + r*df/dr)
Br = -ng * r*df/dx
Bx(axis) = 2*ng*f(axis)
```

This gives the axis limit without division by r or subtractive cancellation.
The reconstructed B is C1 and divergence-free as the curl of one potential.
Axis fixed-node flags are regularity constraints, not physical conductors.
Each Cartesian component retains its own scalar mirror/anti-mirror parity.
Placement preserves field strength when scaling lengths. Reflected
placements transform the curl as an axial vector, `det(R)*R*B`.

Vector-potential sources add to one another and to uniform, scalar-PA, and
coil sources. Existing scalar-PA overlap priority is retained. Vector fields
and magnetic material cell collisions require CPU flying: explicit GPU
requests fail clearly, and automatic device selection uses CPU, including
PIC flights. Native physics preparation also rejects these inputs.

## CLI and input layout

```sh
simrep refine magnet.pa --magnetic-material magnet.json --tolerance 1e-12
simrep fly --particles ions.ion --magnetic-pa magnet.pa
simrep refine az.pa --magnetic-material current.json
simrep fly --particles ions.ion --magnetic-vector-potential az.mag.json
```

Magnetic refinement accepts ordinary refinable magnetic `.pa` files, not
PA# fast-adjust bases. Ordinary Laplace refinement is unchanged. Refine
supports explicit CPU/GPU and the existing material auto-device policy.
Permeability and magnetic specifications are mutually exclusive.

Scalar specification:

```json
{
  "permeability": "mu.pa",
  "magnetization": ["mx.pa", "my.pa", null]
}
```

All keys are optional. Missing permeability means 1 and missing M means 0.
An optional `"negative_divergence":"minus-div-m.pa"` replaces the
divergence calculation with a precomputed source. If M components are not
provided, this requires a `"solid":"magnet-interior.pa"` cell PA, because
surface charge does not identify a magnet's interior. Nonzero solid labels
add occupancy; they do not change the PDE.

Current specification:

```json
{"permeability":"mu.pa", "current_density":"j.pa", "solid":"wire.pa"}
```

J selects planar Az or axisymmetric psi from the potential PA symmetry.
Current density alone does not imply a solid conductor; use `solid` to
declare conductor occupancy. A specification cannot combine currents and
scalar magnetization.

Every input PA is a non-electrode primal-cell array, with
`max(node_count-1,1)` samples per dimension and matching physical spacing
and symmetry. Paths are relative to the JSON. Cylindrical PAs use SIMION
axial-x/radial-y storage; M components are (axial,radial,0). The loader
transposes arrays and components to solver (r,z,0). Sources are averaged to
nodes using the appropriate control-volume measure. Cell M uses shared
face averages and a conservative finite-volume divergence. A precomputed
SIMION source remains distinct from this model: interpolation, finite
differences, and literal Lua shifts can change the discrete source.
Cylindrical source and axial-M face averages use the half-cell annular
centroids `r +/- h/4`, not primal-cell centers `r +/- h/2`. A regression
checks the integrated source of a cell touching the axis; using primal
centers loses one quarter of that cell's represented volume.

Three-component Cartesian specification:

```json
{
  "cartesian": {
    "potentials": ["ax.pa","ay.pa","az.pa"],
    "current_density": ["jx.pa","jy.pa","jz.pa"],
    "permeability": 1
  }
}
```

Run refine on `ax.pa`. Each component uses its own fixed-node and
anti-mirror constraints, with the same coefficient construction. All three
must converge before any component is written. `solid` optionally names
a shared cell PA. The resulting `ax.mag.json` loads all three components.

## Field sidecar

Version 1 is a small JSON header plus ordinary binary PA data:

```json
{
  "version": 1,
  "formulation": "axisymmetric_psi",
  "potential": "psi.pa",
  "ng": 100,
  "solid_cells": [false, false, true],
  "components": null
}
```

`solid_cells` must contain one boolean per primal PA cell in x-fastest PA
storage order. The short array above only illustrates the schema.
Formulations are `scalar`, `planar_az`, `axisymmetric_psi`, or `cartesian_a`.
Cartesian headers require three `components` paths in Ax,Ay,Az order.
Paths resolve relative to the header; absolute paths are accepted.
Scalar headers are picked up automatically by `--magnetic-pa`; vector
headers require `--magnetic-vector-potential`, preventing accidental
interpretation of A as a scalar magnetic potential.

## Rust and planned Python API

`simrep_solve::magnetostatics::MagneticProblem::{scalar,currents,
cartesian_current_component}` prepares reusable physical sources and
variable-coefficient stencils. `solve` uses CPU f64. `solve_gpu` uses
`gpu::Poisson::new_prepared` with that same stencil and physical source,
including native-f64 and CPU-residual fallback paths. Coefficient
construction is shared with `dielectric::Permittivity::from_cells`;
`prepared_stencil()` exposes only a borrowed prepared operator.

Python is intentionally not changed in this increment. Add these bindings:

- `refine(..., magnetic_material=path)` with the same JSON and device rules.
- `VectorPotential.from_file(header, origin_mm, axes, scale, bounds_mm)`.
- Vector-potential sources in magnetic superposition and the CPU flight API.
- Scalar material sidecar loading, occupancy, and `Electrode` terminal index.
- The same explicit GPU rejection and auto-CPU routing.

## Validation

Analytic regressions cover magnetized sphere/cylinder, normal iron-slab
flux, infinite wire/return coax, the psi manufactured solution, axis-limit
reconstruction, Cartesian curl/parity/placement, superposition, material
face splats, and analytic-field flight equivalence. Curved cell interfaces
are staircase models; their geometric error must be separated from solver
residual error. Grid orders and SIMION example audit results are recorded
after execution, not inferred from the residual.

The deterministic analytic tests use cell-volume-averaged M/J and
permeability, so refinement represents the same physical circles/spheres,
not a center-label volume that can oscillate between grids. At relative
physical residual 1e-12, the measured last-pair orders are:

| Analytic case | Grids | Error on finest grid | Order |
| --- | --- | --- | --- |
| Uniform transverse cylinder M | 17,33,65 | RMS p 2.19435e-5 | 1.35332 |
| Uniform sphere M | 17,33,65 cubed | RMS p 1.73595e-5 | 1.31904 |
| Infinite current wire | 33,65,129 | RMS Az 2.95790e-6 | 2.13212 |
| Current-return coax | 33,65,129 | RMS Az 3.17575e-6 | 2.16354 |
| Iron cylindrical shield, mu=20 | 33,65,129 | RMS p 3.83370e-3 | 0.96620 |
| Smooth axisymmetric psi | 17,33,65 | max psi 7.76008e-6 | 2.00186 |

The shield center field is 0.275821, 0.250996, and 0.239051, approaching
the analytic 0.228083. The grid-aligned mu=100 slab has exact continuous
normal flux to the solver tolerance. Curved material interfaces remain
first order; this path does not claim general second-order material
geometry. Separate tests exercise the curl, regular axis, component
parities, placement, superposition, thin material collisions, and flights.

## Installed SIMION audit protocol

`crates/simrep-solve/examples/magnetostatics_audit.lua` runs one named
example and mode per bounded native process in a writable staging
directory. It generates native GEM inputs, preserves runtime Lua setup,
exports pre-solve PAs, IOB node-to-workbench transforms, native default and
strict potentials, field probes, and three independent refine timings.
It invokes no flight or plotting callback and never saves installed inputs.

The runtime details preserved include C-magnet gap=20 (not the GEM fallback
30), mu=1000 and I=2000; current-sphere Q=1 in 2D and Q=1000 in 3D; the
literal `z+0.5` term in the 2D sphere; `field_vc/ng` permanent-magnet
sources; segmented Halbach magnetization; the quadrupole's literal -0.25
permeability shift; the sphere source radius 9.75 and native point
rasterization; mu-label replacement by 5 or 10; the mucurrent cylinder's
shifted center and 20000 top pole; and the solenoid current GEM's absence
of a half-cell shift. Cartesian component parity is (y,x,xy). Dual
scalar/vector examples are audited separately. The uniform scalar is
load-refined, not reported as an explicit timed Lua refine.

SIMION rejects a raw convergence objective below 1e-7. For smaller physical
strict objectives the harness scales poles and sources together before
refine and unscales the result afterward: scale 1000 normally and 100 for
the 3D current case, where scale 1000 hits native roundoff. The largest
201-cubed magnetized sphere retains unscaled 1e-7 strict refinement because
scaled runs exceed the 180-second per-process limit. Strict disables
skipped-point refinement. A native step-change objective is not a physical
residual tolerance.

`crates/simrep-cli/examples/magnetostatics_oracle.rs` solves each exported
problem against a CPU 1e-12 relative-residual reference. For two
high-contrast cases that reach an f64 residual floor, it independently
solves from the native seed and requires agreement within 64 potential
ulps before accepting the reference; the measured floor is printed.
Uniform fields use their exact affine analytic reference. Candidate
solves use 1e-10, or 1e-15 for the uniform precision control.

The audit reports candidate error, native default error, default-minus-
strict numerical change, strict-minus-converged difference, and the
native strict residual under our operator. For cylindrical scalar sources
it also solves an arithmetic-source comparison model, separately
quantifying source-quadrature model differences and remaining native
iteration error. Native field-probe differences additionally include
SIMION's field interpolation versus the C2 reconstruction; they must not
be called solver residual error. Flights through candidate and converged
fields compare endpoint and velocity and check magnetic energy.

All timing controls use at least three repetitions and exclude file IO,
GEM creation, field reconstruction, and source/material preparation.
Hierarchy setup, iteration computation, and transfers are reported
separately. The native Refine call does not expose its internal setup
split, so both iteration-only and setup-plus-iteration comparisons must be
read with that limitation. These runs are **contended**: CPU checks,
software Vulkan tests, an external Opera job, and two busy live-parent
Windows conhosts were observed. They are not quiet-machine acceptance
numbers or a universal speedup claim.

SIMION's `spotential_from_bfield` constructs a scalar potential by line
integration of a prescribed B, rather than solving these PDEs. That
integration utility is outside this solver/flight increment.

## Audit results, October 24, 2026

All 20 PDE example stems were exported and compared: 25 component/scalar
solves and 23 combined-field flights, including the load-refined uniform
scalar. All 25 candidates matched or improved the SIMION **default** maximum
stored-potential error against the converged/analytic reference. The strict
error column is a separate control; candidate timings are not claimed to
match every stricter control's precision.

The table includes CPU hierarchy setup plus iteration computation, excluding
source/material preparation and file IO. Each phase uses a median of three;
the CPU entry sums the setup and iteration medians. Cartesian A combines
its three component phase medians, rather than comparing one
component with the native three-component call. Errors are maximum stored
p/A/psi differences, with each PA's original ng retained. These are the
contended controls described above, not quiet-machine acceptance results.

| Case / field | CPU setup+solve s | SIMION default s | SIMION strict s | Ratio default/CPU | Our error | Default error | Strict error |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| c_magnet_2dp / Az | 0.310451 | 4.206 | 11.726 | 13.55 | 5.49e-6 | 1.86e-1 | 3.27e-7 |
| current_cylinder_2dp / Az | 0.007747 | 0.180 | 0.591 | 23.23 | 5.94e-11 | 1.82e-2 | 1.73e-7 |
| current_sphere_2dc / psi | 0.015866 | 0.091 | 0.188 | 5.74 | 4.58e-10 | 3.62e-2 | 4.69e-8 |
| current_sphere_3dp / Ax+Ay+Az | 0.421483 | 2.026 | 5.479 | 4.81 | 3.06e-7 | 7.82e-2 | 8.30e-7 |
| emag_quad_2dp / Az | 0.064575 | 0.431 | 0.867 | 6.67 | 2.27e-7 | 1.94e-2 | 1.81e-7 |
| mag_2cylinder_2dc / base | 0.130068 | 0.822 | 7.713 | 6.32 | 1.58e-13 | 5.32e-4 | 1.68e-6 |
| mag_2ring_2dc / base | 0.115160 | 0.697 | 7.695 | 6.05 | 2.41e-12 | 4.57e-4 | 1.68e-6 |
| mag_cylinder_2dc / base | 0.061099 | 0.370 | 1.267 | 6.06 | 1.94e-13 | 4.35e-4 | 4.18e-7 |
| mag_halbach_cylinder_2dp / base | 0.006907 | 0.155 | 0.225 | 22.44 | 1.03e-11 | 5.80e-5 | 6.59e-8 |
| mag_quad_2dp / base | 0.460006 | 1.155 | 20.671 | 2.51 | 1.18e-12 | 8.41e-4 | 1.32e-5 |
| mag_ring_2dc / base | 0.108884 | 0.654 | 7.056 | 6.01 | 2.63e-12 | 3.12e-4 | 1.68e-6 |
| mag_sphere_2dc / base | 0.032802 | 0.228 | 0.641 | 6.95 | 7.84e-14 | 2.57e-4 | 2.19e-7 |
| mag_sphere_3dp / base | 1.759477 | 7.619 | 51.867 | 4.33 | 3.55e-10 | 2.80e-4 | 4.03e-4 |
| mu_cylinder_2dp / base | 0.048512 | 0.173 | 3.685 | 3.57 | 1.23e-7 | 2.85e-2 | 3.37e-7 |
| mu_cylinder_2dp / Az | 0.041684 | 0.188 | 2.186 | 4.51 | 2.36e-6 | 2.68e-2 | 3.45e-7 |
| mu_sphere_2dc / base | 0.072604 | 0.139 | 1.954 | 1.91 | 7.04e-8 | 1.73e-2 | 3.48e-7 |
| mu_sphere_2dc / Az | 0.131499 | 0.237 | 1.261 | 1.80 | 6.30e-8 | 2.73e-2 | 2.29e-7 |
| mu_sphere_3dp / base | 0.490606 | 0.824 | 11.213 | 1.68 | 3.50e-7 | 8.08e-3 | 1.23e-7 |
| mu_torus_3dp / base | 0.312333 | 0.981 | 5.663 | 3.14 | 7.12e-8 | 1.22e-2 | 1.23e-7 |
| mucurrent_cylinder_2dp / Az | 0.096310 | 1.006 | 12.292 | 10.45 | 1.55e-3 | 1.02e-1 | 1.93e-5 |
| solenoid_2dc / psi | 0.049384 | 0.131 | 0.201 | 2.65 | 5.43e-8 | 3.21e-2 | 4.29e-8 |
| uniform_2dp / Az | 0.131731 | 0.024 | 1.654 | 0.18 | 1.42e-14 | 3.39e-10 | 3.42e-7 |
| uniform_2dp / base | 0.102812 | load only | load only | - | 1.42e-14 | 6.57e-8 | 6.57e-8 |

The 21 nonuniform timed controls were 1.68--23.23 times faster than native
default at matched-or-better default potential error. The original uniform
Az precision control was a counterexample: 0.132 s versus 0.024 s, about
5.49 times slower. Its cause and fix are measured below; this historical
table is not a universal faster-than-SIMION claim.

Flights used 0.1 us and a 0.001 us ceiling. Maximum candidate-minus-converged
endpoint difference was 5.999e-11 mm and velocity difference 1.203e-9 mm/us;
maximum relative kinetic-energy error was 6.44e-15. Native B probes are not
bit-identical to C2 reconstruction: strict-probe maximum vector differences
include 17.80 G for emag_quad, 14.74 G for c_magnet, and 1.0035 G for the
solenoid. These include interpolation and interface sampling differences;
the potential-error speed gate must not be presented as an equal-B-probe
or equal-native-flight accuracy claim.

The separate conservative-M model is measurably different from the literal
Lua-derived source, even after both numerical solves converge:

| Case | Max conservative-M minus literal-source p |
| --- | ---: |
| mag_2cylinder_2dc | 7.84898e-3 |
| mag_2ring_2dc | 7.76250e-3 |
| mag_ring_2dc | 7.74133e-3 |
| mag_halbach_cylinder_2dp | 3.43282e-2 |
| mag_quad_2dp | 6.25420e-3 |

Thus reproducing these examples requires their precomputed/native
field_vc/ng source, not silently replacing it with the conservative M
constructor. The cylindrical finite-volume versus arithmetic source
comparison changes p by 4.06e-5--1.55e-4; native residuals under the
arithmetic model remain 1.56e-3--1.79e-2 relative. This is distinct from the
much smaller residual under the half-dual-volume source model.

Final checks: cargo fmt --all -- --check, workspace all-target clippy with
-D warnings, explicit simrep-py all-target clippy, and cargo test --workspace
--release passed (265 tests; baseline 251 plus 14 new). Prepared GPU Poisson
passed Vulkan native-f64 and CPU-residual fallback for planar, axis, and
annular grids, with maximum CPU disagreement 2.45e-12. The available Vulkan
adapter was software llvmpipe; DX12 was unavailable on Linux. No hardware-GPU
speed claim is made. LSP diagnostics could not be obtained because the
shared daemon repeatedly timed out; compiler and clippy checks were clean.

Final `winproc.ps1 cleanup` receipt:

```text
orphan conhost: 0
busy conhost (>50% of a core): 2
stopped orphan conhost: 0
```

No SIMION process was listed. The two busy conhosts had live WSL parents
(the project session and the external Opera session) and were preserved.

## Hardware GPU and uniform-field follow-up, October 24, 2026

The Windows RTX 3070 ran the physical and prepared Poisson harness on DX12
with CPU f64 residuals, and Vulkan with both native f64 and CPU f64
residuals. All 18 checks passed, including planar, regular-axis, and annular
boundaries. Maximum prepared-stencil disagreement with CPU at relative
residual 1e-12 was 2.44249e-12 (DX12: 2.43894e-12). No Poisson kernel change
was needed. The full Windows Vulkan magnetostatics oracle also completed:
25 potential controls matched or improved native default error, and all
23 field flights completed. This includes the existing independently
certified f64-floor controls, not a relaxation of their acceptance bound.

`magnetostatics_profile` reproduces the compute measurements from the same
export directory:

```sh
cargo run --release -p simrep-solve --example magnetostatics_profile -- \
  <export-directory> <case> <cpu|vulkan|fallback|dx12> [relative-tolerance]
```

Here `fallback` forces CPU residual evaluation on Vulkan even when the GPU
supports f64. Each entry below is a median of three, in seconds, shown as
**hierarchy setup / solve computation**. Solve includes transfers, residuals,
and host synchronization; it is not a timestamp-only kernel number. PA IO,
physical source/material preparation, and field reconstruction are excluded.
The CPU process requests 16 Rayon workers; the existing CPU solver caps
grids of at most 1,048,576 nodes at four workers. No build or other solver
was running. The idle check found GPU utilization 0% and only two busy
live-parent Windows conhosts, each consuming one CPU core; neither was
killed. These are not completely idle-machine acceptance numbers.

| Control | Nodes | CPU16 | Vulkan native f64 | Vulkan CPU residual | DX12 CPU residual | Max GPU-minus-CPU potential |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| mu_torus_3dp | 520,251 | .025701 / .162870 | .013047 / .112232 | .010405 / .147722 | .020214 / .162933 | 1.94e-7 |
| current_sphere_3dp, Ax+Ay+Az | 1,030,301/component | .082317 / .271386 | .047784 / .152076 | .039356 / .229448 | .062493 / .248091 | 3.25e-7 |
| mu_sphere_3dp | 1,030,301 | .037842 / .227035 | .016655 / .108973 | .012989 / .178689 | .020520 / .181274 | 6.90e-7 |
| mag_sphere_3dp | 8,120,601 | .120678 / 1.197812 | .066249 / .261923 | .066540 / .702827 | .069900 / .607692 | 2.70e-10 |

All these solves used the same 1e-10 physical residual target. The last
column compares against CPU at that target, separately from the stricter
oracle references. Against the oracle references, Vulkan maximum errors
for these controls were respectively 2.62e-7, 1.00e-7, 1.03e-6, and
6.03e-10, all below their native default errors.

The previous shared linear crossover (1,048,576 nodes, 3D only) incorrectly
left the first three controls on CPU. Magnetic refinement still calls
`requests_gpu(grid, Some(false))`, but extends that policy using only the
measured magnetic brackets: 520,251 nodes with native f64, 1,030,301 without.
DX12 solve time is essentially tied with CPU at the smaller bracket, so
automatic selection without native f64 conservatively stays on CPU there.
Automatic backend selection chose Vulkan on this machine.
The extension is confined to magnetostatics; dielectric policy is unchanged.
Explicit devices still override the policy, and software adapters remain
excluded from automatic GPU selection. Two-dimensional controls keep the
existing CPU policy.

### Uniform Az root cause and fix

The uniform PA is only 201 by 201 (40,401 nodes), with constant Dirichlet
data 0 and 100 on its two y faces, homogeneous mirror conditions on the x
faces, vacuum coefficient 1, and zero current. Its exact solution in stored
PA units is `Az(x,y)=100*y/200`. No curved geometry or boundary approximation
is involved.

The zero interior seed unnecessarily iterated toward this affine field.
A fresh Linux CPU16 profile (the solver uses four workers at this size)
at the unchanged 1e-15 target measured 27 cycles: 1.389 ms setup and
72.935 ms solve. Smoothing accounted for about 45 ms, residual evaluation
15 ms, prolongation 9 ms, and restriction 2.5 ms. Setup was not the cause.
Using the ordinary 1e-10 target reduced this to 11 cycles but left a
2.246e-6 maximum analytic error, much worse than native default's
3.387e-10. Loosening the tolerance therefore does not meet the precision
gate. SIMION's default 1e-5 step-change objective is not our physical
residual target; its unusually accurate result here must still be matched.

For a homogeneous planar vacuum operator with exactly two complete,
constant Dirichlet faces and no source, magnetic CPU solving now starts
from their affine interpolant. It then runs the **same** physical-residual
check and, if needed, the normal iterative solver. This is independent of
the example name and works along each active axis in 2D/3D. Fixed values
are preserved, material/source problems take their previous path, and
`max_cycles=0` remains an unmodified residual query. Interpolation work is
charged to solve computation, not hidden in source preparation.

| Uniform Az, median of three | Before | After |
| --- | ---: | ---: |
| Setup | 1.389 ms | .443 ms |
| Solve computation | 72.935 ms | .261 ms |
| Setup + solve | 74.324 ms | .704 ms |
| V-cycles | 27 | 0 |
| Maximum analytic error | 1.421e-14 | 0 |
| Physical relative residual | 8.527e-16 | 0 |

The measured setup-plus-solve improvement is 105.6 times; the revised
control is 34.1 times faster than the recorded 24 ms SIMION default call
at better precision. Solve-only time is 91.9 times faster than that native
call, whose internal setup split is unavailable. The uniform scalar
control also takes zero cycles with zero analytic error.

### Follow-up verification receipt

`cargo fmt --all -- --check`, workspace all-target Clippy with `-D warnings`,
and `cargo clippy -p simrep-py --all-targets --features python -- -D warnings`
all exited 0. `cargo test --workspace --release` exited 0: 274 tests
(271 baseline plus three regressions), with the standalone Linux GPU
harnesses also passing on llvmpipe and explicitly skipping unavailable DX12.
Windows magnetostatics passed all 10 tests. Current RTX smoke checks passed
f32 on both backends and native f64 on Vulkan; DX12 explicitly reported
`SHADER_F64: false`. LSP symbol/diagnostic requests timed out at the shared
daemon; compiler and Clippy checks were clean.

Nine native CLI `--device auto` checks selected GPU for the torus on
auto/Vulkan, CPU for the torus on DX12, GPU for the sphere on all three
backends, and CPU for the uniform grid on all three. These used
`max_cycles=0` to inspect routing without writing a PA. Native audit exports
are marked non-refinable, so writable test copies retained their PDE data
but enabled the refinable header flag; the originals were not changed.
A subsequent real uniform CLI refinement wrote its PA and material sidecar
with zero cycles and zero physical residual. The final Windows CPU oracle
also measured zero analytic error for both uniform controls.

All Windows commands were foreground, timeout-bounded, run from `/mnt/c`
with closed stdin, and forwarded `CARGO_TARGET_DIR`/`RAYON_NUM_THREADS`
through `WSLENV` when needed. Final `winproc.ps1 cleanup` exited 0:

```text
orphan conhost: 1
busy conhost (>50% of a core): 2
stopped orphan conhost: 1
```

No `cargo.exe`, `rustc.exe`, `simrep.exe`, or SIMION process remained.
The two busy conhosts still had live WSL parents and were preserved.

