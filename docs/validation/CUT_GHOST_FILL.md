# Exact-surface vacuum continuation

Measured October 27, 2026, CPU f64, Ryzen 9 5950X / WSL2.
This change prepares ghost values for the existing reconstruction; it does not
change the symmetric cut-cell potential solver, its snapping rule, or the
shared-jet/quintic Hermite assembly.

## Reconstruction and voltage contract

Metal knots within eight Chebyshev layers of vacuum are fitted using original
vacuum values and exact grid-line surface intersections at the electrode
voltage. Total-degree 3, 2, then 1 fits use pivoted Householder QR, at least
1.5 samples per coefficient and an infinity-norm triangular condition limit
of `1e8`. Sample weights are `1/(r^2 + (0.1*h_min)^2)`. Up to twice the
coefficient count of nearest vacuum samples and once that count of nearest
surface samples are selected in a ten-cell search cube. Geometry alone
determines data selection, pivots, rank and fallback degree.

An electrode needs flagged PA metal nodes with one constant potential.
Otherwise it is skipped and reported. Backgrounds and unit-voltage bases
are filled independently, including when bases are attached after the surface.
The regression checks `fill(-3.7*b1 + 2.3*b2)` against
`-3.7*fill(b1) + 2.3*fill(b2)` within `3e-12` V. Original PA potentials and
electrode flags are retained. Failed linear fits keep their original values.
The additive flight JSON `ghost_fill` report leaves schema version 1 intact.
Adaptive fields refresh their owned base and patch grids in either attachment
order. Patch intersections are recomputed from source geometry at the patch
spacing and origin; coarse cut fractions are not reused on a different grid.
Reported counts include those patches, and skipped electrode numbers are
the union of skipped numbers over individual grids.

Three layers were measured and rejected: at n=257 the exact-data band field
maximum remained `1.4900e-2` (sphere) and `9.1519e-3` (cylinder). The spline
solve has a nonlocal decaying tail despite its four-knot one-sided closure.
Eight layers reduce these maxima to `1.4792e-3` and `4.7858e-4` and restore
the orders below. This is measured reach, not a claim of compact support.

## Band convergence

Radii are 0.31/0.83 mm, electrode voltages 1/0 V, domain `[-1,1]`,
`h=2/(n-1)`, n=65/129/257, zero translation phase. Sphere nodal data are
solved on the 3D grid and reconstructed on its equatorial slice; cylinder
data use the planar grid. The 256 non-grid angles are
`2*pi*(j+0.371)/256`. Probes lie in physical vacuum at
`{0.1,0.4,1.1,1.9}*h` from each surface. Potential norms contain 2048
samples; field norms contain 4096 active-component errors. RMS is normalized,
not a shrinking-volume L2 norm. Endpoint order is `log(error65/error257)/log(4)`.

Exact vacuum data isolate reconstruction. The analytic-continuation control
also replaces metal values using the known analytic solution; production
filling never uses that analytic solution.

| Geometry / data | Potential max order | Field max order | Potential RMS order | Field RMS order |
| --- | ---: | ---: | ---: | ---: |
| Sphere, exact vacuum / original metal | 0.945 | 0.002 | 0.875 | -0.031 |
| Sphere, analytic-continuation control | 3.734 | 2.767 | 3.633 | 2.834 |
| Sphere, exact vacuum / numerical fill | 3.671 | 2.832 | 3.839 | 2.897 |
| Sphere, solved / original metal | 0.991 | -0.008 | 0.908 | -0.034 |
| Sphere, solved / numerical fill | 1.727 | 0.330 | 1.760 | 0.421 |
| Cylinder, exact vacuum / original metal | 0.936 | -0.014 | 0.883 | -0.038 |
| Cylinder, analytic-continuation control | 3.888 | 2.925 | 3.674 | 2.878 |
| Cylinder, exact vacuum / numerical fill | 3.705 | 2.858 | 3.888 | 2.945 |
| Cylinder, solved / original metal | 0.959 | -0.021 | 0.895 | -0.041 |
| Cylinder, solved / numerical fill | 1.746 | 0.329 | 1.868 | 0.466 |

The solved-data field does **not** establish first-order convergence over
this whole ladder: maxima/RMS increase between 65 and 129, then decrease
approximately first order from 129 to 257. Numerical filling fixes the
interpolation-only boundary displacement; it does not remove the cut-cell
solution's boundary-layer error or manufacture a better solved-data order.
Existing nodal-solver gates and their bounds are unchanged.

| Geometry / filled data | n | Potential max | Potential RMS | Field max | Field RMS |
| --- | ---: | ---: | ---: | ---: | ---: |
| Sphere / exact | 65 | 4.284265e-4 | 8.980468e-5 | 7.498241e-2 | 1.267983e-2 |
| Sphere / exact | 129 | 3.700450e-5 | 6.575771e-6 | 1.131537e-2 | 1.735585e-3 |
| Sphere / exact | 257 | 2.640249e-6 | 4.387629e-7 | 1.479201e-3 | 2.286292e-4 |
| Cylinder / exact | 65 | 1.523242e-4 | 3.051356e-5 | 2.514837e-2 | 4.176819e-3 |
| Cylinder / exact | 129 | 1.327960e-5 | 2.170013e-6 | 3.836789e-3 | 5.483823e-4 |
| Cylinder / exact | 257 | 8.962544e-7 | 1.393011e-7 | 4.785849e-4 | 7.042753e-5 |
| Sphere / solved | 65 | 2.525632e-3 | 8.228137e-4 | 9.929426e-2 | 1.699367e-2 |
| Sphere / solved | 129 | 8.431786e-4 | 2.716169e-4 | 1.243198e-1 | 1.813337e-2 |
| Sphere / solved | 257 | 2.304016e-4 | 7.173008e-5 | 6.287975e-2 | 9.482524e-3 |
| Cylinder / solved | 65 | 8.284614e-4 | 2.977294e-4 | 3.231299e-2 | 5.548195e-3 |
| Cylinder / solved | 129 | 4.606403e-4 | 1.033206e-4 | 3.917259e-2 | 6.274700e-3 |
| Cylinder / solved | 257 | 7.366156e-5 | 2.233005e-5 | 2.048815e-2 | 2.905973e-3 |

All four shapes/data combinations use the same geometry-only fit decisions:

| n | Degree 3 | Degree 2 | Degree 1 | Kept / skipped electrodes |
| --- | ---: | ---: | ---: | ---: |
| 65 | 2017 | 0 | 0 | 0 / 0 |
| 129 | 4652 | 0 | 0 | 0 / 0 |
| 257 | 9315 | 25 | 0 | 0 / 0 |

Fixed physical interior probes use radii 0.42, 0.47, 0.57, 0.67, 0.72 mm.
At n=257 the filled exact-data potential/field maxima are
`1.120738e-8 / 3.503206e-6` (sphere) and
`3.726463e-9 / 7.623334e-7` (cylinder), agreeing with analytic continuation
to about `1e-14 / 1e-11`. Original vacuum nodal potentials are unchanged
exactly; off-node interior samples need not retain their bits because spline
derivatives are nonlocal. No-cut sample and flight bits are separately tested.

## Spherical analyzer

Negative singly charged 1 amu ions start tangent to radius 0.57 mm (center)
or 0.34 mm (near plate), fly 90 degrees to x=0, and hit a 0.005 mm radial
slit centered on the launch radius. The centroid is the mean of the two
slit-edge energies found by 30 bisections, not a full transmission spectrum.
Exact centroids are 0.4340418353576 and 0.7276583710407 eV.
RK45 uses explicit `1e-11` tolerances, maximum step 0.002 us, and disables
automatic field tolerance. An analytic-field flight verifies the reference
to the energy-root resolution.

| n | Center bias before | Center bias after | Near-plate bias before | Near-plate bias after |
| --- | ---: | ---: | ---: | ---: |
| 65 | +0.011795% | +0.012013% | +4.875481% | -0.425666% |
| 129 | -0.003785% | -0.003785% | -1.170657% | -0.058985% |
| 257 | -0.001181% | -0.001181% | -0.060209% | -0.025610% |

These are solved-data flights. After filling, near-plate energy errors are
`-3.097392e-3`, `-4.292076e-4`, `-1.863518e-4` eV: decreasing absolute bias
throughout this ladder. Center-orbit behavior remains effectively unchanged.
Exact-vacuum-data filled near-plate biases are `+0.175664%`, `-0.005501%`,
`-0.000019%`; analytic continuation gives `-0.014918%`, `+0.000109%`,
`+0.000005%`. The finite-grid continuation is not asserted to have the
analytic control's error coefficient. The near-plate ray leaves the shrinking
surface band on refinement, so these energy ratios are not an asymptotic GCI.
Tracing every accepted state of every energy-root flight keeps all rays in
physical vacuum; filled solved-data minimum clearances are
0.0142302, 0.0133012, 0.0130824 mm.

## Quadrupole reference provenance and preparation cost

The published quadrupole RMS of 0.00343534 mm belongs to the
**pre-2026-10-28 reconstruction**. Its retained physical 8x reference was
also produced by SimRep, using a separate 481x289x289 cut-cell solve and
the old global cubic, not an analytic or strict-SIMION answer. Both 4x
and 8x references have now been regenerated with numerical ghost filling
and the memory-bounded global cubic; the coarse field still uses Hermite.

Against the retained reference, historical/current RMS is
0.003435341/0.003856358 mm. Against the regenerated reference it is
0.003459626/0.003878169 mm, with zero exit-class changes. The retained/new
8x reference difference is 0.000239850 mm RMS, and regenerated 4x/8x
refinement drift is 0.000548371 mm. The current-code comparison therefore
rounds to **0.00388 mm against the new reference**, not 0.00386 mm (which
uses the old reference). Both are below the retained 0.005 mm acceptance
bound. The metric moved within reference uncertainty; this is not evidence
of an independently established physical regression. The near-plate analyzer
improvement from +4.88% to -0.43%/-0.059%/-0.026% is separately measured
against its analytic solution.

The README, public report source and claims registry label the historical
reconstruction and put the current comparison and reference drift beside it.
The [bench record](../../bench/fly-real.md#vacuum-continuation-reconstruction-comparison)
preserves both comparisons. No private ESA report was changed.

Preparation adds geometry/data collection, one pivoted QR fit per eligible
metal knot, and a second existing jet/coefficient build. Fits are already
parallel over nodes. A separable sliding-window mask marks the eight-layer
Chebyshev band in O(nodes), avoiding fit-neighborhood searches deep inside
thick conductors. Prepared field coefficients are cached in `GridField`:
flight samples and time-dependent voltage evaluation do not repeat fitting.
Each dynamic unit basis is prepared once with its own PA voltage contract;
the static CPU fast-adjust route prepares one combined PA, equivalent by
the tested linearity. Cross-array QR/operator caching was not added: it
would need to account for different valid-electrode sets and retain the
large per-node data stencils, rather than being a small safe cache.

On the matched real 1,000-ion quadrupole, five interleaved serial processes
per executable (16 Rayon workers, after the other verification/reference
jobs finished) give median preparation **163.103499 -> 892.960587 ms** and
compute-only flight **51.667600 -> 50.363999 ms**. Preparation includes the
extra collection/fits and second reconstruction; no separate function-level
time split was measured. The small flight-time difference is not a speedup
claim. Original/updated `--no-cuts` flight CSVs are byte-identical by `cmp`,
SHA-256 `97ea53037f933a824d9697b42da45ef4be9608628fbe31e1a3769f98ecd1ec31`.

The real 121x37x37 quadrupole background fits 36783 knots cubically, 1811
quadratically and 1112 linearly, keeping/skipping none. For the independent
reference preparation, 4x uses degree-3/2/1 counts
310082/175352/116923 with one kept knot; 8x uses
1202731/757202/1118857 with no kept knots. Neither skips an electrode.
These fallbacks are reported, not silently promoted to cubic fits.

## Reproduction and CI gates

```sh
export CARGO_TARGET_DIR=/tmp/simrep-ghost-st01
export CARGO_BUILD_JOBS=8 RAYON_NUM_THREADS=8
DIAG_PHASE0=1 DIAG_TRACE=1 \
DIAG_DATA=solved,clamped,continuation,filled-solved,filled-clamped \
cargo run --release -p simrep-fly --example cutcell_diagnosis -- default 65 129 257
cargo run --release -p simrep-fly --example cutcell_cost
cargo test --release -p simrep-fly --test surface_reconstruction -- --nocapture
```

The CI-fast gates use n=33/65/129 and 128 angular probes, with separate
max/RMS potential/field rows for sphere and cylinder. They retain the
smooth exact-data order references 4/3 and report observed endpoint and
minimum adjacent orders as OPEN findings, not uncertainty estimates.
Regression acceptance requires endpoint potential order above 3 and field
order above 2. No existing gate bound was loosened.

Unit/integration regressions also cover 3D polynomial reproduction,
cylindrical axis/revolution mapping, rank fallback, tiny fractions,
missing/nonconstant voltages, basis linearity, both attachment orders,
FLY2 initial queries, additive JSON counts and non-cut bit identity.
GPU staircase flights remain on the original reconstruction path.

## Verification

Final `cargo fmt --all --check`, workspace/all-target Clippy with
`-D warnings`, explicit `simrep-py --features python` Clippy, and
`cargo test --workspace --release` pass. The release run reports
356 standard tests passed, zero failed, 17 opt-in/fixture/hardware-gated tests
ignored; custom Vulkan solver/flight/PIC harnesses also pass on llvmpipe.
DX12/hardware-GPU execution is not claimed. The LSP daemon timed out;
strict compiler/Clippy diagnostics provide the successful source check.

Eight new computed band rows are registered without changing any existing
gate row or bound. The claims registry includes four additional checked
README entries for the historical RMS, current/retained-reference RMS,
current/regenerated-reference RMS and regenerated-reference drift.
