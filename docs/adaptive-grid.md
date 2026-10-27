# Opt-in adaptive electrostatic grids

`refine-geometry --refine-region xmin,ymin,zmin,xmax,ymax,zmax:factor`
adds a Cartesian fine patch. Bounds must coincide with base nodes; the factor
is 2 or 4. Repeat the option for separated patches. Patches cannot overlap or
touch. This is static, CPU, f64 electrostatic refinement, not error-driven AMR.
Without this option the existing uniform solver and output path are retained.
The regression suite compares complete uniform PA families byte for byte.

```sh
simrep refine-geometry lens.gem \
  --electrode 1=1:0 --electrode 2=2:110 --electrode 3=3:0 \
  --refine-region 24,-20,-20,34,20,20:2 \
  --refine-region 52,-20,-20,62,20,20:2 \
  --out lens --device cpu
simrep fly --pa lens.adaptive.json --particles ions.ion --device cpu
```

## Discrete space and conservation contract

The domain is partitioned into base cells outside patches and fine cells inside
patches. Coarse nodes covered by patch interiors are removed from the unknown
vector. Interior fine vacuum nodes are independent unknowns. On an internal
patch face, fine nodes are trilinear interpolants of the coarse face trace;
in 2D this is bilinear interpolation. At a trace edge crossing metal, the
coarse vacuum linear polynomial is first continued through the metal node
using the existing cut distance and prescribed surface voltage. Interpolating
the stored metal voltage instead would move the surface to that node and
produce first-order interface errors. This vacuum continuation is an affine
function of coarse unknowns and therefore remains part of `P` and its lift.
Faces at the physical domain boundary
remain fine unknowns and retain the base homogeneous Neumann condition.
Electrode values supply an affine lift; the correction prolongation is `P`.

Each cell contributes the Cartesian volume-weighted nodal edge energy

```text
1/2 * [cell_volume / (2^(dimension-1) * h_axis^2)] * (u_i-u_j)^2.
```

Summing adjacent cell contributions recovers the existing symmetric
`-W L` operator multiplied by physical cell volume, including half weights
on mirror faces. Cells covered by a patch contribute nothing at base spacing:
there is no double counting of coarse and fine energy. On an electrode cut,
the existing cut-row distance fraction `theta` replaces the free/free edge
with the free-to-Dirichlet energy `k/theta * (u_i-V)^2/2`.
If both endpoints are free but an electrode lies between them, both
free-to-surface energies are retained. Quadratic snapping is inherited from
`CutCells`; physical electrode masks, not solver snapping, are exported.

Substitution of the hanging-node constraints before assembly gives exactly
`A = P^T A_f P`, with load `P^T (b_f - A_f g)`. The assembled CSR matrix is
symmetric to floating-point accumulation error. It is positive definite when
the constrained vacuum space is anchored by Dirichlet electrodes. Cut
conductances are positive; no nonsymmetric interpolation equations are added.

**Conservation is variational:** the sum of the coarse and fine flux work
vanishes against every retained coarse trace basis function. The constant
trace test gives total interface flux balance. Individual fine face fluxes
need not equal a rediscretized coarse face flux. This is not a claim of
facewise finite-volume conservation or pointwise derivative continuity.

## Linear solver

Preconditioned CG solves the composite matrix. The preconditioner is the sum
of composite diagonal scaling and an additive base-grid correction
`T B_MG T^T`. `T` evaluates coarse correction interpolants at composite
independent nodes; `B_MG` is one existing symmetric Galerkin V-cycle with
equal pre/post smoothing. The correction has homogeneous conductor data.
Weighted residuals are converted back to physical sources at the existing
finest-grid smoother boundary. Base cell volume normalizes the coarse
correction to the composite energy units.

The diagonal term controls fine modes absent from the base space, while the
multigrid term removes long-wavelength error without a new FAC hierarchy.
CG was chosen because the constrained energy supplies an SPD operator and
the additive correction reuses the established multigrid transfers.
Every iteration checks the true recomputed CSR residual, not just the
recursive CG residual. `--max-cycles` caps CG iterations for adaptive runs.
The reported relative residual is the infinity norm of the assembled weak
residual divided by its initial norm; it is not the uniform solver's
unweighted physical Laplace residual.

## Files, ghosts, and flight

The base `stem.pa0`, `stem.paN`, `stem.pa#`, and `stem.cuts.json` retain their
ordinary uniform format. Base values at covered nodes are sampled from the
composite fine solution. Each patch writes `stem.patchK.pa0` and all matching
`stem.patchK.paN` files, preserving sparse electrode numbering.
`stem.adaptive.json` has `schema_version: 1`, relative base/patch filenames,
physical origins, spacings, patch core boxes, levels, factors, ghost width,
reference voltage, and initial electrode voltages.

Patch arrays extend two fine nodes beyond each internal core boundary,
clipped at the physical domain. Core values come from the composite solve;
ghost values come from the composite base interpolant outside the core.
The field selects the finest core containing a sample, otherwise the base,
and reconstructs using the ghosted array. Ghosts are not extra solved DOFs.
The base instance owns domain exits, electrode events and exact-surface
collisions. Patch boundaries do not cause splats. Placement, axes, scale,
workbench bounds and detector planes still apply to the enclosing instance.
As with a uniform PA, `--origin` supplies placement; manifest origins define
patch offsets relative to the base origin.

Both ordinary CPU flight and the FLY2 field provider use `AdaptiveField`.
Static `--voltage` adjustments scale every matching patch basis as well as
the base. The manifest's initial voltage table is necessary because a patch
may contain no conductor node for an electrode whose field it carries.
Such cropped patches cannot use ordinary standalone `read_fast_adjust`;
the manifest loader handles their normalization explicitly.

## Verification and studies

The reproducible, external-fixture-free measurement driver is:

```sh
RAYON_NUM_THREADS=4 cargo run --release -p simrep-cli --example adaptive_study
cargo test -p simrep-solve --lib adaptive::tests -- --nocapture
cargo test -p simrep-cli --test adaptive_cli
```

The tests cover assembled-matrix symmetry, full small-matrix Cholesky,
manufactured linear weak flux balance, a Poisson convergence ladder across
interfaces, curved cuts crossing interfaces, affine ghost reconstruction,
base collision ownership, sparse fast-adjust bases without local conductor
nodes, ordinary flight, FLY2 source field evaluation, and uniform-family
byte identity.

Measurements below use compute time only: uniform multigrid solve or adaptive
CG/preconditioning. Geometry preparation, matrix/hierarchy construction and
file IO are excluded. Scalar bytes count one f64 potential per stored node;
CSR bytes count the composite operator/load, not total process RSS. The
temporary map assembly, transfer lists, masks, cut rows, multigrid hierarchy,
CG vectors and spline reconstruction consume additional memory. A smaller
node count therefore does not establish smaller total memory or faster solve.
Node counts below include the base and patch-core potential vectors, including
covered base nodes, but exclude the extra exported ghost nodes. They are not
counts of independent unknowns.

Measured on October 27, 2026, Linux x86-64, AMD Ryzen 9 5950X, with four Rayon
workers and release optimization. These are single-run compute measurements,
not statistically controlled performance claims.

The assembled linear-interface tests have zero measured matrix asymmetry for
both factors, smallest Cholesky pivots 1.409 and 1.437, and maximum manufactured
weak-flux imbalance `2.220e-16`. The smooth Poisson MMS maximum errors at base
sizes 17, 33 and 65 are `4.783487782e-3`, `1.327444633e-3`, and
`3.645970130e-4`, giving successive orders 1.849 and 1.864. For a circle crossing
the interface, worst errors over four subcell translations are
`3.746508789e-3`, `1.018188477e-3`, and `3.675879053e-4`; the two-refinement
order is 1.675 and every grid satisfies the tested `2 h^2` envelope. That
curved test also measures zero matrix asymmetry. Single-level curved rates
vary with quadratic snapping; no per-level second-order rate is asserted.

The spherical test uses the mirrored octant of concentric shells with radii
0.31 and 0.83 mm at 1 and 0 V. Its exact vacuum potential is
`(1/r - 1/R2)/(1/R1 - 1/R2)`, and the analytic field is its negative gradient.
The energy-per-charge check is `U R1 R2/(R2^2-R1^2)`, recovered from the
numerical radial field as `E_r r^2/(R1+R2)`. This is an electrostatic
passband formula check, not a simulated analyzer transmission curve.

| Shell solve | Nodes | Free DOFs | Potential bytes | CSR bytes | Compute ms | Max potential error V | Max nodal field error V/mm | E/q eV/e |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| Uniform 17^3 | 4,913 | 1,322 | 39,304 | 0 | 4.617 | 1.276923077e-2 | 5.259115263e-1 | 0.4406291385 |
| Uniform 33^3 | 35,937 | 9,988 | 287,496 | 0 | 37.411 | 4.792431593e-3 | 5.117640249e-1 | 0.4354351675 |
| Uniform 65^3 | 274,625 | 77,065 | 2,197,000 | 0 | 250.742 | 7.742340517e-4 | 1.989182956e-1 | 0.4343671490 |
| Base 17^3 + factor-2 patch | 29,302 | 9,964 | 234,416 | 1,299,336 | 23.111 | 4.792431593e-3 | 5.118709177e-1 | 0.4354676641 |

The shell patch is `[0,0,0]..[0.875,0.875,0.875]`, covering the entire octant
gap. Its potential accuracy matches uniform 33^3 in this test, with 18.46%
fewer stored solver nodes. The exact passband is `0.4340418353576 eV/e`;
the adaptive field check differs by 0.329%. This field error is not an energy
acceptance tolerance for trajectories. CSR storage alone exceeds the uniform
fine potential vector; the observed compute reduction does not imply a
memory reduction.

The einzel study uses the electrode axial intervals and radius from
`examples/einzel_convergence.rs`: grounded 0..28, 110 V at 30..56,
grounded 58..90, and inner radius 18 mm. Since cylindrical AMR is not
implemented, these surfaces are represented in 3D Cartesian coordinates on
a square transverse domain [-20,20]^2. The square exterior in the gaps is
not the original cylindrical r=19 mirror boundary, so its numbers must not
be presented as cylindrical-example or SIMION parity. Gap patches cover
x=24..34 and x=52..62 with the entire transverse domain. Errors compare
seven fixed potential probes with a finer Cartesian reference.

| Cartesian einzel solve | Nodes | Free DOFs | Potential bytes | CSR bytes | Compute ms | Max probe error V |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Uniform h=2 mm | 20,286 | 11,454 | 162,288 | 0 | 31.240 | 0.3897018322 |
| Uniform h=1 mm | 152,971 | 92,807 | 1,223,768 | 0 | 159.977 | 0.5460012793 |
| h=2 mm + h=1 mm gap patches | 57,268 | 28,904 | 458,144 | 4,052,520 | 165.248 | 0.6136473805 |
| Reference h=0.5 mm | 1,187,541 | 747,941 | 9,500,328 | 0 | 1,334.483 | reference |

The adaptive solve uses 62.56% fewer solver nodes than uniform h=1 mm, but is
slightly slower and has a larger worst probe error. Uniform refinement is
also nonmonotonic at these probes because surface snapping and the sampling
locations change relative to the grid. The dominant adaptive error is at
`[31,16,0]`, near the central electrode edge. At gap centers `[29,0,0]` and
`[57,0,0]`, adaptive errors are 0.01679735 and 0.01736337 V. This study does
not establish a general adaptive accuracy or speed advantage.

| Probe mm | Adaptive V | Fine-reference V |
| --- | ---: | ---: |
| [29,0,0] | 52.93014632 | 52.94694367 |
| [29,10,0] | 53.74320733 | 53.75365962 |
| [29,16,0] | 54.68595945 | 54.68573295 |
| [31,16,0] | 83.75680305 | 83.14315567 |
| [43,0,0] | 84.09357740 | 84.29749552 |
| [57,0,0] | 52.90566789 | 52.92303126 |
| [57,16,0] | 54.68230015 | 54.68212036 |

## Limitations

- Cartesian, electrostatic, CPU only. Cylindrical AMR, GPU solves/flight,
  dielectric coefficients and magnetic potential refinement are not supported.
  Unsupported geometry/devices are rejected; automatic flight stays on CPU.
- Only one refinement generation and disjoint non-touching boxes are assembled.
  Factor 4 is a direct refinement, not a recursively nested hierarchy.
- A vacuum opening absent from every coarse trace corner cannot gain an
  independent trace mode by hanging-node elimination. Resolve such openings
  on the base grid; vacuum continuation only repairs represented cut edges.
- Time-dependent voltage expressions are rejected for manifest fields.
- Fine metal surfaces do not change base-owned staircase collision geometry.
  Keep the generated exact-source `.cuts.json` for fine surface collisions.
- Ghosted spline reconstruction is not a globally coupled spline. Potential
  and electric-field matching are exact for affine manufactured fields, but
  general patch interfaces are not promised to be globally C1. The solver's
  weak conservation contract is a nodal contract, not a spline-flux claim.
- Matrix assembly uses ordered sparse maps and stores explicit CSR. It is a
  correctness-first CPU implementation, not a memory-optimized production AMR
  solver; no universal speedup or memory saving is claimed.
