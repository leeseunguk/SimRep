# Mesh import and scientific export

`simrep-mesh` is a native Rust library, included by the workspace's `crates/*`
membership. `simrep mesh-import` and `simrep export` are additive commands.
STEP/IGES, animation, skinning, materials/textures, and CAD healing are outside
this crate's geometry-only scope.

## Coordinates and parts

All library vertices and grid coordinates are **millimeters**. Samples are
nodes, not cell centers, with x varying fastest:
`index = x + nx * (y + ny * z)`.

| Import | Grouping | Default source length |
| --- | --- | --- |
| STL, ASCII or binary | Edge-connected components, in first-face order | mm |
| OBJ | Named objects/groups from `tobj`, triangulated | mm |
| PLY 1.0, ASCII / binary little or big endian | Face `object_id` or `part_id`, otherwise one part | mm |
| 3MF core | Build instances and component/object paths | Model `unit`, mm when omitted |
| glTF 2.0 / GLB | Scene node/object, combining material primitives | m |

`--units mm|cm|m|inch` overrides the source unit. The library also recognizes
microns and feet. Unit conversion follows baked source scene transforms;
the user affine transform is applied last, with translation in mm.
Negative-determinant transforms reverse triangle winding.

glTF supports indexed/nonindexed triangles, strips, fans, external buffers,
base64 buffer data URIs, and GLB buffers. It imports the default scene (or
first scene), using static node transforms. It does not fetch network assets
or decode compressed geometry extensions. 3MF follows the root package
relationship, core meshes, nested components, build transforms and printable
flags. Unsupported required extensions and external production-extension
components are errors. OBJ material changes do not split an object.
OBJ and PLY planar polygons are projected and ear-cut, including concave
polygons. Vertex colors and face materials do
not affect electrode numbers.

## Import a mesh as electrodes

```sh
cargo run --release -p simrep-cli -- mesh-import lens.stl 'lens.pa#' \
  --grid 161,121,121 --spacing 0.25,0.25,0.25 --origin -20,-15,-15 \
  --units mm --electrodes 1,2 --repair
```

Without `--electrodes`, parts receive numbers 1, 2, ... in printed import
order. Numbers must be in 1..=100000, matching the default PA electrode
encoding range. The output contains electrode **numbers**, not voltages.
Overlap resolution is deterministic: later parts win; counts and overlaps
are printed. A PA cannot store a grid origin, so retain `--origin` in the
project and supply it again on export. Spacing is stored in the extended
PA header when it is not 1 mm.

`--transform` accepts 12 comma-separated values forming a row-major 3x4
matrix. Its default is `1,0,0,0,0,1,0,0,0,0,1,0`.
Grid samples on a solid's boundary count as inside.
Only planar 3D voxelization is provided; cylindrical revolution is not
silently approximated.

The checker reports coincident vertices, zero-area and duplicate triangles,
open edges, simple boundary loops (holes), non-manifold edges, inconsistent
winding, inward closed shells, and orientation conflicts. `--repair` welds
exact coincident vertices, removes degenerate/duplicate triangles, makes
orientable components consistent, and turns closed shells outward. It does
not fill holes, weld near-coincident surfaces, split non-manifold junctions,
or guess a missing surface. Parts that are not closed manifolds cannot be
sampled as solids by default. For a known defective source, `--allow-defects`
explicitly enables best-effort parity sampling without changing the surface;
the issue report remains visible and the result must be compared with the
intended geometry. This option cannot generate SDFs. The library equivalent
is `voxelize_with_defects` / `Solid::from_surface`. Self-intersections are not
detected by the edge-topology checker and must be resolved in the source model.

Solid sampling uses a median-split BVH. Voxelization traces one x ray per
grid row and handles shared-edge/vertex ambiguities with non-axis-aligned
point rays, then a generalized winding-number fallback. Signed distances
use BVH nearest-triangle queries and the same classifier. Membership uses
the even/odd rule, so nested closed shells form cavities. Rayon parallelizes
rows, pointwise SDF samples, and large BVH construction branches.

## Sampled SDF contract

```sh
simrep mesh-import lens.stl 'lens.pa#' --grid 81,81,81 \
  --spacing 0.5,0.5,0.5 --origin -20,-20,-20 --repair \
  --sdf-prefix lens-sdf --electrodes 3,7 --voltages 0,120
```

For each part this also writes `lens-sdf-0.vti` and `lens-sdf-0.json`, etc.
VTI contains a `signed_distance_mm` scalar field. JSON contains the full
plain sampled values, `dims`, `origin`, `spacing`, `electrode_number`, and
`voltage`; it can be large. The library `sample_sdf` returns the same layout:
finite `Vec<f64>`, mm, **negative inside the conductor**, x-fastest order.
`SignedDistanceGrid::interpolate` is trilinear within the closed sample
domain and returns `None` outside it. No extrapolation is performed. The
values are exact distances to the triangulated surface, not to the original
CAD surface; interpolation and tessellation contribute separate errors.
Trilinear interpolation of sampled distances is not necessarily an exact
distance function (a conservative gradient bound is sqrt(3)).

## Export surfaces and fields

```sh
simrep export geometry 'lens.pa#' lens.stl
simrep export geometry 'lens.pa#' lens.obj --electrode 7
simrep export geometry 'lens.pa#' lens.glb --origin -20,-20,-20
simrep export grid solved.pa solved.vti
simrep export isosurface solved.pa equipotential.gltf --potential 50
simrep export geometry einzel.pa0 einzel.glb --angular-segments 96
simrep export geometry einzel.pa0 cutaway.obj --sweep-degrees 270
simrep export geometry planar.pa planar.stl --extrusion-depth 5
simrep export grid einzel.pa0 meridional.vti
simrep export grid einzel.pa0 revolved.vts --revolve --angular-segments 64
simrep export isosurface einzel.pa0 equipotential.glb --potential 50
```

Geometry extraction emits outward exposed voxel faces, each voxel extending
half a spacing around its PA node. Internal faces between equal-valued
electrode voxels disappear; interfaces between different values remain.
Parts are grouped by electrode potential/number. In a solved PA, different
electrodes at the same potential cannot be distinguished by the base array.
Reflection symmetry is expanded to the full domain, without duplicating
nodes on the mirror plane. An anti-mirror changes scalar potential sign.
Mirror planes pass through the original first node, translated by `--origin`.

Cylindrical 2D arrays use x as the axial coordinate and y as radius.
Geometry extraction revolves exposed x-r voxel edges about the x axis;
the y mirror represents radial symmetry, not a duplicated negative-radius
plane. Axis voxels are clipped at r=0 and angular seam/axis vertices are
shared. `--angular-segments` defaults to 64 intervals and
`--sweep-degrees` defaults to 360. Partial sweeps, such as 270 degrees,
include triangulated end caps, including annular holes. Each electrode
retains its own boundary and caps. Planar 2D geometry is extruded along z
with closed end caps and `--extrusion-depth` (default 1 mm), centered on
the origin z coordinate. Geometry retains the voxel staircase; it does
not infer subcell electrode shapes.

STL export is binary Float32 and loses names; OBJ keeps named groups.
glTF uses embedded buffers, and GLB embeds its binary chunk. Both convert mm
to meters per the glTF convention. The potential isosurface uses classic
marching cubes with linear edge interpolation and no artificial domain
caps. As with classic marching cubes, saddle cells can have ambiguous
topology. Potential differences are formed in f64 before normalized f32
marching-cubes interpolation, avoiding loss of a large DC offset.
For 2D PAs, marching squares with f64 linear edge interpolation extracts
the iso-contour, which is revolved or extruded with the same surface
options. Saddle connectivity is chosen by the bilinear cell-center value.
As in 3D, isopotentials have no artificial domain or sweep/extrusion caps;
an open contour or a planar extrusion can therefore produce an open mesh.
The additive library entry points are `electrode_surface_with_options`
and `pa_isosurface`, using `SurfaceOptions`; `electrode_surface` uses
the defaults and `isosurface` remains the Cartesian 3D grid API.

VTI uses appended, uncompressed little-endian binary blocks with UInt64
length headers. ParaView can read the Float64 `potential`, UInt8 `electrode`
mask and Float64 vector `field`. For an electric PA, `field = -grad(V)` is
in V/mm. Boundary differences are one-sided; interior differences centered.
`--no-field` omits that array. This is a visualization derivative, not the
flight solver's interpolation. Magnetic arrays export the negative scalar
potential gradient without SIMION ng/workbench scaling. Planar 2D arrays
write a single z plane. Cylindrical VTI is explicitly an x-r coordinate
chart, also a single plane: its vector is `(E_x,E_r,0)`, with `E_r=0`
on the axis. Radius is the chart's y coordinate minus origin y. This is
not a sampled Cartesian volume.

`export grid --revolve` writes cylindrical data as `.vts` StructuredGrid
instead. Coordinates and vectors are Cartesian, with
`y=origin_y+r*cos(theta)`, `z=origin_z+r*sin(theta)` and
`(E_x,E_y,E_z)=(E_x,E_r*cos(theta),E_r*sin(theta))`. Storage order is x,
radius, then angle. The grid contains `angular_segments+1` angular planes;
the last plane repeats the first exactly for a full sweep. Axis points
coincide, as required by this cylindrical structured-grid parameterization.
`--sweep-degrees` also supports a partial VTS volume. Potential and
electrode masks are repeated at each angle; `--no-field` works in both
formats. The library equivalent is `write_revolved_vts`.

## Trajectories

```sh
simrep export trajectories samples.csv trajectories.vtp
simrep export trajectories samples.csv normalized.csv
```

The numeric sample CSV schema is:

```csv
trajectory_id,time_us,x_mm,y_mm,z_mm,vx_mm_per_us,vy_mm_per_us,vz_mm_per_us
1,0,0,0,0,1,0,0
1,1,1,0,0,1,0,0
```

Each id needs at least two finite samples with nondecreasing time. VTP emits
one polyline per trajectory and retains its id, times, and velocities. This
expects full sample histories, not the current `simrep fly` termination-only
CSV. The library export functions accept trajectory vectors directly and do
not depend on `simrep-fly`.

## Dependency decisions

All production dependencies below are Rust implementations; no native CAD
or C/C++ triangulation library is linked. Versions are resolved in Cargo.lock.

| Dependency | Why it is present |
| --- | --- |
| `simrep-core` | Existing PA and symmetry contract; avoids a second grid type for PAs |
| `stl_io` 0.11 | Established ASCII/binary STL decoding and binary writing; avoids ambiguous `solid` header detection |
| `tobj` 4 | Established Wavefront object/group and index handling; f64 coordinates enabled; materials not loaded into geometry |
| `gltf` 1.4 | Khronos glTF document/accessor validation and scene transforms; image importing and its unnecessary codecs disabled |
| `zip` 9 | Actively developed ZIP reader for 3MF packaging; only Rust deflate support enabled |
| `quick-xml` 0.42 | Maintained streaming XML parser for 3MF core, without an XML DOM or native library |
| `serde_json` 1 | glTF JSON generation and CLI SDF interchange |
| `base64` 0.22 | Standard glTF embedded buffer encoding/decoding |
| `mcubes` 0.1.7 | Current pure-Rust classic marching-cubes implementation; avoids shipping a copied lookup table |
| `earcutr` 0.5 | Established Rust ear-cut triangulation for concave OBJ/PLY polygons; preserves face winding after stable plane projection |
| `rayon` 1.11 | Shared native data-parallel worker pool for BVH construction and sampling |

PLY is a bounded local reader for PLY 1.0 scalar/list records; it avoids
adding the older `ply-rs` parser and handles both binary byte orders.
`tempfile` is test-only for isolated round-trip fixtures, and `simrep-io`
is test-only for PA interoperability.

## Validation and performance

```sh
cargo test -p simrep-mesh
cargo run --release -p simrep-mesh --example mesh_bench
cargo run --release -p simrep-mesh --example simion_compare -- \
  '/mnt/c/Program Files/SIMION-2024/examples/geometry/two_cylinder-1.stl' \
  '/mnt/c/Users/lsu10/simrep-mesh-oracle/mesh-point.pa#' 0 10 10
```

Tests compare cube, sphere and torus membership and SDF to analytic formulas,
exercise repairs and round trips, check affine transforms and format grouping,
and decode VTK block headers, array ordering and field values.
The benchmark constructs a closed torus with exactly 1,000,000 triangles
and samples 256^3 nodes; its printed elapsed time includes topology checks,
BVH construction and voxelization, not mesh generation or file I/O.

On October 23, 2026, Ryzen 9 5950X / WSL2 / Rust 1.95.0 release build with
`RAYON_NUM_THREADS=16`: **2.877828 s**, including topology checks, BVH and
voxelization. There were 1,513,624 occupied nodes; sampled torus volume error
was 0.09114%. Mesh construction took 0.029 s, excluded from that time.
Other workspace checks were running concurrently during this final run.

### SIMION-2024 oracle, October 23, 2026

The installation contains `geometry/two_cylinder.stl`,
`two_cylinder-1.stl` and `two_cylinder-2.stl`. Each cylinder has four open
edges and four non-manifold edges around its angular seam. Safe repair
deliberately leaves those defects unchanged. Strict import rejects them;
the comparison example uses the explicit defective-surface sampling API and
prints the remaining issues and edge locations.

SIMION's `--nogui --help` and `--nogui gem2pa --help` both work headlessly.
`tools/simion-oracle/mesh.gem` imports the read-only shipped
`two_cylinder-1.stl`, applies `(0,10,10)` mm translation, and uses a planar
31x21x21 unit-spaced PA. `within_inside_or_on` makes the comparison use the
same node-membership rule. This matters: the older implicit `within` fill
with `surface='none'` expands geometry by about half a grid unit, producing
3,080 metal nodes rather than 2,640; that is a fill-rule difference, not a
mesh classification error.

The point-membership oracle has **13,671 nodes, 2,640 metal nodes, and zero
electrode-mask mismatches**, including points on the surface. Our sampling
took 0.004171 s for that small mesh. This compares voxelization, not a
fractional-surface refine or a flight trajectory.

One reproduction command (create the output directory first):

```sh
gem="$(wslpath -w tools/simion-oracle/mesh.gem)"
s="$(wslpath -w tools/win/winproc.ps1)"
(cd /mnt/c && timeout 30 '/mnt/c/Program Files/SIMION-2024/simion.exe' \
  --nogui --noprompt gem2pa "$gem" \
  'C:/Users/lsu10/simrep-mesh-oracle/mesh-point.pa#' < /dev/null)
(cd /mnt/c && timeout 30 powershell.exe -NoProfile -NonInteractive \
  -ExecutionPolicy Bypass -File "$s" cleanup < /dev/null)
```

All installation paths were read-only during validation. Generated oracle
PAs live outside the repository, under the user's Windows profile.
