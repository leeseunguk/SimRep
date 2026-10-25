# Variable-permittivity electrostatics

`simrep_solve::dielectric::Permittivity` solves

```text
-div(epsilon_r grad V) = rho_free / epsilon_0
```

for finite, strictly positive, scalar relative permittivity. The zero-charge
case supports CPU f64 and the existing GPU mixed-precision Galerkin solver.
The CPU also supports free volume charge through `solve_poisson`. Coordinates
and spacing use the same length unit as `mg::Grid`: Cartesian 3D/2D, or
cylindrical `(r,z,0)`, including the axis and positive-origin annuli. Fixed
potentials are retained exactly; unmasked outer faces are zero-normal-flux
mirrors. At least one fixed node is required.

## Inputs and CLI

```sh
simrep refine potential.pa --permittivity material.pa --tolerance 1e-10
simrep refine electrodes.pa# --permittivity material.pa --device gpu --backend vulkan
simrep refine potential.pa --permittivity materials.json
```

The CLI defaults to `--device auto`: dielectric 2D/r-z and small 3D problems
use CPU; 3D grids at or above a problem-class threshold use a hardware GPU if
available. The policy is class-aware: linear solves cross at 1,048,576 nodes,
nonlinear solves at 131,072 nodes (the nonlinear GPU path amortizes its
transfers across all inner solves). These are the measured Ryzen 5950X /
RTX 3070 crossovers in `dielectric::prefer_gpu`. A missing adapter or a
software CPU adapter keeps auto on CPU. Explicit `--device cpu` and
`--device gpu` override the choice. Ordinary Laplace refinement retains its
CPU default behavior under auto. Python's `device="auto"` uses the same
dielectric policy; its existing default remains `device="cpu"`.

PA# refinement applies the same material operator to every electrode basis,
so subsequent fast adjustment remains linear in voltage. Changing the material
requires rebuilding all bases.

**A SIMION dielectric PA is a cell array, not a nodal potential array.** If
the potential PA has `(nx,ny,nz)` nodes, the dielectric PA must have
`(max(nx-1,1), max(ny-1,1), max(nz-1,1))` samples. Entry `(x,y,z)` is the
relative permittivity of the primal cell from that vertex to the next vertex,
sampled at its center or averaged over the cell. Every point must be
non-electrode, and symmetry and spacing must match the potential PA. Material
labels are literal values, not electrode numbers. The loader does not refine
the material PA. Cylindrical SIMION arrays use axial-x/radial-y storage; both
material and potential arrays are transposed together to solver `(r,z)`.

This matches the local SIMION help `dielectric.html` and the
`examples/dielectric/*-dielectric.gem` files. Those examples use `n(1)` to fill
vacuum, another `n(epsilon)` to fill the material, and a half-cell negative
coordinate shift so GEM sampling occurs at cell centers. The existing
`simrep gem2pa` command can turn such a GEM into the input material PA.
In the sphere examples, Lua changes the GEM's inside label `5` to
`kinside=10` at fly time; using the unmodified PA means epsilon is **5**, not
10. Our solver does not execute this Lua.

The additive JSON alternative describes SDF regions, in mm:

```json
{
  "background": 1,
  "origin_mm": [0, 0, 0],
  "regions": [
    {
      "epsilon": 3,
      "shape": {"kind": "plane", "normal": [-1, 0, 0], "offset": -50}
    }
  ]
}
```

Here epsilon is 3 on `x >= 50`. `origin_mm` defaults to zero and is the
physical position of potential node zero. JSON coordinates are solver
coordinates, including `(r,z,0)` for cylindrical inputs, not SIMION's axial
storage order. Shapes support `sphere`, `cylinder`, `box`, `plane`, and
`union`/`intersection`/`difference` with `a` and `b` children. Fields match
`simrep_geom::Sdf`: `center`, `radius`, `axis` (0..2), `half_extent`, or unit
`normal` and `offset`. Unknown fields and invalid dimensions are rejected.
Regions are sampled at potential nodes; later regions win in overlaps.

## Rust and Python

```rust
use simrep_solve::dielectric::Permittivity;

// epsilon_nodes is x-fastest and has one value per potential node.
let material = Permittivity::from_nodes(grid, &fixed, &epsilon_nodes)?;
let report = material.solve(&mut potential, options)?;
// Optional CPU charge: source = rho_free / epsilon_0, not rho / epsilon.
let report = material.solve_poisson(&mut potential, &source, options)?;
// GPU uses the same prepared coefficients, including native-f64 residuals
// where available, or the existing CPU-f64 residual fallback.
let report = material.solve_gpu(&gpu, &mut potential, options)?;
```

`from_cells` accepts the SIMION cell layout directly. `from_regions` accepts a
background value, origin, and `Region` values containing any `SignedDistance`
implementation, including imported GEM sampled SDFs. Preparation is reusable
across guesses and boundary voltages, but the fixed-node mask cannot change.
`dielectric::load` is the shared PA/JSON loader used by the CLI and Python.

```python
field = simrep.refine("electrodes.pa#", permittivity="material.pa",
                      device="cpu", tolerance=1e-10)
adjusted = field.fast_adjust({1: 100, 2: 0})
```

Python also accepts a JSON specification path and in-memory potential PAs.
The native computation releases the GIL as ordinary refinement does.
With mm coordinates and charge density in C/mm^3, the Rust source array uses
`rho_free / 8.8541878128e-15` (epsilon_0 in F/mm). Sources at fixed nodes are
ignored. GPU free-charge forcing is not exposed by this additive path.

## Discretization and interfaces

For nodal epsilon, a link uses the harmonic mean
`2 / (1/epsilon_i + 1/epsilon_j)`. This is the series resistance of two
half-links: voltage is single-valued at nodes, and normal displacement is
conserved on a shared link. A planar material boundary represented between
unlike nodes lies at their midpoint. Geometry sampling does not find a
subcell material boundary; its interface-position error is generally first
order. This path does not claim second-order accuracy for arbitrary curved
dielectric interfaces.

For SIMION primal-cell epsilon, each node-to-node link lies inside one cell
in its longitudinal direction. Its coefficient is the arithmetic area
average of the cells touching it transversely (parallel conductances), not a
harmonic average of transverse materials. This distinction preserves an
interface aligned with potential nodes: the link on one side uses epsilon_1
and the link on the other uses epsilon_2. Applying a nodal harmonic mean to
the PA samples would shift and smear that interface. Cylindrical links use
the existing radial geometric factors and mirror/axis quadrature.

Every undirected link is evaluated in the same direction in both rows.
Thus `W_i a_ij = W_j a_ji > 0`; the reduced operator is symmetric positive
definite after fixing the gauge. The existing transpose restriction,
Galerkin `P^T A P`, and Chebyshev/Jacobi smoothers apply without new shader
code. Sparse exceptional fine rows reuse the established cut-cell storage;
the ordinary Laplace path is unchanged, and epsilon=1 reproduces it
bit-for-bit in the regression test. A material solve does not also apply
continuous electrode cut cells.

The reported absolute residual is the unweighted physical
`div(epsilon_r grad V) + source` infinity norm, not a residual divided by local
epsilon. The relative scale includes epsilon-weighted Dirichlet forcing and
the free-charge source. Accuracy near a material interface must be assessed
with one-sided fields or fluxes: ordinary potential interpolation smooths
the discontinuous electric field, just as noted by SIMION's help.

## Validation

```sh
cargo test -p simrep-solve --release --test dielectric -- --nocapture
cargo test -p simrep-solve --release --test dielectric_gpu
cargo test -p simrep-cli --release --test dielectric_cli
cargo run -p simrep-solve --release --example dielectric_oracle -- \
  /mnt/c/Users/lsu10/simrep-oracle/dielectric cpu
```

Analytic tests use residual tolerance `1e-12`, independent of SIMION:

| Case | Grids | Measured error | Last-pair order |
| --- | --- | --- | --- |
| Grid-aligned dielectric slab, 2D/3D, epsilon ratio 3 | 33 x 5 x 1/4 | max potential < 3.7e-12 V; both sides D agree within 1e-8 | Exact piecewise-linear solution to solver tolerance |
| Off-grid nodal slab at x=1/3, 2D/3D | nx=17,33,65 | max V: 7.23938e-3, 3.75000e-3, 1.88758e-3 | 0.99035 |
| Coaxial sleeve, r=1..3, interface r=2, epsilon ratio 4 | nr=17,33,65 | max V: 1.01109e-4, 2.53573e-5, 6.34436e-6 | 1.99885 |
| Dielectric sphere in uniform field, epsilon=8, exact outer Dirichlet field | 17^3,33^3,65^3 | RMS V: 5.30174e-3, 1.20048e-3, 3.74634e-4 | 1.68005 |

The sphere's center fields are 0.320390, 0.312564, and 0.307031 V/length,
converging toward the analytic 0.3. The free-charge test uses a piecewise
quadratic solution with continuous voltage and epsilon-weighted slope and
checks a nonzero-RHS warm start. Separate tests check weighted-link symmetry
at mirrors, the axis, and annuli; invalid coefficients; fixed values;
CLI cell-array permutation; JSON materials; and Python dielectric basis
superposition.

The read-only `dielectric_oracle` example loads `plates_2dp`, `sphere_2dc`,
and `sphere_3dp`, applies the same sphere-inside epsilon=10 as the example
Lua, and compares the result at tolerance `1e-12` to a CPU `1e-13` converged
discrete reference. It prints the full-grid maximum voltage difference,
center field, and three-repetition median setup/solve/upload/readback times.
It does not modify the licensed example
files. Preserved SIMION runs are dated October 23, 2026:

| SIMION input | Printed analytic E(center) | Printed actual E(center), V/mm | SIMION reported refine compute |
| --- | --- | --- | --- |
| `sphere_2dc` | 0.25 | 0.25654121405188 | 0.15 s, convergence 1e-5 V |
| `sphere_3dp` | 0.25 | 0.24873766211532 | 0.57 s, convergence 1e-3 V |
| `plates_2dp.pa#` | Piecewise-linear slab | No field check printed | 0.32 s, two nonzero electrode bases |

These are solver-reported compute intervals, not process wall times.
The example reports preparation, hierarchy/shader setup, and solve separately;
startup, input reading, and output writing are excluded. Plate timings are
not a like-for-like speed ratio: the example solves one 100 V boundary
problem, whereas SIMION refines two bases. Sphere theory is the infinite
uniform-field solution; the shipped examples instead use a finite mirrored
box, so theory/actual differences include truncation and interface sampling.
Do not use SIMION agreement alone as an accuracy reference.

### Strict oracle and compute medians (October 24, 2026)

`crates/simrep-solve/examples/dielectric_strict.lua` runs the same three
ordinary PA boundary problems with SIMION's minimum accepted convergence
objective, `1e-7` V, from a zero free-node guess, three times each. Run it
with headless SIMION from the writable dielectric example directory. It
saves separate `*.simrep-strict.pa` snapshots without overwriting inputs;
`dielectric_oracle` compares those snapshots with its tighter CPU reference
when present. This is the matched-precision comparison, rather than the
shipped sphere_3dp `1e-3` setting.

The strict SIMION center fields are 1.000000308070 V/mm for the slab,
0.256607403377 for sphere_2dc, and 0.257183602977 for sphere_3dp.
The corresponding converged native values are 1.000000000002,
0.256607750896, and 0.257183937462. The large change from the shipped
sphere_3dp printout, 0.24873766211532, is therefore convergence error, not
evidence that its coarse result is a more accurate dielectric reference.
The finite-box result itself still differs from the infinite-domain sphere
theory 0.25; the independent analytic-grid convergence test above separates
that issue.

Windows Vulkan selected the discrete NVIDIA GeForce RTX 3070. At residual
tolerance `1e-12`, these are
medians of three fresh solves per case, excluding I/O and process startup:

| Case | SIMION strict refine s, all repetitions | Median SIMION s | Median GPU solve s | Median GPU hierarchy/shader setup s | Median upload / readback s |
| --- | --- | --- | --- | --- | --- |
| plates_2dp, one 100 V problem | 0.17, 0.13, 0.13 | 0.13 | 0.164317 | 0.036306 | 0.001441 / 0.008949 |
| sphere_2dc | 1.27, 0.43, 0.42 | 0.43 | 0.260937 | 0.018825 | 0.000711 / 0.009383 |
| sphere_3dp | 27.82, 25.78, 24.60 | 25.78 | 0.940902 | 0.063119 | 0.014709 / 0.023664 |

Material preparation was 0.015966, 0.010211, and 0.358755 s respectively
(one preparation, reused by the three solves). It is compute work and must
be included along with hierarchy setup when comparing one-shot total compute;
solve-column ratios alone are not total-compute speedups. Small problems do
not uniformly favor the GPU. The GPU solve ranges were 0.149719..0.230270,
0.255938..0.292033, and 0.774513..0.950755 s.

**Timing caveat:** the GPU slot was reserved, but strict SIMION CPU runs
overlapped it and host transfer timings remained variable (one small upload
17.6 ms; one 3D readback 64.0 ms). These medians are measured values with
host-side contention, not isolated throughput claims. They were retained
rather than requesting another timing slot.

GPU results differed from the `1e-13` CPU reference by at most 1.69e-8,
3.23e-9, and 2.59e-9 V respectively. Strict SIMION's full-grid maximum
differences were 8.26e-5 V for the slab, 4.63e-4 V for sphere_2dc, and
6.80e-4 V for sphere_3dp.
SIMION's per-iteration voltage-change objective is not a bound on global
solution error; both solvers' results must be compared to references.

Linux CPU solve medians, also three fresh solves, were 0.023405,
0.041426, and 1.088561 s respectively. Median hierarchy setup was 0.003129,
0.002950, and 0.059507 s; one material preparation took 0.008833, 0.008032,
and 1.195561 s. These are not isolated CPU throughput measurements either.
The CPU is preferable for the small plate and r-z examples. The converged
3D reference agrees with the GPU to 2.59e-9 V across the full grid, and both
agree with strict SIMION's center field within 3.35e-7 V/mm.

### Automatic selection at the default residual tolerance

The following additional measurements use **the same `1e-10` relative
residual on both CPU and GPU**, three fresh solves per row. The benchmark's
optional last argument selects that tolerance:

```sh
cargo run -p simrep-solve --release --example dielectric_oracle -- \
  /mnt/c/Users/lsu10/simrep-oracle/dielectric cpu 1e-10
# Use Windows Vulkan per docs/dev/windows-gpu.md for the GPU rows.
```

| Case | Device | Auto choice | Median solve s | Median hierarchy/setup s | Material preparation s, once | Strict SIMION median s |
| --- | --- | --- | --- | --- | --- | --- |
| plates_2dp | CPU f64 | Selected | 0.053610 | 0.003050 | 0.008300 | 0.13 |
| plates_2dp | Vulkan GPU | | 0.077542 | 0.010408 | 0.075188 | 0.13 |
| sphere_2dc | CPU f64 | Selected | 0.161406 | 0.013060 | 0.009803 | 0.43 |
| sphere_2dc | Vulkan GPU | | 0.141039 | 0.048220 | 0.386761 | 0.43 |
| sphere_3dp | CPU f64 | | 1.501676 | 0.113046 | 1.859506 | 25.78 |
| sphere_3dp | Vulkan GPU | Selected | 0.286475 | 0.174082 | 0.640721 | 25.78 |

Auto chooses CPU for the small examples because GPU setup/preparation and
transfer overhead outweigh a possible solve-only advantage. Selected solve
medians are below strict SIMION for all three examples. Including the
one-time material preparation and median hierarchy setup gives phase sums
0.064960, 0.184269, and 1.101278 s respectively, still below strict SIMION.
These sums are not medians of whole one-shot executions.

GPU median upload/readback times were 0.001336/0.031385,
0.001041/0.054281, and 0.014139/0.298648 s. Including those transfers in
the selected 3D phase sum gives 1.414065 s. **This run remained contended**
(r-z first readback 340 ms; active unrelated Windows cargo/rustc processes),
so the phase table is measured evidence, not an isolated-machine speed
guarantee. No further timing retry was made.

At `1e-10`, full-grid maximum voltage differences from the `1e-13` CPU
reference were 1.48e-6 / 5.34e-7 / 8.89e-8 V on CPU and
3.52e-6 / 3.75e-7 / 2.31e-7 V on GPU (plates / r-z sphere / 3D sphere).
Thus automatic selection does not trade the requested accuracy for speed.
Routing tests cover both overrides, the small and large measured grids,
ordinary Laplace behavior, and Python automatic CPU basis refinement.

Windows release tests also ran the old Laplace/cut-cell GPU suites and
24 new dielectric comparisons: 3D/2D planar, cylindrical axis and annulus,
cell/nodal materials, DX12 CPU-f64 fallback, and Vulkan native/fallback
residuals. The largest CPU/GPU difference was 1.31e-10 V. Linux
`cargo fmt --all -- --check`, both required clippy commands with
`-D warnings`, `cargo test --workspace --release`, and all 27 Python tests
passed. The LSP daemon did not answer its diagnostics requests; compiler and
clippy diagnostics were clean. Final Windows cleanup reported zero orphan
consoles and no task-owned cargo/rustc/simrep processes.

## Field-dependent isotropic permittivity

`NonlinearPermittivity` solves the self-consistent zero-free-charge problem
with cell-centred `epsilon(|E|)`. It reuses the CPU f64 or GPU mixed-precision
linear multigrid solver inside a relaxed Picard iteration. The coefficients
are rebuilt from the current field; a growing physical nonlinear residual
halves the potential relaxation down to 1/128. Fixed voltages are retained.
Failure of the inner solve is an error; exhaustion of the outer iteration
budget is reported as nonconvergence, not a usable converged solution.
Arbitrary positive constitutive laws need not define a unique elliptic
problem; damping does not establish uniqueness.

Convergence requires **both** the physical residual re-evaluated with
`epsilon(E)` and the relative infinity norm of the change in the vector
field at the material samples. A small lagged linear residual alone cannot
terminate the nonlinear solve. The residual tolerance is the ordinary
`--tolerance` / Python `tolerance`; independent outer controls are in the
JSON specification.

The existing `--permittivity` argument and Python `permittivity=` accept this
additive JSON form. The cell PA path is relative to the JSON file, and must
obey the same cell dimensions, symmetry, spacing, and non-electrode rules as
linear PA materials:

```json
{
  "nonlinear": {
    "cell_permittivity": "sphere_nonlinear_3dp-dielectric.pa",
    "laws": [
      {
        "epsilon0": 6,
        "law": {"kind": "saturating_quadratic", "lambda": 8e-11, "minimum": 1}
      }
    ],
    "sampler": {
      "kind": "sphere_offset",
      "center": [50, 0, 0],
      "radius": 10,
      "offset": 0.5
    },
    "options": {
      "max_iterations": 100,
      "field_tolerance": 1e-8,
      "relaxation": 1
    }
  }
}
```

Laws select exact zero-field cell labels; unlisted labels stay linear.
`saturating_quadratic` evaluates
`max(epsilon0*(1-lambda*|E|^2),minimum)`. Fields are in V per solver length
unit, hence **V/mm for PA inputs**. The SIMION example's lambda `8e-17`
with E in V/m therefore becomes `8e-11` here. A second law,
`{"kind":"table","points":[[0,2],[100,3],[1000,4]]}`, linearly interpolates
positive epsilon values at strictly increasing nonnegative field magnitudes,
and holds the endpoint values outside the table. Unknown fields, duplicate
labels, invalid knots, nonpositive epsilon, and invalid controls are rejected.
The default sampler is `{"kind":"cell_center"}`.

The sphere sampler implements the example Lua's radial projection: samples
within `offset` of the material radius move to `radius-offset` inside or
`radius+offset` outside. Coordinates are physical solver coordinates relative
to node zero. Fields are the negative derivative of the multilinear nodal
potential at that point, not an interpolation mixing both material sides.
This reconstruction and SIMION's `field_vc` are separately implemented;
agreement with a SIMION iterate is not used as the reference.

Nonlinear fast-adjust bases are invalid: changing electrode voltage changes
epsilon and destroys linear superposition. CLI PA# and Python
`adjustable=True` reject nonlinear specifications. Refine an ordinary PA at
its actual voltages; the returned Python field has no adjustable bases.
Python reports include `relative_field_change`.

```rust
use simrep_solve::dielectric::{
    FieldLaw, FieldSampler, NonlinearOptions, NonlinearPermittivity,
};
let material = NonlinearPermittivity::from_cells(
    grid, &fixed, &epsilon0,
    vec![(6.0, FieldLaw::SaturatingQuadratic { lambda: 8e-11, minimum: 1.0 })],
    FieldSampler::CellCenter,
)?;
let report = material.solve(&mut potential, options, NonlinearOptions::default(), None)?;
// Pass Some(&gpu) to use the existing GPU linear inner solver.
```

The shipped `sphere_nonlinear_3dp.lua` actually runs **20** iterations,
at `convergence=0.1` V, enabling skipped points only for the first five.
Its saved log ends at `k(center)=5.1166090275215`. The retained 106.8 s
SIMION fly compute includes updates and repeated refinements, not one
linear solve. `nonlinear_dielectric_oracle.lua` repeats the same 20-update
problem three times without redraw or file I/O inside its compute interval,
and saves only a separately named snapshot.

Independent release tests cover a layered quadratic-law capacitor whose
constant displacement is obtained from a scalar cubic, and a nonlinear
coaxial capacitor with `epsilon=1+|E|`, using quadrature of
`E(r)=(sqrt(1+4*C/r)-1)/2` as the reference. The layered test's maximum
voltage error is below 5.6e-12 V and its flux error below 3.4e-11.
The coaxial maximum errors on 17/33/65 radial nodes are
7.52722e-5 / 1.88672e-5 / 4.71989e-6 V: last-pair order 1.99905.
A nonlinear GPU-inner test differs from CPU by 8.0e-13 V on Linux
software Vulkan; this is numerical coverage, not hardware performance.

```sh
cargo test -p simrep-solve --release --test nonlinear_dielectric -- --nocapture
RAYON_NUM_THREADS=4 cargo run -p simrep-solve --release \
  --example nonlinear_dielectric_oracle -- \
  /mnt/c/Users/lsu10/simrep-oracle/dielectric cpu 1e-10
```

The oracle tightens its own CPU reference from relative residual `1e-13`
to `2e-14` and measures the change. Three fresh runs include constitutive
sampling, coefficient preparation, all hierarchy rebuilds, and GPU transfers
where applicable; input/output and process startup are excluded.

### Nonlinear sphere results (October 24, 2026)

The shipped 201 x 101 x 101 potential grid, 0.5 mm spacing, original
cell labels, 10 MV/0 V plates, and one-sided sphere sampler were retained.
The tighter CPU reference reached nonlinear relative residual
`9.68575e-15`; tightening changed the full-grid potential by at most
`2.14204e-8 V`. Timed native runs use nonlinear residual `1e-10` and field
change tolerance `1e-10`, with four Rayon workers.

| Solver | Fresh compute repetitions, s | Median compute, s | Numerical error vs that solver's own converged discrete model |
| --- | --- | --- | --- |
| SIMION, example's 20 updates / 0.1 V refine objective | 96.138, 87.494, 61.164 | 87.494 | 291.098 V |
| Linux CPU f64, converged nonlinear solve | 19.117463, 15.189213, 13.801389 | 15.189213 | 4.03011e-5 V |
| Windows Vulkan RTX 3070, converged nonlinear solve | 13.440915, 12.202519, 10.576639 | 12.202519 | 3.15160e-5 V |

These whole-computation medians include coefficient and hierarchy work,
not just the inner-solve phase. CPU took 18 outer iterations / 31 total
V-cycles, GPU 19 / 40. Their final nonlinear relative residuals were
`3.16647e-11` and `2.43277e-11`; final field change was zero because the
last inner warm start already met its tolerance.

The default-SIMION-to-native full-grid RMS difference was 170.872847 V,
not SIMION numerical error against its own discrete model. Its center field was
42899.999663 V/mm; native CPU/GPU fields were 42905.403986 /
42905.403983 V/mm. The retained example printout's center permittivity
`5.11660902752` was reproduced. SIMION uses a voltage-change stopping
objective and a fixed number of material updates; it is not a converged
nonlinear reference. Differences also include the independent field
reconstruction near the sphere interface.

At lower numerical error within their own discrete models, these measured CPU/GPU compute medians are
5.76x / 7.17x faster than the newly measured SIMION median, or
7.03x / 8.75x faster than its previously retained 106.8 s fly compute.
The large grid crosses the existing automatic GPU-selection threshold.

**Contention:** this is a shared workstation, not an isolated throughput
measurement. An earlier GPU triplet was 8.705 / 8.832 / 21.313 s;
the slow third run prompted a bounded retry (14.463 / 16.706 / 15.705 s).
The final-source triplet in the table was then measured after making
residual evaluation read-only. GPU utilization was zero immediately before
the first run and retry, but host compilation/tests and other workers
remained active. The ranges and all retries are retained rather than
selecting the fastest triplet.

The final Windows cleanup reported `orphan conhost: 0` and
`stopped orphan conhost: 0`, with no cargo/rustc/simion/simrep executable
left from this task. It reported two busy consoles parented by existing
WSL sessions; those unrelated live processes were left untouched.

Final-source verification passed `cargo fmt --all -- --check`,
`cargo clippy --workspace --all-targets -- -D warnings`,
`cargo clippy -p simrep-py --features python --all-targets -- -D warnings`,
`cargo test --workspace --release`, and all 28 Python tests. Python's
CLI comparisons used `SIMREP_BIN` to select the isolated build directory.
The language-server daemon timed out for all three changed Rust surfaces;
compiler and clippy diagnostics were clean. No commits were created.

### Physical grid-refinement audit (October 24, 2026)

**The preceding 5.76x / 7.17x ratios are not matched-physical-accuracy
ratios.** The earlier model audit separated a 424.769 V maximum converged
SIMION/native model difference from SIMION's 291.098 V default-to-own-
converged numerical error. The constitutive law, units, cell labels, and
radial projection agree. SIMION evaluates its constitutive field using
half-grid potential differences, while native differentiates within the
containing cell. This section tests physical refinement instead of treating
either discrete model as ground truth.

The reproducible runners are
`crates/simrep-solve/examples/nonlinear_dielectric_refinement.{rs,lua}`.
The Rust runner generates cell-centre sphere labels and electrode masks;
its `verify` action checks **every** 1x label and electrode flag against
the shipped arrays. The Lua runner loads the constitutive, `field_vc`,
and material-update functions directly from the original
`sphere_nonlinear_3dp.lua`, rescaling only its sphere coordinates for
each grid. It omits identically-one vacuum writes for convergence
measurements; its elapsed times are therefore **not** substitutes for the
original full-update benchmark.

The physical box stays 100 x 50 x 50 mm with y/z mirrors, a 10 mm sphere
at (50,0,0), and 10 MV / 0 V x faces. Spacing is 2, 1, 0.5, and 0.25 mm:
51 x 26 x 26 through 401 x 201 x 201 potential nodes. Thus 0.5 mm is
the shipped **1x** grid and 0.25 mm is **2x**. The one-sided sampling
offset shrinks with spacing: one grid unit, not a fixed 0.5 mm. Holding
that offset fixed would extrapolate a spatially nonlocal constitutive
model rather than the requested local continuum law.

Native CPU references use nonlinear residual and field-change tolerances
`1e-12`; their final residuals are 2.96e-13, 2.66e-13, 4.66e-13, and
2.70e-13. SIMION uses 40 updates / `1e-7` V at the two coarse grids.
At 1x, a fresh 25-update run starts from the previous strict SIMION
snapshot. At 2x, it starts from the converged native potential but rebuilds
epsilon with **SIMION's own field routine**, then performs 25 SIMION
updates at `1e-4` V. This seed saves work; it does not replace SIMION's
nonlinear fixed point with the native one.

The last five SIMION updates change the full potential by 8.01e-8,
2.72e-7, 4.02e-7, and 4.09e-4 V respectively. The 2x final maximum
epsilon change is 6.56e-10. Its convergence uncertainty is negligible
beside the hundreds-to-thousands of volts of physical grid error below.
The 2x continuation took 221.554 s, excluding I/O. Two earlier 1x
`1e-7` attempts, from zero and native seeds, were stopped for cost;
neither supplies a scientific result or a benchmark time.

The native 1x/2x maximum resident sets were 979,384 / 6,090,312 KiB.
Simple 8x node-count scaling already puts native 4x near 46 GiB, beyond
the roughly 20 GiB available; 8x is further out of reach. SIMION 4x was
not attempted: even linear-in-node-count scaling of the 221.554 s 2x
continuation exceeds the 1,200 s per-run budget, before the worse
relaxation conditioning. No 4x/8x result is implied.

#### Continuum estimates and observed orders

Probes A/B/C/D are potentials at fixed physical positions
(25,0,0), (45,0,0), (65,5,5), and (75,20,0) mm. Additional observables
are the centred x field at the sphere centre, the total induced dipole
`p_x = integral epsilon_0*(epsilon_r-1)*E_x dvolume`, and excess charge
on the high-voltage plate relative to the empty box. Integrals include
the four mirrored quadrants. Epsilon is sampled at the projected point;
the field multiplying it in the polarization integral is at the actual
material-cell centre. This distinction matters near the interface.

Converged results satisfy `p_x ~= 100 mm * excess_charge` to at worst
1.21e-12 C mm, a useful conservation check independent of the centre
field. Neither quantity is obtained by fitting the potential probes.

The observed order is
`log2((O(2h)-O(h))/(O(h)-O(h/2)))`. Both methods show the same trends,
but the order is **not stable enough to claim clean second-order
Richardson convergence**:

| Observable | Native orders, coarse then fine triplet | SIMION orders, coarse then fine triplet |
| --- | --- | --- |
| Centre field | 0.905, 0.791 | 0.899, 0.792 |
| A | 1.672, 1.254 | 1.669, 1.255 |
| B | 0.810, 0.869 | 0.799, 0.871 |
| C | 1.586, 1.269 | 1.580, 1.271 |
| D | 1.628, 1.183 | 1.626, 1.182 |
| Excess charge / dipole | 1.630, 1.196 | 1.628, 1.195 |

A first-order leading error is consistent with the shrinking one-cell
projection shell and voxelized curved interface. Use the mean of the two
finest-pair first-order extrapolants, `2*O(h/2)-O(h)`, as the common
continuum **estimate**. For sensitivity, also compute three-grid
Richardson extrapolants using each method's observed fine-triplet order.
The range below spans all four extrapolants; it is a model-sensitivity
range, **not a certified error enclosure**:

| Observable | Common first-order estimate | Extrapolation sensitivity range |
| --- | --- | --- |
| Centre E, V/mm | 42065.717 | 41910.486 .. 42065.781 |
| A, V | 7414239.819 | 7413961.797 .. 7414240.737 |
| B, V | 5210285.011 | 5209839.256 .. 5210286.845 |
| C, V | 3687447.542 | 3687445.795 .. 3688061.232 |
| D, V | 2538210.174 | 2538209.655 .. 2538313.172 |
| Dipole, C mm | 6.46757843e-6 | 6.46749580e-6 .. 6.48601808e-6 |
| Excess charge, C | 6.46757966e-8 | 6.46749827e-8 .. 6.48601808e-8 |

The two methods' first-order extrapolated limits differ by only
0.128 V/mm at the centre, 1.836 / 3.666 / 3.495 / 1.039 V at A/B/C/D,
and 1.63e-12 C in excess charge. These differences are much smaller
than the extrapolation sensitivity. The data support a **common
continuum limit**, not a persistent 424.8 V physical-model disagreement.
They do not establish a high-precision continuum field everywhere.

Absolute estimated physical errors at every grid follow. Charge errors
are in nC; dipole errors are numerically 0.1 times that column in
micro-C mm for the converged rows, within the conservation uncertainty.

| Method / grid | Centre E error, V/mm | A error, V | B error, V | C error, V | D error, V | Excess charge error, nC |
| --- | --- | --- | --- | --- | --- | --- |
| Native, 0.25x / 2 mm | 2926.379 | 11952.707 | 14890.298 | 24490.759 | 5424.129 | 9.26515 |
| SIMION, 0.25x / 2 mm | 2909.078 | 11872.685 | 14736.540 | 24299.917 | 5398.918 | 9.22151 |
| Native, 0.5x / 1 mm | 1566.140 | 4374.273 | 8104.837 | 9276.665 | 2051.925 | 3.49373 |
| SIMION, 0.5x / 1 mm | 1559.573 | 4348.590 | 8049.470 | 9220.023 | 2043.825 | 3.47980 |
| Native, 1x / 0.5 mm | 839.687 | 1995.742 | 4235.009 | 4207.658 | 961.085 | 1.62876 |
| SIMION, 1x / 0.5 mm | 835.899 | 1982.309 | 4205.599 | 4176.856 | 957.093 | 1.62189 |
| Native, 2x / 0.25 mm | 419.812 | 998.330 | 2116.588 | 2104.703 | 480.802 | 0.81479 |
| SIMION, 2x / 0.25 mm | 417.981 | 990.695 | 2103.716 | 2087.554 | 478.287 | 0.81054 |

The maximum nodal native/SIMION differences shrink as
1081.338 / 624.911 / 424.769 / 226.084 V. SIMION's half-grid evaluation
is systematically slightly nearer the extrapolated continuum for these
observables at a fixed grid: about 0.4--0.8% less error at 1x. This is a
small effect beside the common physical discretization error, not a
different convergence order. I considered a configurable constitutive
field evaluator, but retained the production API and default: this
single curved-interface case does not justify a general default change,
and reducing the common projection/interface bias matters far more.
No dielectric solver or analytic-case behavior was changed.

#### Speed at matched estimated physical accuracy

The acceptance criterion is componentwise: native must have no greater
estimated physical error than **SIMION's shipped 1x default** at all four
potential probes, centre field, induced dipole, and plate charge. The
decision remains the same throughout the extrapolation sensitivity ranges
above. This is a matched-observable physical-accuracy verdict, not a
certified continuum infinity-norm bound over the whole field.

Screening covered native 1x tolerances `1e-3`, `1e-4`, `3e-5`, `2e-5`,
`1.5e-5`, `1e-5`, and `1e-6`, plus the tight reference. None passed all
observables: at `1e-4`, probe A is worse than default SIMION; at `3e-5`,
B/C/D remain worse; tighter candidates retain the centre/B/C/D physical
grid bias. Native 2x at `1e-4` also fails, because its excess charge is
7.18447e-8 C. Native **2x at `1e-5` passes every observable**. This is the
cheapest qualified tested grid/tolerance, not a claim of an exhaustive
continuous-parameter optimum.

| Configuration | Centre E error, V/mm | A / B / C / D error, V | Dipole error, C mm | Excess charge error, nC |
| --- | --- | --- | --- | --- |
| SIMION 1x default, 20 updates / 0.1 V | 834.283 | 2197.204 / 3914.693 / 3958.935 / 804.187 | 1.62070e-7 | 2.42048 |
| Native 2x CPU, residual/field tolerance 1e-5 | 415.704 | 1007.374 / 2089.326 / 2115.542 / 481.172 | 8.10736e-8 | 0.84114 |
| Native 2x Vulkan, residual/field tolerance 1e-5 | 413.942 | 1025.477 / 2059.230 / 2097.689 / 468.964 | 8.09659e-8 | 1.02216 |

Three fresh Linux CPU solves, with four Rayon workers, took
43.636006 / 44.899943 / 41.628331 s, median **43.636006 s**. Seven outer
iterations reached nonlinear residual 2.07620e-6. This is the cheapest
qualified measured native configuration in this audit.

Three fresh Windows Vulkan RTX 3070 runs at this accuracy took
48.866745 / 42.851494 / 48.368654 s, median **48.368654 s**. Each process
was bounded by 180 s; only these three GPU measurements were made.
All include constitutive preparation, hierarchy rebuilds, inner solves,
and transfers, but exclude I/O and startup. Seven outer iterations reached
nonlinear residual 3.31917e-6. Their identical reported observables are
from saved independent solves, not an accuracy inferred from timing.

Against the retained three-run original-SIMION default median
**87.494 s** (96.138 / 87.494 / 61.164 s), native is **2.01x faster on
CPU and 1.81x on GPU at matched estimated physical accuracy**, with
roughly half the tested physical-observable errors. The original
5.76x / 7.17x figures do not survive this stricter comparison. SIMION's
timings were retained from the same-day
full original-update benchmark, not remeasured with the optimized audit
continuation. GPU utilization was zero immediately before the triplet,
but compilation/release tests and an unrelated Windows computation
overlapped parts of it. These are shared-machine medians, not isolated
throughput guarantees. The CPU triplet ran after that compilation/testing
work, so its small advantage over this GPU triplet is not a new isolated
CPU/GPU crossover calibration.

Example reproduction, using an unused writable output directory:

```sh
root=/mnt/c/Users/lsu10/simrep-oracle/dielectric
out=/mnt/c/Users/lsu10/simrep-oracle/physical-h025
cargo run -p simrep-solve --release --example nonlinear_dielectric_refinement -- \
  "$root" "$out" 0.25 generate
RAYON_NUM_THREADS=4 cargo run -p simrep-solve --release \
  --example nonlinear_dielectric_refinement -- "$root" "$out" 0.25 cpu 1e-12 1
# From the output directory, run the Lua runner in foreground headless SIMION:
# lua <refinement.lua> <original sphere_nonlinear_3dp.lua> 25 1e-4 native.pa
# Then use the Rust runner's observe action on simion.pa.
```

Windows invocations used a `/mnt/c` cwd, closed stdin, a timeout, and
foreground processes with piped output. Only the identified task-owned
SIMION trees from the two discarded strict-objective attempts were killed.
The final observer source builds and both strict clippy commands and
formatting pass. `cargo test --workspace --release` passes all 249 regular
tests, including the layered analytic nonlinear case. The language-server
daemon timed out; compiler and clippy diagnostics supply the available
source checks. Final `winproc.ps1 cleanup` output was:

```text
orphan conhost: 0
busy conhost (>50% of a core): 2
stopped orphan conhost: 0
```

The following explicit process query found no `simion.exe`,
`nonlinear_dielectric_refinement.exe`, `cargo.exe`, or `rustc.exe`.
The two busy consoles are the pre-existing WSL-parented processes;
they and the unrelated Windows computation were left untouched.

### Class-aware auto thresholds (October 25, 2026)

`dielectric::prefer_gpu(grid, nonlinear)` selects a hardware GPU only for 3D
grids (`dims[2] > 1`) whose node count reaches the class threshold:
**1,048,576 nodes for linear solves** and **131,072 nodes for nonlinear
solves**. The nonlinear inner linear solver runs many times per outer
iteration, so the GPU's fixed setup/transfer cost amortizes at a smaller
grid than for a single linear solve.

The crossover calibration below was reported by the 2026-10-24
implementation run on this workstation (Ryzen 9 5950X, 16 Rayon workers;
Windows RTX 3070 Vulkan). Linear rows use `dielectric_oracle` at relative
tolerance `1e-10`; nonlinear rows use `nonlinear_dielectric_refinement` on
the generated sphere box at residual/field tolerance `1e-5`. All values are
three-run median compute seconds.

| Nodes | Linear CPU s | Linear Vulkan s | Nonlinear CPU s | Nonlinear Vulkan s |
| ---: | ---: | ---: | ---: | ---: |
| 34,476 | 0.036586 | 0.109938 | 0.104235 | 0.202247 |
| 262,701 | 0.106402 | 0.128869 | 0.466726 | 0.430785 |
| 2,050,401 | 0.512117 | 0.191680 | 2.809180 | 2.537782 |
| 3,984,876 | 0.764693 | 0.219370 | 5.385029 | 4.347306 |
| 16,200,801 | 4.237719 | 0.967404 | 23.619048 | 16.076825 |

Fresh re-check on 2026-10-25 at the two decision points (same binaries,
same machine, shared load ~4-9):

| Case | Nodes | CPU median s | Vulkan median s | Auto choice |
| --- | ---: | ---: | ---: | --- |
| `dielectric_oracle` sphere_3dp, linear `1e-10` | 2,050,401 | 0.599625 | 0.150889 | GPU (>= 1,048,576) |
| `nonlinear_dielectric_refinement` h=1 sphere box, `1e-5` | 262,701 | 0.521977 | 0.551291 | GPU (>= 131,072) |

The nonlinear re-check straddles parity (CPU 0.95x of GPU), consistent with
the reported bracket; the linear re-check keeps a 4x GPU advantage. The
nonlinear grid sits just above its threshold, so the GPU pick is a near-tie
rather than a large win. Exact reproduction commands:

```sh
# CPU, Linux
RAYON_NUM_THREADS=16 target/release/examples/dielectric_oracle \
  /mnt/c/Users/lsu10/simrep-oracle/dielectric cpu 1e-10
RAYON_NUM_THREADS=16 target/release/examples/nonlinear_dielectric_refinement \
  /mnt/c/Users/lsu10/simrep-oracle/dielectric target/nl-h1 1.0 cpu 1e-5 3
# GPU, Windows Vulkan (foreground, /mnt/c cwd, closed stdin, timeout)
timeout 900 /mnt/c/Users/lsu10/simrep-target/release/examples/dielectric_oracle.exe \
  'C:\Users\lsu10\simrep-oracle\dielectric' vulkan 1e-10 </dev/null
timeout 600 /mnt/c/Users/lsu10/simrep-target/release/examples/nonlinear_dielectric_refinement.exe \
  'C:\Users\lsu10\simrep-oracle\dielectric' 'C:\Users\lsu10\simrep-oracle\qa-nl-h1' \
  1.0 vulkan 1e-5 3 qa-gpu.pa </dev/null
```
