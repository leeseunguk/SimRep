# Geometry to a refined fast-adjust field

`refine-geometry` is an additive command. GEM conversion, mesh import,
staircase `refine`, and the flyer retain their existing contracts.

```sh
cargo build --release -p simrep-cli
target/release/simrep refine-geometry lens.gem \
  --electrode 1=1:0 --electrode 2=2:110 --electrode 3=3:0 \
  --device cpu --out target/lens
target/release/simrep refine-geometry lens.stl \
  --grid 81,81,81 --spacing 0.5,0.5,0.5 --origin -20,-20,-20 \
  --electrode 0=1:0 --electrode 1=2:110 --repair \
  --device gpu --backend vulkan --out target/lens-mesh
```

## Coordinates and electrode assignment

Coordinates, distances, and spacing are in millimeters; nodes are x-fastest.
GEM defaults to `pa_define` dimensions, spacing, symmetry, mirrors, and `ng`.
`--grid nx,ny,nz`, `--spacing sx,sy,sz`, and `--origin x,y,z` override the
solve lattice without modifying the source geometry. Origin defaults to zero.
For meshes, grid dimensions are required and spacing defaults to 1 mm.
STL, OBJ, PLY, 3MF, glTF, and GLB use `simrep-mesh`'s format/scene/unit rules.
`--units` overrides mesh source units; `--repair` uses its safe repair path.
Open/non-manifold meshes are rejected rather than silently defining a solid.

Each `--electrode part=number:voltage` assigns one input part. For GEM,
`part` is the original source electrode value (for example `2` in `e(2)`).
For meshes it is the printed part name or zero-based import index. Every
part must have exactly one unambiguous assignment, and unused selectors
are errors. Several parts can share a number if their voltage agrees.
Fast-adjust numbers are 1 through 1000; gaps are permitted. Every represented
number must have an inside/on-surface grid node for its PA0 representative.
Different-number overlaps are rejected, including overlapping snap regions.

Cylindrical GEM PAs retain axial-x/radial-y storage. The CLI transposes to
the solver's radial-x/axial-y coordinates and back. Radial and z origins must
remain zero because PA files/flying represent a radial axis, not an annulus
with a missing core. Use noninteger spacing or an axial shift to change
alignment. Mesh refinement is planar 3D. Only refinable electrostatic GEM
geometry is accepted by this command.

## Geometry and solver boundary

### Inter-electrode gap resolution

Before constructing masks, cuts, or output files, `refine-geometry` measures
each pair of different electrode numbers. `--gap-check warn` is the default:
it warns below **2.5 cells across** a gap and reports the spacing for
**10 cells across**. The 2.5-cell threshold is the resolvability rule from
Iteration-CFD's site-mesh guidance; 10 cells is accuracy guidance, not an
error estimate or a replacement for grid convergence. Here `cells_across=g/h`,
where `h` is the largest spacing on active PA axes (conservative for
anisotropic grids). The recommended spacing is `g/10`, using the smallest
pair gap. These checks do not measure thin features of a single electrode.

`--gap-check error` fails before writing any PA or cuts file when a pair has
fewer than 2.5 cells across or a GEM pair is unresolved. `off` skips all gap
measurements. The check only reads source geometry; PA and cuts bytes are
identical with checking enabled or disabled.

Mesh gaps are Euclidean triangle-surface minima, using simultaneous BVH
traversal, AABB lower-bound pruning, and face/edge/vertex witnesses. CAD gaps
use exact placed B-rep face sets and an independent tessellation cross-check,
even when the solve uses `--cad-mode tessellated`. These are surface
separations, not solid-overlap tests; nested boundaries may have positive
separation. Parts sharing a number are not checked against each other; the
reported number-pair gap is the minimum over their different-number part pairs.

GEM CSG distances are conservative bounds, **not Euclidean distances**.
The adaptive search maintains a bracket `[lower,upper]` in mm. It covers the
enclosing hull of the electrodes' conservative source bounds, rather than
only their overlap (disjoint bounds have no overlap). On each box,
`abs(d_A(center))+abs(d_B(center))-box_diagonal` is a lower bound;
the distance between the source bounds supplies another lower bound.
Boxes whose lower bound exceeds the best upper bound are discarded.
Upper bounds come from sign-changing surface brackets found with gradient
projection and bisection; the physical bracket widths are added to the
witness distance, rather than treating a small SDF value as a surface error.

The search stops at bracket width `min(h/10, 0.001 mm)` or 32,768 box
subdivisions per source pair. GEM results are certified **only as the
reported bracket**, not as an exact gap. Conservative CSG fields can retain
a loose lower bound even after spatial refinement. Such pairs are reported
as `unresolved`, with their bracket, and warn in default mode. Unbounded
source axes are searched within the PA extent for upper-bound witnesses,
but only the global source-box separation supplies their lower bound (zero
when the boxes overlap): finite PA samples cannot prove a global gap.
If no witness is found, the upper endpoint and
location are `null` (no finite upper bound). Resolution and recommended
spacing use the lower endpoint, never a guessed midpoint.

The standard `--json` summary keeps schema version 1 and adds `gap_check`:
`mode`, `threshold_cells` (2.5), `recommended_cells` (10), `pairs`, and
`recommended_spacing_mm`. Each pair has `electrodes`, `method`
(`exact` or `bounded`), either `gap_mm` or `gap_bracket_mm`, `cells_across`,
`location_mm` (two witness points in electrode order), and `status`
(`ok`, `warn`, or `unresolved`). The recommendation is `null` when there are
no checked pairs. Off mode returns an empty `pairs` array. The same
measurements and guidance appear in the human log.

GEM's physical CSG SDFs are evaluated directly, without staircase rasterization
margins or an intermediate lattice. Primitive distances are combined after
evaluation, so aperture/cap corners retain their source sign and zero set.
Mesh shapes use `Solid::signed_distance` directly, avoiding an extra sampled
mesh-SDF interpolation error. No dependencies were added between the geometry
importers and the solver.

The additive `CutCells::new_with_root_tolerance` lets this adapter tighten
root stopping distances to `1e-9*h_min` without changing
the quadratic snap rule or the existing constructor's defaults. This matters
for alignment: stopping a scaled conservative SDF at the usual snap-derived
distance slightly before an on-node surface would change an otherwise
identical staircase operator. The sidecar uses the additive
`cutcell::surface_fraction` helper and the same tight physical distance.

Physical PA electrode membership is `SDF <= 0`, treating
`abs(SDF) <= 1e-9 * h_min` as zero. Surface nodes are electrodes.
The solver independently snaps positive-distance nodes within its quadratic
small-cut distance; those nodes remain **non-electrodes in every output PA**.
Each 10000 V basis is solved with the symmetric ghost-fluid operator and
the same fixed geometry. Actual assigned voltages are combined in PA0.
Residual exhaustion fails the command. `--tolerance` defaults to `1e-10`
and `--max-cycles` to 100.

## Outputs and flying

For `--out target/lens`, the command writes:

* `lens.pa0`: the field at the assigned voltages and SIMION PA0 metadata.
* `lens.pa1`, `lens.pa2`, etc.: represented 10000 V fast-adjust bases.
* `lens.pa#`: the physical nodal electrode-number definition for a same-input
  staircase comparison using `simrep refine`.
* `lens.cuts.json`: versioned geometry data in original PA node order.

All PA siblings have identical physical electrode flags. PA0 representative
indices point to flagged nodes, preserving `read_fast_adjust` validation.
The PA format cannot store a physical grid origin; provide the same placement
to the flyer:

```sh
target/release/simrep fly --pa target/lens.pa0 --particles ions.ion \
  --voltage 1=0,2=120,3=0 --origin 0,0,0 --output target/ions.csv
```

The version 3 sidecar has `version`, `dims`, `origin_mm`, `spacing_mm`, `symmetry`,
`assignments`, `mirrors`, `electrode_numbers`, `source_geometry`,
`surface_tolerance_mm`, and `snap_distance_mm`. Source geometry is the default
flight representation; dense sampled SDFs are optional.

Default v3 files store the v1 node/link information losslessly in compact form:
`node_electrode_runs` and `solver_fixed_runs` contain `[start,end,value]` records
covering the entire x-fastest node array in order, with half-open index ranges.
Node values are an electrode number or `null`; solver-fixed values are booleans.
`link_intersections` contains only nodes with physical hits, as
`{node,fractions,electrodes}` records in strictly increasing node order.
Unlisted nodes have six null fractions/electrode numbers. Each listed row has
six entries ordered `-x,+x,-y,+y,-z,+z`; a missing hit or outer-domain link is
JSON `null`. Fractions are the first physical surface
intersection divided by link length, including intersections from
positive-distance nodes snapped by the solver. Physical conductor start
nodes have null fractions. A conductor entirely between free endpoints is
detected by conservative sphere tracing.

`--sidecar-dense-sdf` opts into the old dense compatibility representation:
`node_electrodes`, `solver_fixed`, `cut_fractions`, and `cut_electrodes` each have
`nx*ny*nz` entries, and `sampled_sdfs` retains the full v2 distance fallback.
Version 2 retains all version 1 dense link data and adds `mirrors` (x,y,z
booleans), `electrode_numbers`, and `sampled_sdfs`.
The latter contains one union SDF per electrode number, in the same order as
`electrode_numbers`: `number`, `dims`, `origin_mm`, `spacing_mm`, and x-fastest
`values` in mm, negative inside the conductor. Full-domain samples are stored.
`--surface-supersampling N` sets fallback samples per PA interval (default 2)
with `--sidecar-dense-sdf`; a PA axis with one point still has one sample.
Default v3 files have an empty `sampled_sdfs` array (readers also accept its
absence). Sampling and serialization are preparation, not solver compute time.

V3 adds `source_geometry`, an array of `{number, geometry}` records. Several
parts may share a number; their conductor union is evaluated at query time.
The records must cover exactly `electrode_numbers`. Each `geometry` has an
explicit `encoding` and base64 `data`:

* `gem-csg-bincode1-base64`: the ordered analytic CSG tree, including electrode
  values, vacuum overwrites, transforms and mm/grid-unit scaling. The payload
  uses bincode 1 fixed-width little-endian encoding of `ElectrodeSdf`: voltage,
  fills, coordinate scale. A fill is electrode flag, potential, edge flag,
  shape. Shape/primitive/region/transform fields and enum variant order are
  defined by `simrep-gem/src/{lib,geometry}.rs` and frozen for this encoding.
  IEEE f64 parameters retain infinite primitive extents without JSON nulls.
  Runtime CSG combines exact primitive bounds, never interpolated composites.
* `mesh-f64-u64-le-base64`: two u64 little-endian counts (vertices, triangles),
  then xyz f64 little-endian vertices (24 bytes each), then three u64
  little-endian vertex indices per triangle (24 bytes each). No quantization,
  platform-sized integers, normals or BVH nodes are stored. The loader validates
  closed manifold topology and rebuilds the BVH. `chordal_tolerance_mm` is null
  for triangle-file inputs and records the requested deflection for CAD.

STEP/IGES refinement may use exact OCCT B-rep distances, but its portable v3
flight geometry is always the repaired fine tessellation at that deflection.
Flight does not load OCCT or query B-reps. Tessellation error remains; exact
ray/triangle intersections mean exact to these stored triangles.
Changing a payload's layout requires a new encoding/version. Unknown versions,
encodings, corrupt payloads and mismatched electrode maps fail explicitly.
Old dense v1/v2/v3 sidecars still load. Readers use v3 source geometry directly;
v1/v2 files keep their existing approximate reconstruction. A v3 reader does not
silently downgrade a damaged source payload. For older readers, generate with
`--sidecar-dense-sdf`: an older v2 reader can consume a copy with `version` set
to 2 and ignores the extra source field. Compact v3 files require a reader that
supports the run and sparse-link fields; changing only their version to 2 is
not a valid conversion. Source binary encodings are unchanged.

`fly` automatically loads the sibling `<stem>.cuts.json`, or accepts
`--cuts path/to/geometry.json`. `--no-cuts` disables pickup for a staircase
comparison. A malformed, incompatible, or explicitly missing sidecar fails;
it is not silently ignored. V3 initial membership uses source CSG or mesh
classification. Conservative segment tracing on exact composite CSG finds the
first conductor even when both accepted
step endpoints are vacuum; bracketed roots are bisected and the flight is
re-integrated to the hit time. The CLI ties localization to the flight position
tolerance divided by placement scale. Triangle segments use BVH ray/triangle
hits, including coplanar face/edge contact; mirrored segments are split at
reflection planes and clipped to the PA domain before triangle queries.
Source-grid origins are carried by the sidecar;
`--origin` still places PA node zero in the workbench, so use the source origin
there too when source and workbench coordinates should coincide.

V1 and V2 sidecars remain loadable and are reported as **APPROXIMATE**. V2 uses
trilinear sampled SDFs and can round CSG corners regardless of supersampling.
V1 nodal conductor
signs and incident link distances define continuous trilinear sign fields with
watertight cell interfaces. Conflicting link constraints, curved surfaces and
conductors entirely between free nodes cannot be recovered uniquely from V1.
Regenerate V3 when those features matter. Legacy V1 omitted mirror flags; its
default is unmirrored and incompatible mirrored PAs are rejected.

The Rust API provides `surface::SurfaceModel::from_reader` and `from_sdfs`;
the latter accepts numbered `Arc<dyn simrep_geom::SignedDistance>` sources
without resampling. For source geometry with PA symmetry or a nonzero source
origin, call `.with_pa_domain(&array, source_origin_mm)` before attaching it.
Attach either with `InstanceField::with_surface`.
Field reconstruction remains independent of collision geometry (its boundary-aware
quintic Hermite math is described in `crates/simrep-fly/README.md` and
`docs/spec/equations.md`). Grid exits,
workbench bounds, detector planes, placement, scaling, mirrors and cylindrical
axial-x/radial-y revolution retain their event semantics.
`SplatReason::SurfaceElectrode { number }` reports the physical electrode number;
`Electrode { index }` still means the PA node index. CLI CSV appends an
`electrode_number` column, empty for staircase or non-electrode events.
Surface collisions currently run on CPU: `--device auto` selects CPU even for
large beams; explicit `--device gpu` with a surface returns a clear error.

### Historical V2 surface-flight acceptance (October 23, 2026)

```sh
cargo build --release -p simrep-cli --bin simrep --example geometry_pipeline
RAYON_NUM_THREADS=16 target/release/examples/geometry_pipeline surface
RAYON_NUM_THREADS=16 target/release/examples/geometry_pipeline surface-einzel
```

The original `surface` run refined generated closed sphere/cylinder STL files,
loaded v2 sidecars, and compared first hits with analytic source SDF roots. It also
replaces the samples with analytic samples on the identical lattice to separate
tessellation from sampling error. Sampling factors 1, 2, 4 and 8 keep the PA
spacing fixed at 0.2 mm. The sphere has 146,688 triangles, the closed cylinder
8,192; both have radius 0.63 mm. Rays cross the complete solid with two vacuum
endpoints. The harness measures 10,000 repeated segment queries with setup
excluded; those timings are not full-flight throughput.

| Surface | Sampling factor | Analytic SDF hit error mm | Generated STL hit error mm |
| --- | ---: | ---: | ---: |
| Sphere | 1 | 0.01247767 | 0.01250609 |
| Sphere | 2 | 0.00259958 | 0.00263303 |
| Sphere | 4 | 0.00091512 | 0.00094478 |
| Sphere | 8 | 0.00017100 | 0.00019896 |
| Cylinder | 1 | 0.00378780 | 0.00378808 |
| Cylinder | 2 | 0.00155808 | 0.00155844 |
| Cylinder | 4 | 0.00048735 | 0.00048771 |
| Cylinder | 8 | 0.00004217 | 0.00004276 |

These are along-ray hit-position errors, which also bound the radial error.
The factor-1/factor-8 STL reductions are 62.9x and 88.6x, consistent with
second-order sampling error over an eightfold sampling refinement, with
cell-phase variation. At default factor 2 both errors are below 1.4% of PA h.
The same-grid nearest-node staircase errors are 0.09495798 and 0.10578131 mm.
The independent 37-ray regression gives the following RMS staircase errors,
showing first-order scaling with PA spacing rather than SDF sampling.

| PA h mm | Sphere nearest-node RMS mm | Cylinder nearest-node RMS mm |
| --- | ---: | ---: |
| 0.2 | 0.07074297 | 0.07702275 |
| 0.1 | 0.03356060 | 0.03705490 |
| 0.05 | 0.01585307 | 0.01556115 |

All staircase measurements in this subsection use **nearest-node voxels**,
not SIMION's all-corner node-cell convention. The harness explicitly selects
nearest-node collisions on builds offering that selector; the measurement
baseline predates the all-corner default.

`surface-einzel` independently refines physical-surface 4x and 8x fields
(0.25 and 0.125 mm), then compares the non-aligned 92x21 cut-cell PA using
surface and staircase splats on the **same field**. The installed 1,000-ion
subset clears every electrode, so its endpoints are identical in those two
collision modes: RMS error versus physical 8x is 0.35875317 mm, dominated by the
coarse field rather than splat geometry. The physical 4x/8x endpoint RMS
difference is 0.04699807 mm (maximum 0.14047484 mm); the fine reference is not
claimed analytically exact.

To measure actual lens-wall collisions, the harness adds 36 deterministic
neutral radial impacts across all three physical electrodes. Neutrals isolate
collision error from field error. Their endpoint RMS error versus physical 8x is
`4.65007e-11 mm` for surface splats versus `0.425000 mm` for staircase splats.
Across the combined 1,036-particle population the RMS is 0.35246490 versus
0.36125898 mm. With 16 Rayon threads on the Ryzen 9 5950X, an isolated serial
release repeat measured 0.168595 s surface flight versus 0.132176 s nearest-node
staircase flight: **1.276x compute overhead**. The first harness run recorded
1.319x in `surface-einzel-results.json`; these are single runs, not medians.
Setup, sidecar loading/sampling, output and GPU initialization
are excluded. These runs validate geometry and report cost; they do not claim
matched-precision speedup over SIMION.

### Historical V2 quadrupole user-route validation

The staged quadrupole GEM was regenerated through the actual `refine-geometry`
command, preserving the supplied v1 control. Outputs are
`C:\Users\lsu10\simrep-oracle\_quad_cut\cli-v2.*` (default 2x sidecar) and
`cli-v2-4x.*` (4x sidecar). Both use the full 121x37x37 quadrant PA at 0.5 mm,
Y/Z mirrors, and voltages `1=2.5,2=-2.5,3=0`. All three bases converged below
`1e-12` relative residual. Flights used the original 1,000 ions, tolerance
`1e-11`, maximum step 0.025 us, relativistic dynamics, bounds
`0,-18,-18,60,18,18`, and an explicit `--cuts` path.

The independent physical 8x reference is a separately solved 481x289x289
quarter/half-domain field at 0.0625 mm with exact-SDF splats, not an upsampled
coarse PA. Its supplied 4x/8x endpoint RMS/max difference is
0.000696157/0.00480402 mm with no exit-class changes.

| CLI reconstruction | Sidecar sampling | RMS vs physical 8x mm | Max mm | Exit-class changes | RMS for matching classes mm | Matching-class max mm |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Local default boundary-aware | 2x | 0.19562950 | 6.18309971 | 1/1000 | 0.00634197 | 0.06868403 |
| `--global-cubic` | 2x | 0.19562733 | 6.18318100 | 1/1000 | 0.00619390 | 0.06902721 |
| `--global-cubic` | 4x | 0.19559345 | 6.18318100 | 1/1000 | 0.00500945 | 0.05038442 |

Ion 972 dominates the aggregate error: the sampled-GEM CLI route exits at
x=60 mm, while the physical reference hits the downstream aperture at
x=53.85885743 mm, y=0.44401776 mm, z=-3.97527964 mm (radius 4 mm).
Increasing sidecar sampling from 2x to the existing GEM source's 4x lattice
improves the other 999 ions but does not remove that class change.
These results do **not** reproduce the supplied exact-SDF coarse-control
0.00343527 mm RMS / 0.0201892 mm maximum with zero class changes. The comparison
does not separate the existing sampled GEM geometry, its effect on the coarse
cut field, and subsequent sidecar interpolation. It is a remaining physical
accuracy limitation of this user route, not an exact-surface acceptance claim.

The terminal files are `cli-v2-flight.csv`, `cli-v2-global-flight.csv`, and
`cli-v2-4x-global-flight.csv` in the staged folder. Reproduce the global-cubic
default-sampling comparison with:

```sh
Q=/mnt/c/Users/lsu10/simrep-oracle/_quad_cut
target/release/simrep fly --device cpu --global-cubic \
  --pa "$Q/cli-v2.pa0" --cuts "$Q/cli-v2.cuts.json" \
  --particles /mnt/c/Users/lsu10/simrep-oracle/_quad_gpu/particles.ion \
  --voltage 1=2.5,2=-2.5,3=0 --bounds=0,-18,-18,60,18,18 \
  --tolerance 1e-11 --max-step-us 0.025 --output "$Q/cli-v2-global-flight.csv"
python3 "$Q/compare_quadrupole.py" \
  "$Q/physical-8-global.csv" "$Q/cli-v2-global-flight.csv"
```

### V3 quadrupole user-route validation (October 23, 2026)

The actual `refine-geometry quadrupole.gem` -> `fly --cuts` route now gives
**0.00343534 mm endpoint RMS**, **0.02018961 mm maximum**, and **0/1000 exit-class
changes** against the independent physical 8x reference. This passes the
0.005 mm RMS / zero-class-change requirement using the default boundary-aware
field reconstruction. All three exact-source coarse-grid bases converged below
1e-12 relative residual. Ion 972 hits the aperture rather than escaping.
The reference's separately measured physical 4x/8x RMS difference remains
0.000696157 mm; it is a converged numerical reference, not an analytic solution.

```sh
Q=/mnt/c/Users/lsu10/simrep-oracle/_quad_cut
mkdir -p target/cuts-v3
RAYON_NUM_THREADS=16 target/release/simrep refine-geometry "$Q/quadrupole.gem" \
  --electrode 1=1:2.5 --electrode 2=2:-2.5 --electrode 3=3:0 \
  --tolerance 1e-12 --out target/cuts-v3/quadrupole
RAYON_NUM_THREADS=16 target/release/simrep fly --device cpu \
  --pa target/cuts-v3/quadrupole.pa0 --cuts target/cuts-v3/quadrupole.cuts.json \
  --particles /mnt/c/Users/lsu10/simrep-oracle/_quad_gpu/particles.ion \
  --voltage 1=2.5,2=-2.5,3=0 --bounds=0,-18,-18,60,18,18 \
  --tolerance 1e-11 --max-step-us 0.025 --output target/cuts-v3/quadrupole.csv
python3 "$Q/compare_quadrupole.py" "$Q/physical-8-global.csv" \
  target/cuts-v3/quadrupole.csv
```

The grazing-aperture regression round-trips source CSG through v3, checks exact
initial membership and first entry at the cap, and demonstrates that the same
v2 fallback misses the metal. Additional regressions cover transformed grid
units/infinite primitive extents, portable mesh payloads, finite segments,
shared triangle vertices/edges and coplanar contact.

The v3 STL sphere/cylinder harness also passes. Along-ray errors against the
analytic surfaces are **2.740999e-5 mm** (sphere) and **6.564742e-7 mm**
(cylinder), identical for fallback sampling factors 1, 2, 4 and 8. These are
triangle tessellation errors, not SDF-lattice errors. The harness separately
downgrades the analytic samples to v2 and retains its sampling-convergence
checks. Its 10,000 isolated segment-query timings are not whole-flight costs.

The einzel harness passes against independently refined physical 4x/8x fields.
All 36 neutral wall impacts match physical 8x exactly at printed precision
(0 mm RMS), versus 0.425 mm RMS with nearest-node staircase collisions. For
the original 1,000 ions the coarse field dominates: 0.42564868 mm RMS against
physical 8x; the physical 4x/8x RMS difference is 0.04699796 mm. This is not a
claim of 0.005 mm field accuracy for einzel.

#### Source-only sidecars and narrow-band mesh sampling (October 24, 2026)

The default writer no longer samples the redundant v2 lattice. It stores the
unchanged v3 source payload, lossless node/solver runs, and sparse physical link
records. `--sidecar-dense-sdf --surface-supersampling N` restores the dense
compatibility output, including exact full-domain distance samples. The flag
does not change the PA, solver geometry, physical cut locations, or v3 flight
intersections.

`simrep_mesh::sample_sdf` now computes exact BVH closest-point distances only
within `|d| <= 2*h`, where `h` is the maximum grid spacing. Outside that band,
the magnitude is clamped to `2*h`; membership still comes from the same coherent
row ray classification, with the existing robust ray/winding fallback for
degenerate rows. Sampling is parallel over z slabs. Nearest-triangle seeds use
actual triangle distances, and BVH distance bounds exclude only triangles
farther than the band/current best distance. No distance propagation or
approximate closest points are used.

`SignedDistance::sample_grid` and point queries remain full-distance APIs.
The additive `sample_grid_band` API supplies the clamped grid; its default
implementation clamps ordinary samples, while meshes bound their BVH queries.
Regression grids cover aligned faces, spheres, a torus, nested cavities and
anisotropic spacing: every node has the same conductor flag, and every value
equals the exact pointwise distance clamped to the band within `1e-12` mm.
An additional full-grid comparison on the million-triangle torus checked all
16,777,216 nodes against the unchanged full-distance sampler: 412,928 nodes
were inside the band, maximum clamped-distance difference was exactly zero,
and zero conductor flags changed.
Clamped grids are intended for local cut geometry, not far-field distance
measurements. Dense v2 fallback samples continue using the full-distance API.

Release measurements on the same Ryzen 9 5950X use `RAYON_NUM_THREADS=16`.
The final controls ran serially after the release suite, surface harnesses and
dense compatibility comparison had finished.

| Case | Previous dense/full-distance preparation | Source-only / narrow-band preparation |
| --- | ---: | ---: |
| 256x256x256 solid-box GEM | 8.627 s at 2x; 4.551 s at 1x | **2.610 s**, median of three runs |
| 1,000,000-triangle torus, 256x256x256 SDF | 30.878 s | **3.727 s**, including topology and BVH |
| 121x37x37 quadrupole GEM, three bases | 0.791 s | 0.829 s, including 0.254 s source-file loading |

The three solid-box preparation times were 3.941, 2.610 and 2.399 s. A preceding
run immediately after the compatibility work took 4.503 s, dominated by PA
output, cut-cell setup and basis copying; the writer itself took 0.110 s.
The 3 s target is met as a three-run median, not as a cold-filesystem upper
bound. In the last control the breakdown was ownership 0.384 s, solver cuts
1.092 s, physical links 0.029 s, PA# output 0.328 s, sidecar 0.025 s and basis
remapping 0.289 s; the remainder is source loading and multigrid setup.
The mesh result is 8.28x faster than the previous full-distance sampler and
excludes mesh generation, sidecar writing and solving, exactly as before.

| Sidecar | Before, default dense 2x | Default source-only |
| --- | ---: | ---: |
| 256-cubed solid box | 1,967,039,985 bytes | **676 bytes** |
| Quadrupole | 57,115,706 bytes | **1,549,029 bytes** |

The box's uniform masks compress to one run each; it is not a general
compression ratio for arbitrary geometry. The quadrupole shrinks by 97.29%.
The full 256-cubed output regenerated with `--sidecar-dense-sdf` is byte-identical
to the previous 1,967,039,985-byte sidecar, and its PA0 is byte-identical.
The compact and dense CLI regression also expands every run and sparse hit
back to the old dense values and compares all PA files byte for byte.

The actual default quadrupole user route still gives
**0.003435341053 mm RMS**, **0.020189608873 mm maximum** and **0/1000 exit changes**
against physical 8x. Loading the old dense v3 sidecar with the new reader
produces a byte-identical flight CSV. V1/v2 approximate-reader tests remain
green. STL sphere/cylinder hit errors remain 2.740999e-5/6.564742e-7 mm at all
four sampling factors, and all 36 einzel neutral wall impacts still match
physical 8x exactly. The surface sampling harness explicitly opts into dense
output because it also exercises deliberate v2 downgrades.

Reproduce the default and dense controls using the solid-box GEM below:

```sh
RAYON_NUM_THREADS=16 target/release/simrep refine-geometry \
  /tmp/preparation-256.gem --electrode 1=1:0 --out /tmp/preparation-source
RAYON_NUM_THREADS=16 target/release/simrep refine-geometry \
  /tmp/preparation-256.gem --electrode 1=1:0 --sidecar-dense-sdf \
  --surface-supersampling 2 --out /tmp/preparation-dense
wc -c /tmp/preparation-{source,dense}.cuts.json
RAYON_NUM_THREADS=16 cargo run --release -p simrep-mesh \
  --example mesh_bench -- --sdf
```

Verification exited zero for `cargo fmt --all --check`,
`cargo clippy --workspace --all-targets -- -D warnings`,
`cargo clippy -p simrep-py --all-targets -- -D warnings`,
`RAYON_NUM_THREADS=16 cargo test --workspace --release`, and both
`geometry_pipeline surface` and `geometry_pipeline surface-einzel`.
Vulkan GPU tests passed on llvmpipe; DX12 had no adapter and used the existing
skip path. LSP diagnostics could not run because this toolchain lacks
`rust-analyzer`; both clippy invocations completed without warnings.

#### Preparation performance before the source-only default

Release measurements on the Ryzen 9 5950X with `RAYON_NUM_THREADS=16` separate
geometry preparation from the existing solver compute timer. The quadrupole
command above retains the default 2x fallback sampling and all v3 source data.
Final after-measurements and the default-2x/mesh controls were run serially
after formatting, clippy, and release tests completed.

| Case | Before | After | Timed work |
| --- | ---: | ---: | --- |
| 121x37x37 quadrupole GEM, three bases | 32.429 s | 0.791 s | Full preparation, including sidecar and solver setup |
| 256x256x256 solid-box GEM, 1x fallback sampling | 12.640 s | 4.551 s | Full preparation, including sidecar and solver setup |
| 256x256x256 solid-box GEM, default 2x fallback sampling | 25.753 s | 8.627 s | Full preparation, including sidecar and solver setup |
| 1,000,000-triangle torus, 256x256x256 SDF | 40.976 s | 30.878 s | Topology inspection, BVH construction, exact SDF sampling |

The quadrupole's final compute time was 0.546 s and its end-to-end time was
1.351 s. Its preparation breakdown was source loading 62.556 ms, physical
node ownership 39.899 ms, solver cuts 122.678 ms, physical links 163.018 ms,
PA# output plus fallback sampling 152.281 ms, and sidecar serialization
128.740 ms. Per-basis geometry remapping took 3.981, 1.694, and 0.930 ms;
the remaining preparation time is the existing multigrid setup.

The main bottleneck was tangent-link sphere tracing: the link from
`[34,0.5,5]` to `[34,0,5]` required 151,629 SDF evaluations. When analytic
CSG interval bounds prove that a segment's conductor sublevel set is one
interval, a near-tangent endpoint supplies a direct 48-step bracket instead.
Ordered overwrites, exclusions, and multiple possible entries retain the
generic tracer unless those conditions can be proved. Interval rejection
uses the existing root tolerance; it does not clamp or approximate stored
SDF samples.

Ownership, physical-link tracing, sampling, and bounded-size JSON chunks now
run in parallel. The solver cut geometry is built once and its Dirichlet
voltages are remapped for each basis. Physical cuts are stored sparsely in
memory, but the sidecar still contains the same full x-fastest arrays,
including null rows. Empty output blocks are reused rather than formatting
millions of identical null arrays. Mesh sampling batches inside/outside
classification by row and seeds each nearest-triangle search with the
previous sample's actual nearest triangle.

Against the pre-change quadrupole artifacts, the maximum PA0 difference is
3.553e-15 V and the maximum basis difference is 1.456e-11 V. Every decoded
sidecar field except `cut_fractions` is identical, including all fallback
SDF values, node ownership, solver flags, electrode labels, and the complete
v3 CSG payload. The maximum cut-fraction difference is 9.949e-10, or
4.975e-10 mm on this grid, within the 5e-10 mm surface tolerance. The actual
flight still gives **0.003435341 mm RMS**, **0.020189609 mm maximum**, and
**zero exit changes** against physical 8x.

The 256-cubed result does **not** meet the 3 s preparation target. Even this
fully fixed, single-electrode control retains the dense compatibility
sidecar and the existing solver setup. Its measured costs after optimization
were 1.473 s for cut-cell setup, 0.666 s for PA# output and samples, 1.217 s
for serialization, 0.439 s for basis remapping and copying, and approximately
0.359 s for multigrid setup; ownership and sparse physical-link collection
account for the rest. The mesh result is also not a sub-3 s claim, and
excludes sidecar writing and solving.
At the default 2x sampling, the same 256-cubed control requires 1.793 s for
PA# output plus sampling and 4.088 s for sidecar serialization alone. Its
sidecar is 1,967,039,985 bytes; the fallback samples and both dense cut arrays
are byte-identical under streaming SHA256 comparison, and the PA files are
byte-identical to the pre-change control.
These measurements predate the decision to make dense compatibility data
opt-in. That earlier optimization retained the dense schema and all samples.

Reproduce the large-grid control and the mesh SDF measurement with:

```sh
printf '%s\n' \
  "pa_define{nx=256,ny=256,nz=256,symmetry='planar',dx=1,surface='none'}" \
  'e(1){fill{within{box3d(-1,-1,-1,256,256,256)}}}' > /tmp/preparation-256.gem
RAYON_NUM_THREADS=16 target/release/simrep refine-geometry \
  /tmp/preparation-256.gem --electrode 1=1:0 --surface-supersampling 1 \
  --sidecar-dense-sdf \
  --out /tmp/preparation-256
RAYON_NUM_THREADS=16 cargo run --release -p simrep-mesh \
  --example mesh_bench -- --sdf
```

#### Exported quadrupole STL

The actual PA geometry export produced 106,800 triangles. STL reimport split
the four rods and two apertures into six closed components; repair reported
no defects. The full 121x73x73 mesh PA has origin `[0,-18,-18]` mm. All three
bases converged below 1e-12. The actual mesh CLI flight agrees with direct
`Solid::signed_distance` source flights on the same field to
**1.442479e-8 mm RMS**, **1.516489e-7 mm maximum**, with **zero exit changes**.
This controls triangle-hit/sidecar error independently of field error.

`export geometry` exports PA voxels, not an analytic tessellation of the GEM.
That STL therefore describes a different physical quadrupole: versus the
analytic physical 8x reference it gives 1.74105488 mm RMS and 43 exit changes.
V3 cannot undo geometry already changed during voxel export. The 0.005 mm /
zero-change analytic acceptance above applies to the GEM route, while the
mesh validation is matched to the imported triangle geometry.

```sh
target/release/simrep export geometry target/cuts-v3/quadrupole.pa# \
  target/cuts-v3/quadrupole.stl
RAYON_NUM_THREADS=16 target/release/simrep refine-geometry \
  target/cuts-v3/quadrupole.stl --grid 121,73,73 --spacing 0.5,0.5,0.5 \
  --origin=0,-18,-18 --electrode 0=1:2.5 --electrode 1=1:2.5 \
  --electrode 2=2:-2.5 --electrode 3=2:-2.5 --electrode 4=3:0 \
  --electrode 5=3:0 --surface-supersampling 1 --repair --tolerance 1e-12 \
  --out target/cuts-v3/mesh
RAYON_NUM_THREADS=16 target/release/examples/cuts_trace target/cuts-v3/mesh \
  --mesh target/cuts-v3/quadrupole.stl --boundary \
  --output target/cuts-v3/mesh-trace
RAYON_NUM_THREADS=16 target/release/simrep fly --device cpu \
  --pa target/cuts-v3/mesh.pa0 --cuts target/cuts-v3/mesh.cuts.json \
  --particles /mnt/c/Users/lsu10/simrep-oracle/_quad_gpu/particles.ion \
  --voltage 1=2.5,2=-2.5,3=0 --origin=0,-18,-18 \
  --bounds=0,-18,-18,60,18,18 --tolerance 1e-11 --max-step-us 0.025 \
  --output target/cuts-v3/mesh.csv
python3 "$Q/compare_quadrupole.py" target/cuts-v3/mesh-trace/source.csv \
  target/cuts-v3/mesh.csv
```

STEP/STP/IGES/IGS CLI feature tests additionally verify both exact-refinement
and tessellated-refinement modes write v3 mesh payloads at the chosen 0.002 mm
deflection, then query their membership and segment hits. A located two-body
STEP assembly still fast-adjusts and flies through the detector.

#### V3 compute overhead and verification

Five interleaved serial repetitions, with 16 Rayon threads and no concurrent
benchmark/build, give the following medians. The PA, particles, voltages,
integration tolerances and bounds are identical within each pair. `--no-cuts`
uses the current default all-corner staircase convention; the older einzel
harness comparison above explicitly uses nearest-node voxels instead.

| Case | Ions | V3 flight s | No-cuts flight s | Compute ratio | V3 setup s | No-cuts setup s |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Quadrupole | 1,000 | 0.130802 | 0.097657 | 1.339x | 0.636995 | 0.208365 |
| Einzel plus neutral impacts | 1,036 | 0.156970 | 0.103468 | 1.517x | 0.012957 | 0.012807 |

The quadrupole meets the approximately 1.5x compute target; einzel is close
but slightly above 1.5x. Loading/validating the retained full-domain fallback
is setup, not flight compute. Median total times, including output but not
process startup, are 0.788538/0.329190 s (quadrupole cuts/no-cuts) and
0.170630/0.117345 s (einzel). These are not end-to-end speedup claims.
GEM evaluation skips fills preceding the first fill of an electrode because
they cannot affect its initially empty conductor, without changing later
overwrite order. The timed quadrupole retains the same 0.00343534 mm RMS and
zero exit changes.

V3 verification exited zero for `cargo fmt --all --check`,
`cargo clippy --workspace --all-targets -- -D warnings`,
`cargo clippy -p simrep-py --all-targets -- -D warnings`, and
`RAYON_NUM_THREADS=16 cargo test --workspace --release`: 188 standard tests
passed, plus 16 Vulkan multigrid comparisons and f32/f64 smoke kernels.
DX12 had no WSL adapter. CAD-enabled Clippy and the two CLI feature tests also
exited zero; the installed OCCT headers emitted their existing C++
`std::iterator` deprecation warning. Rust LSP timed out after 30 seconds;
compiler and strict Clippy checks provide the code diagnostics.

#### Exact-surface flight overhead after far-pair rejection (2026-10-25)

`SurfaceModel::segment_hit` now rejects electrode pairs conservatively
before invoking the shared surface-fraction tracer (`distance/lipschitz >
length`), so exact-surface segments far from every conductor skip the
tracer's double start-point evaluation. Seven interleaved serial
repetitions per leg, 16 Rayon threads, release build d1010c4 on the Ryzen
9 5950X (shared-machine 1-minute load average 5.7-8.8 during the run; a
sibling worktree build finished before the first round). `--no-cuts` uses
the default all-corner staircase convention; the einzel+36-neutral case
was also timed with the harness's `--nearest-node-collisions` convention.

| Case | Ions | Cuts fly s | No-cuts fly s | Compute ratio |
| --- | ---: | ---: | ---: | ---: |
| Quadrupole user route | 1,000 | 0.047991 | 0.049114 | 0.977x |
| Einzel plus neutral impacts | 1,036 | 0.052973 | 0.050733 | 1.044x |
| Einzel plus neutral impacts (nearest-node) | 1,036 | 0.052973 | 0.051480 | 1.029x |

Reported compute medians from the 2026-10-24 implementation run were
0.051105/0.048107 s (1.062x) for the einzel case and 0.046271/0.042488 s
(1.089x) for the quadrupole; this re-measurement confirms the sub-1.1x
overhead and shows the quadrupole at parity within noise. All seven
endpoint CSVs per leg are byte-identical, and the timed quadrupole retains
**0.003435341053206124 mm** endpoint RMS, **0.02018961 mm** maximum, and
0/1000 exit-class changes against the physical 8x reference. Per-run
`fly_s` values (quadrupole cuts): 0.043116, 0.043815, 0.047991, 0.051137,
0.051091, 0.048411, 0.044655; (quadrupole no-cuts): 0.041722, 0.060426,
0.045103, 0.049114, 0.060458, 0.045300, 0.050492; (einzel cuts): 0.050299,
0.052973, 0.052215, 0.054403, 0.056431, 0.054079, 0.051893; (einzel
no-cuts): 0.045957, 0.049077, 0.053557, 0.052965, 0.053662, 0.050733,
0.049169; (einzel nearest-node): 0.049990, 0.051480, 0.053177, 0.054925,
0.053611, 0.050896, 0.050233.

Exact commands, run from the repository root after the documented
`refine-geometry` preparation (einzel uses the installed
`einzel.gem` at 92x21x1 with spacing 90/91,0.95,1 mm; quadrupole uses
`$Q/quadrupole.gem`):

```sh
Q=/mnt/c/Users/lsu10/simrep-oracle/_quad_cut
# per leg: --cuts <stem>.cuts.json or --no-cuts (add --nearest-node-collisions
# for the harness convention on the einzel pair)
RAYON_NUM_THREADS=16 target/release/simrep fly --device cpu \
  --pa target/qa-fly/quadrupole.pa0 --cuts target/qa-fly/quadrupole.cuts.json \
  --particles /mnt/c/Users/lsu10/simrep-oracle/_quad_gpu/particles.ion \
  --voltage 1=2.5,2=-2.5,3=0 --bounds=0,-18,-18,60,18,18 \
  --tolerance 1e-11 --max-step-us 0.025 --output target/qa-fly/qcuts-1.csv
RAYON_NUM_THREADS=16 target/release/simrep fly --device cpu \
  --pa target/qa-fly/surface-nonaligned.pa0 \
  --cuts target/qa-fly/surface-nonaligned.cuts.json \
  --particles target/qa-fly/surface-einzel-impact.ion \
  --voltage 1=0,2=110,3=0 --bounds=-101,-19,-19,291,19,19 \
  --tolerance 1e-11 --max-step-us 0.05 --output target/qa-fly/ecuts-1.csv
python3 "$Q/compare_quadrupole.py" "$Q/physical-8-global.csv" \
  target/qa-fly/qcuts-1.csv
```

`fly_s` on stderr is the compute clock (setup, output and process startup
excluded). The einzel ion file is the 1,000-ion subset plus 36
deterministic neutral radial impacts, matching the `surface-einzel`
harness recipe.

## Reproduction and timing

```sh
cargo build --release -p simrep-cli --bin simrep --examples
RAYON_NUM_THREADS=16 target/release/examples/geometry_pipeline mesh
RAYON_NUM_THREADS=16 target/release/examples/geometry_pipeline einzel
```

`mesh` generates and imports fine closed sphere/cylinder STL surfaces and
compares the cut-cell and physical-mask staircase operators with the analytic
concentric solutions. The outer STL defines the cavity surface, so the harness
uses its complemented SDF as the grounded exterior conductor. It does not
claim that a disconnected STL shell preserves a named cavity part on import.
The command integration tests separately exercise file-to-PA0 mesh refinement.

`einzel` reads the installed `einzel.gem` without modifying it, invokes the
actual `refine-geometry`, `refine`, and `fly` commands, compares potentials
with the existing independent 4x/8x fields, and compares the 1000-ion subset's
endpoints with the 8x results. The non-aligned 92x21 grid has spacing
`90/91,0.95,1` mm, preserving the exact physical `[0,90]x[0,19]` domain.
The aligned 91x20 grid is also reported; its surfaces already lie on nodes.
`SIMREP_BIN` and `SIMREP_REFERENCE` override the executable/reference directory.
Defaults for the latter and the subset are the locations used in
`einzel_convergence.rs` and `bench/fly-real.md`.

The example writes JSON and PA/CSV artifacts under `target/geometry-pipeline`.
`compute_ms` sums solver compute phases only. Source loading, repair/BVH,
SDF sampling, cut preparation, multigrid construction, GPU initialization,
upload, readback, and serialization are excluded. `preparation_ms` is a
diagnostic, not a speed denominator; end-to-end time is also printed.
For GPU staircase comparison, sum the individual `GPU ... solve` fields
printed by `simrep refine`, not its aggregate `solve` field, which historically
also includes transfer time.

## Measured acceptance, October 23, 2026

Ryzen 9 5950X, Linux release f64, 16 Rayon threads. These are single serial
runs, not timing medians. The analytic mesh harness uses a 65³ sphere grid
and a 65² cylinder cross-section. Each sphere STL has 36,480 triangles;
each closed cylinder STL has 2,048 triangles. Inner/outer radii are
0.31/0.83 mm at 1/0 V.

| Imported surface | Cut max error V | Staircase max error V | Error reduction | Cut compute ms | Staircase compute ms |
| --- | ---: | ---: | ---: | ---: | ---: |
| Sphere STL | 0.00479243 | 0.10487824 | 21.9x | 47.096 | 39.357 |
| Cylinder STL | 0.00081280 | 0.07394388 | 91.0x | 2.330 | 2.358 |

The following potentials include all non-electrode nodes, sampled from the
independent fine fields with bilinear interpolation at the candidate nodes.
Endpoint errors use all 1,000 original subset ions, the workbench bounds in
`bench/fly-real.md`, relativistic RK45, tolerance `1e-11`, maximum step
0.05 us, and the existing 8x endpoint CSV. Neither the reference files nor the
installed GEM were changed.

| Grid / method | Potential RMS vs 4x V | Potential RMS vs 8x V | Potential max vs 8x V | Endpoint RMS vs 8x mm | Endpoint max vs 8x mm |
| --- | ---: | ---: | ---: | ---: | ---: |
| Aligned cut | 0.19494511 | 0.21054188 | 3.35666458 | 0.22583775 | 1.32390857 |
| Aligned staircase | 0.19494511 | 0.21054188 | 3.35666457 | 0.22583772 | 1.32390832 |
| Non-aligned cut | 0.27134131 | 0.26478055 | 6.04325750 | 0.35875345 | 2.03126525 |
| Non-aligned staircase | 0.99792128 | 0.98327698 | 18.87538642 | 1.57718686 | 5.28167911 |

On the non-aligned grid, cut-cell potential RMS is **3.71x smaller** and
endpoint RMS is **4.40x smaller**. On the aligned grid the maximum cut/staircase
nodal difference is `1.08802e-8 V`: equal to root/solve precision, as expected.
The harness asserts aligned equality and non-aligned improvement.

| Device / case | Cut compute ms | `simrep refine` compute ms |
| --- | ---: | ---: |
| Linux CPU / aligned | 4.675 | 5.925 |
| Linux CPU / non-aligned | 6.487 | 7.179 |
| Windows RTX 3070 Vulkan / non-aligned | 151.463 | 115.543 |

All three electrode bases are included. Tolerance is `1e-12` in every row.
The tiny einzel grid is dispatch-bound on GPU; this is not a large-grid
throughput claim. GPU cut-cell overhead in this run is 1.31x. Native GPU
geometry construction and transfers are excluded from both compute columns.
The DX12 CPU-f64-residual fallback also converged all three cut bases
(190.541 ms compute). Both Windows smoke kernels passed f32, Vulkan additionally
passed native f64 with zero discrepancy on its exactly representable fixture,
and DX12 explicitly reported `SHADER_F64: false`.
The non-aligned PA0 agrees with the Linux CPU field to `1.41967e-11 V`
on Vulkan and `1.41789e-11 V` on DX12; physical electrode masks match exactly.
After the native runs, `geometry_pipeline gpu` checks these stored arrays.

Final Linux verification exited zero for `cargo fmt --all --check`,
`cargo clippy --workspace --all-targets -- -D warnings`, and
`RAYON_NUM_THREADS=16 cargo test --workspace -- --nocapture`.
All 120 standard tests passed, plus 16 standalone GPU multigrid comparisons
and two Vulkan smoke kernels on llvmpipe. WSL DX12 was explicitly unavailable;
the native Windows checks above used the hardware device instead.
The Rust LSP daemon did not respond within 30 seconds; compiler and strict
Clippy diagnostics were clean. No failing tests were suppressed or changed.

### Windows hygiene

Every native process ran in the foreground from `/mnt/c`, with closed stdin
and a timeout. The final Windows command was exactly:

```sh
s="$(wslpath -w tools/win/winproc.ps1)"
(cd /mnt/c && timeout 30 powershell.exe -NoProfile -NonInteractive \
  -ExecutionPolicy Bypass -File "$s" cleanup < /dev/null)
```

It completed successfully and printed:

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

No task-owned `cargo.exe`, `rustc.exe`, or `simrep.exe` remained.

## Tessellated CAD preparation, October 27, 2026

Ryzen 9 5950X, Linux release f64, `RAYON_NUM_THREADS=8`. The CLI default is
every core unless that variable is set. These comparisons use 8 threads
because that is the recorded baseline. Single runs, not medians.
`geometry_preparation_ms` is the phase log. `preparation_ms` includes solver
setup and excludes solver `compute_ms`.

Private case, recorded 8-thread comparison: an ESA-like toroidal analyzer
STEP, not stored in this repo, 2.89M-node 3D grid. Preparation 361.4 s to
19.7 s. Phase times, seconds: source 34.7 to 7.2, owners 28.6 to 1.3, cut
cells 136.4 to 5.7, links 160.4 to 4.2. Outputs were byte-identical.

Public fixture, measured here with load average about 13.4. `pair.step` is
the two-sphere assembly from `tools/cad-fixtures` (also
`simrep_cad::write_fixture` shape 3). Grid 101x101x101, spacing 0.02 mm,
origin -1 mm, tessellated, chordal tolerance 0.001 mm.

| Phase | Before ms | After ms | Speedup |
| --- | ---: | ---: | ---: |
| source | 30.538 | 22.901 | 1.33x |
| owners | 432.777 | 38.416 | 11.27x |
| cut cells | 2482.947 | 84.776 | 29.29x |
| physical links | 3702.519 | 29.129 | 127.11x |
| samples | 22.435 | 12.150 | 1.85x |
| sidecar write (I/O) | 4.376 | 2.204 |  |
| `preparation_ms` | 6822.993 | 326.619 | 20.89x |

`.pa#`, `.pa0`, `.pa1`, `.pa2`, and `cuts.json` were byte-identical. Cut-node
count was 5188 both.

The acceleration follows ideas in
[Iteration-CFD](https://github.com/using76/Iteration-CFD) (commit
`1475bbf781e08f33f681661ec40e00727f1e7021`, Prosperity Public License 3.0.0),
studied and not copied. No source file in this repo contains its code or
identifiers. The adopted ideas are: classify a surface band and flood the
interior instead of querying every node; reject a segment whose bounds cannot
meet a triangle before tracing it; take the inside/outside sign from an
outward face normal only for a single outward shell when the nearest point is
in a face interior, and keep the ray classifier for nested shells and edge
hits; tessellate independent faces on all cores. A uniform triangle bin was
tried and removed. On this mesh it was slower than the existing BVH for
nearest queries. The parallel face mesher reproduces the serial triangulation
on the sphere fixture, and the private-analyzer arrays above stayed
byte-identical with it enabled.
