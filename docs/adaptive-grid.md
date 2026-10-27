# Opt-in adaptive electrostatic grids

`refine-geometry --refine-region xmin,ymin,zmin,xmax,ymax,zmax:factor`
adds a Cartesian fine patch. Bounds must coincide with base nodes; the factor
is 2 or 4. Repeat the option for separated patches. Patches cannot overlap or
touch. This is static, CPU, f64 electrostatic refinement, not error-driven AMR.
Without a refinement option the existing uniform solver and output path are retained.
The regression suite compares complete uniform PA families byte for byte.

```sh
simrep refine-geometry lens.gem \
  --electrode 1=1:0 --electrode 2=2:110 --electrode 3=3:0 \
  --refine-region 24,-20,-20,34,20,20:2 \
  --refine-region 52,-20,-20,62,20,20:2 \
  --out lens --device cpu
simrep fly --pa lens.adaptive.json --particles ions.ion --device cpu
```

## Automatic regions

`--refine auto` (equivalently `--refine auto:2`) opts into surface and gap
marking. `--refine auto:4` raises the maximum factor to 4. Automatic and manual
regions are mutually exclusive. Cartesian CPU geometry is required.

The selector subdivides the base domain dyadically, rejecting a box only when
every electrode's conservative signed distance at its centre exceeds the
box's circumscribed radius plus one coarsest-axis base spacing. Leaves are at
most four base cells across. Surface leaves gain a one-base-cell buffer.
Gap pairs below 2.5 base cells, or with an unresolved bracket, also mark a box
around both witness points with a two-base-cell buffer. Bounds round outwards
to base nodes and clamp to the domain. All clearance lengths scale with h;
there is no absolute millimetre threshold. The surface band includes sharp
edges and wedges without assuming opposing surface normals or modifying the
electrode geometry. It is not a separate CAD wedge classifier.

Sorted touching/overlapping boxes merge to a fixed point, including contacts
created by a previous merge. This also removes nesting. A factor-2 box has a
2:1 interface to its unrefined surroundings. A partial factor-4 box would
need a surrounding factor-2 halo, which the current non-touching, non-nested
prototype cannot assemble. Such boxes explicitly cap at factor 2 and increment
`balance_limited_boxes`; only a whole-domain box may use factor 4. Manual
factor-4 regions keep their existing contract.

The log and additive schema-1 `auto_refinement` JSON object report requested
factor, candidate/merged/balance-limited counts, skipped gap witnesses,
selected bounds/factors/dimensions, base/patch/stored node counts, and gap
brackets, witness locations and expected cells across. Stored counts include
covered base nodes and patch cores, not exported ghost nodes. A witness-local
prediction is not a certificate over an entire electrode pair:
`minimum_cells_across` uses base h unless refinement covers the whole domain.
An inadequate requested factor reports `gap_unresolvable`; an uncertified
GEM measurement remains `unresolved_bracket`, even after refinement.
`--gap-check error` requires the conservative minimum and a resolved bracket.
With `--gap-check off`, auto still measures gaps for selection but suppresses
gap-policy warnings. An unresolvable pair is reported, never silently called
resolved or repaired by changing geometry.

This is conservative marking under a restricted patch contract, not a promise
of fewer nodes: a connected curved surface band can merge into one box covering
the entire domain. The measurements below deliberately retain that outcome.

The design references are Iteration-CFD's
[`automesher/octree.rs`](https://github.com/using76/Iteration-CFD/blob/main/rust/src/automesher/octree.rs)
(dyadic marking and monotone 2:1 closure),
[`castellate.rs`](https://github.com/using76/Iteration-CFD/blob/main/rust/src/automesher/castellate.rs)
(partition/keep-set accounting),
[`features.rs`](https://github.com/using76/Iteration-CFD/blob/main/rust/src/automesher/features.rs)
(sharp edges cannot be inferred from face proximity alone),
[`quality.rs`](https://github.com/using76/Iteration-CFD/blob/main/rust/src/automesher/quality.rs)
and [`driver.rs`](https://github.com/using76/Iteration-CFD/blob/main/rust/src/automesher/driver.rs)
(separate measurement, gates and stage reports), and
[`adapt.rs`](https://github.com/using76/Iteration-CFD/blob/main/rust/src/adapt.rs),
[`adapt/rebuild.rs`](https://github.com/using76/Iteration-CFD/blob/main/rust/src/adapt/rebuild.rs)
and [`adapt/transfer.rs`](https://github.com/using76/Iteration-CFD/blob/main/rust/src/adapt/transfer.rs)
(ordered rebuild and an explicit transfer/conservation contract).
The upstream file is `rust/src/adapt.rs`, not `rust/src/adapt/adapt.rs`.
The [site-mesh playbook](https://github.com/using76/Iteration-CFD/blob/main/docs/08-site-mesh-playbook.md)
supplies the g/2.5 rule, unresolvable-gap reporting, wedge caveat and
size-relative post-processing policy. SimRep does not import CFD topology
repair, face-flux transfer, feature snapping or solid merging. The adapted
selection policy's Prosperity 3.0.0 notice is in `licenses/THIRD_PARTY.md`.

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
The tables were re-measured with the instrumented driver in this change;
the earlier einzel timings of 165.248 ms adaptive and 159.977 ms uniform
are historical, not the current run's timings.

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
| Uniform 17^3 | 4,913 | 1,322 | 39,304 | 0 | 2.469 | 1.276923077e-2 | 5.259115263e-1 | 0.4406291385 |
| Uniform 33^3 | 35,937 | 9,988 | 287,496 | 0 | 10.684 | 4.792431593e-3 | 5.117640249e-1 | 0.4354351675 |
| Uniform 65^3 | 274,625 | 77,065 | 2,197,000 | 0 | 62.867 | 7.742340517e-4 | 1.989182956e-1 | 0.4343671490 |
| Base 17^3 + manual factor-2 patch | 29,302 | 9,964 | 234,416 | 1,299,336 | 11.151 | 4.792431593e-3 | 5.118709177e-1 | 0.4354676641 |
| Base 17^3 + auto factor-2 patch | 40,850 | 9,988 | 326,800 | 1,302,600 | 10.746 | 4.792431593e-3 | 5.117640261e-1 | 0.4354351675 |

The shell patch is `[0,0,0]..[0.875,0.875,0.875]`, covering the entire octant
gap. Its potential accuracy matches uniform 33^3 in this test, with 18.46%
fewer stored solver nodes. The exact passband is `0.4340418353576 eV/e`;
the adaptive field check differs by 0.329%. This field error is not an energy
acceptance tolerance for trajectories. CSR storage alone exceeds the uniform
fine potential vector. In this re-measurement neither adaptive variant
establishes a compute reduction over uniform 33^3.

Automatic surface marking merges 45 candidate boxes into `[0,0,0]..[1,1,1]`.
At matched potential accuracy, it therefore stores the uniform 33^3 patch
plus the 4,913-node base. Its 26 CG/MG iterations compare with 9 uniform
multigrid cycles. The analytic shell result is a spherical-analyzer field
check; no trajectory acceptance or energy-resolution accuracy is inferred.

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
| Uniform h=2 mm | 20,286 | 11,454 | 162,288 | 0 | 14.700 | 0.3897018322 |
| Uniform h=1 mm | 152,971 | 92,807 | 1,223,768 | 0 | 62.818 | 0.5460012793 |
| h=2 mm + manual h=1 mm gap patches | 57,268 | 28,904 | 458,144 | 4,052,520 | 76.105 | 0.6136473805 |
| Wider manual h=1 mm patches | 84,164 | 42,992 | 673,312 | 5,941,352 | 88.911 | 0.5509113444 |
| h=2 mm + auto h=1 mm patch | 173,257 | 92,807 | 1,386,056 | 12,341,728 | 141.097 | 0.5460012821 |
| Reference h=0.5 mm | 1,187,541 | 747,941 | 9,500,328 | 0 | 330.133 | reference |

The manual gap-patch solve uses 62.56% fewer solver nodes than uniform h=1 mm, but is
slower and has a larger worst probe error. Uniform refinement is
also nonmonotonic at these probes because surface snapping and the sampling
locations change relative to the grid. The dominant adaptive error is at
`[31,16,0]`, near the central electrode edge. At gap centers `[29,0,0]` and
`[57,0,0]`, adaptive errors are 0.01679735 and 0.01736337 V. This study does
not establish a general adaptive accuracy or speed advantage.

Automatic marking merges 740 surface/gap candidates into the whole Cartesian
domain `[0,-20,-20]..[90,20,20]`. It matches the uniform h=1 mm probe error
within 3e-9 V, but stores the 152,971-node fine grid plus the 20,286-node base
and takes more compute time. Both 2 mm electrode gaps still have only two
fine cells across; auto:2 explicitly reports `gap_unresolvable`. A factor-4
whole-domain selection would pass the 2.5-cell minimum, but is not part of this
matched-h=1 mm comparison.

| Probe mm | Manual gap-patch V | Fine-reference V |
| --- | ---: | ---: |
| [29,0,0] | 52.93014632 | 52.94694367 |
| [29,10,0] | 53.74320733 | 53.75365962 |
| [29,16,0] | 54.68595945 | 54.68573295 |
| [31,16,0] | 83.75680305 | 83.14315567 |
| [43,0,0] | 84.09357740 | 84.29749552 |
| [57,0,0] | 52.90566789 | 52.92303126 |
| [57,16,0] | 54.68230015 | 54.68212036 |

### Why the prototype costs more and can be less accurate

The original 57,268-versus-152,971 stored-node comparison hides work and
discretization differences. Assembly below includes affine DOF/trace-map
construction and CSR construction, not an isolated allocator profile.
The instrumented single-run phases are:

| Solve | CG / MG applications | CSR products | Assembly ms | Transfer setup ms | MG hierarchy ms | Preconditioning ms | CSR multiply ms |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| Manual sphere | 26 / 26 | 52 | 22.722 | 0.944 | 0.993 | 7.688 | 2.345 |
| Auto sphere | 26 / 26 | 52 | 15.352 | 0.612 | 0.810 | 7.258 | 2.113 |
| Manual einzel | 35 / 35 | 70 | 44.588 | 1.656 | 4.085 | 56.528 | 14.729 |
| Wider einzel | 34 / 34 | 68 | 66.170 | 2.526 | 3.821 | 58.212 | 23.502 |
| Auto einzel | 28 / 28 | 56 | 134.941 | 5.491 | 3.076 | 78.287 | 48.557 |

Uniform einzel uses 9 multigrid cycles at either h=2 or h=1 mm. Every
composite iteration gathers onto the complete base grid, applies one MG
V-cycle, scatters back, and performs two CSR products, including the true
residual check. Fewer stored nodes do not imply fewer smoother/transfer
operations. The manual einzel preconditioner alone takes 56.528 ms versus
62.818 ms for the entire uniform h=1 mm compute phase. The explicit ordered
map assembly takes another 44.588 ms of preparation: it worsens total cost
but cannot explain a compute-only regression because it is excluded from
`compute_ms`. Reconstruction adds 0.523 ms to the manual solve. Per-basis
`adaptive_profiles` JSON exposes these phases and operation counts.

The safe local change caches the infinity norm of each recomputed residual
instead of rescanning it several times. It does not change the residual,
stopping rule, assembled operator or true-residual verification. Timing
differences between the historical run, baseline and instrumented run are
not a controlled speedup measurement.

Interface placement also matters: moving x=24..34/52..62 to
x=20..38/48..66, with the same fine h and transverse extent, reduces the
worst probe error from 0.613647 to 0.550911 V. It adds unknowns and increases
compute time; a narrower patch is not automatically an accurate patch.
The potential near `[31,16,0]` is sensitive to the coarse trace only a few
fine cells from the electrode edge.

There is a separate surface-mask confound. `CutCells::new_patch` inherits
`snap_distance = 2 h^2 / L`, but L is the *patch* maximum extent. Both manual
einzel patches use L=40 mm and snap=0.05 mm, whereas uniform h=1 uses
L=90 mm and snap=0.022222 mm. Thus identical fine h need not produce identical
snapped conductor masks. The wider-patch experiment keeps the same 0.05 mm
snap and still improves error, so the whole error difference cannot be
attributed to snapping. Changing that policy needs a physical-domain scale
passed into cut-cell preparation and a subcell-phase convergence ladder;
it is documented rather than silently changing the established cut-cell
contract in this feature.

Finally, a cylindrical r=19 mm mirror and a square `[-20,20]^2` exterior
are different boundary-value problems. The h=0.5 mm cylindrical reference
differs from the Cartesian reference by up to 0.0619933 V at these probes.
This finite-grid comparison includes discretization differences and is not
a certified pure boundary-condition error. It prevents treating the
Cartesian probe results as cylindrical/SIMION parity.

The next solver changes should be measured separately: preserve a common
physical snap scale; build a patch-aware multilevel/FAC correction rather
than one coarse MG plus diagonal fine smoothing; cache voltage-independent
composite geometry, CSR and transfers across bases; and replace per-energy
ordered-map insertion with deterministic sorted sparse accumulation.
Supporting nested/touching patches and transition halos is needed before a
surface ring can stay a ring instead of merging into its bounding box.

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
