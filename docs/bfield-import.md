# Importing magnetic field exports

`simrep fly --bfield-map field.csv --particles ions.ion` flies through an
external static vector B map. Repeated `--bfield-map` arguments add fields.
Imported maps superpose with `--bfield`, magnetic PAs and Biot-Savart coils;
the existing first-containing-PA overlap rule applies only to magnetic PAs.
The map does not create electric potential or material electrodes.

```bash
simrep fly --particles ions.ion --bfield-map comsol.txt \
  --bfield-map-format comsol --bfield-map-units m/T \
  --bfield-map-divergence --duration-us 20 --device cpu

simrep fly --particles ions.ion --bfield-map axis.txt \
  --bfield-map-format axisymmetric --bfield-map-units mm/G \
  --bfield-map-transform '10,20,30,1,0,0,0,1,0,0,0,1,2'
```

All internal values and public flight fields use mm and gauss. Supported
coordinate units are `mm`, `m`, `cm`, `inch` (`in`); flux units are `G`
(`gauss`), `T` (`tesla`), `mT`. `--bfield-map-units length/flux` overrides
recognized bracketed/parenthesized units or unit words in headers. Otherwise
headers are inspected, then **mm/G** is assumed and printed. Model, version,
date, file and project labels are ignored during unit detection. Mixed
coordinate or component units are not supported: re-export using one unit of each kind,
or explicitly normalize the columns first. The import diagnostic prints the
resolved units, dimensions, spacing and outside policy for each file.

## Accepted data

`--bfield-map-format` accepts `auto` (default), `comsol`, `opera`, `csv`,
`txt`, and `axisymmetric`.

* COMSOL Spreadsheet: `%` header lines followed by point rows
  `x y z Bx By Bz`. COMSOL Grid: `% Grid`, one coordinate-vector row per
  dimension, then three `% Data ...` blocks, in **Bx, By, Bz** order. Each
  block contains exactly the product of the coordinate-vector lengths,
  flattened with x fastest. A missing third coordinate vector means z=0.
  Export one static vector field, not multiple time or parameter frames.
* Opera-3d `.table`: an initial header block (dimensions and numbered column
  descriptions) terminated by a line containing `0`, followed by point rows.
  `auto` recognizes the `.table` extension; pass `opera` for other extensions.
* CST, Ansys, FEMM and other delimited text exports: a header is optional;
  default columns are `x,y,z,Bx,By,Bz`. Commas, semicolons, tabs and whitespace
  are accepted. Use `--bfield-map-delimiter ';'` to select a delimiter.
  `--bfield-map-columns 2,1,0,5,4,3` sets **zero-based** source indices in that
  same semantic order, allowing unrelated columns to be ignored.
* Axisymmetric text: explicitly select `axisymmetric`. Rows are
  `z,r,Bz,Br`, or configure four source column indices. The symmetry axis is
  source Cartesian **Z**, with radial directions in source XY. All azimuths
  are included; no azimuthal field is represented. Radius must start at zero,
  with at least two radii and Br=0 on the axis. There must be no missing nodes.

Blank lines and `%`, `#`, `!`, `//` comments are ignored. Scientific notation
including Fortran `D` exponents is accepted. Nonnumeric headers precede data;
malformed numeric data after the first point is an error.

Rows can arrive in any order. Coordinates must cover a **complete, uniformly
spaced tensor grid**; exact repeated coordinates identify axis nodes. Export
enough coordinate digits to preserve these equalities and regular spacing.
There is no implicit interpolation of genuinely scattered or nonuniform
input. Such point clouds, duplicate nodes, missing nodes and nonfinite data
produce a clear error. Re-export on a regular grid in the originating solver.
This deliberate requirement avoids an unreported RBF/natural-neighbour
resampling error. Single-point Cartesian axes are translationally invariant,
not zero-thickness slabs.

The originating solver's gridded export is the required resampling method;
its solver/interpolation error is additional to the re-import errors below.

## Placement, reflection and edges

`--bfield-map-transform` takes 13 comma-separated values:

```text
ox,oy,oz, axx,axy,axz, ayx,ayy,ayz, azx,azy,azz, scale
```

The origin is a workbench translation in mm. The next nine numbers are
**columns** of a proper orthonormal rotation; scale is positive. Source mm
coordinates become `origin + rotation * (scale * source_position)`.
The map's nonzero minimum coordinates are retained. The vector B is rotated
but **not scaled in magnitude**. These settings are independent of electric
PA placement. Every map in one invocation shares the selected import options.

`--bfield-map-mirror x,y,z` reflects Cartesian magnetic **axial vectors**
about the selected source grid-minimum planes: the component normal to a
plane is even; the other two are odd. Odd components must vanish on the
plane. An axisymmetric map supports only `--bfield-map-mirror z`: Bz is even
and Br odd about its minimum axial coordinate. Reflection parities are
imposed during C2 reconstruction, not applied to a nonsymmetric spline
after reconstruction. The Rust API also permits explicit per-component
reflection parity.

`--bfield-map-outside` selects:

* **error** (default): leaving any imported map stops an ion at that map's
  physical boundary with `OutsideGrid`. Starting outside stops immediately.
  The CLI diagnostic identifies the imported map and this termination policy.
  Integrator trial stages may use endpoint-polynomial extrapolation, as for
  PA grids; accepted trajectories are clipped to the boundary.
* **zero**: the map contributes zero outside its domain. Its edge may be
  discontinuous, so put export boundaries in field-free space or resolve the
  fringe with a fine grid and appropriate explicit flight step ceiling.
* **clamp**: use the nearest coordinate on the grid box (or axial/radial
  limits for axisymmetric data). This is constant extension along outward
  normals, not a physical fringe-field model.

For several default-error maps, the valid flight volume is their domain
intersection. Use explicit zero extension for separate field regions.

## Reconstruction and diagnostics

Each Cartesian B component uses the same compact, shared nodal-derivative
tensor quintic Hermite C2 reconstruction as the calibrated coil maps.
It interpolates the exported vectors, not a scalar magnetic potential.
An axisymmetric map reconstructs Bz and the smooth **Br/r** scalar; the axis
value of Br/r is a one-sided even-polynomial extrapolation from up to three
positive radii. The reconstructed Br/r multiplies the physical transverse
coordinate, giving exactly zero transverse field at r=0. The C2 polynomial
does not repair inconsistent source data or enforce div B=0.

`--bfield-map-divergence` reports nodal maximum/RMS finite-difference div B
in G/mm and `max|div B| * min_spacing / max|B|`. The cylindrical diagnostic
uses `dBz/dz + dBr/dr + Br/r`, including the finite axis limit. Differences
are centered in the interior and one-sided at the outermost nodes. A
normalized maximum above 0.01 prints a warning to inspect units, noise and
resolution. This threshold is a diagnostic heuristic, not an accuracy gate
or a proof that lower values are physically correct.

Both CPU and GPU use the identical nodal jets, without GPU resampling.
Native f64 `Precise` and f32 `Preview` are supported. As for other magnetic
flights, **`--device auto` remains CPU**: imported maps have no measured
GPU crossover. Explicit `--device gpu --backend vulkan` enables the existing
GPU path; Preview remains a Rust GPU API selection, not a promise of f64
accuracy. Map preparation/upload is separate from flight compute.

## Validation and reproduction

`cargo test -p simrep-io bfield` checks manufactured COMSOL Grid/Spreadsheet,
Opera header blocks, generic tables, configurable columns/units, Fortran
numbers, axisymmetric input and rejection of malformed/scattered input.
`cargo test -p simrep-fly --release --test bfield_import -- --nocapture`
exports/reimports exact uniform, single-loop, 16-hoop solenoid and
source-free analytic 2D sector fields at 6, 11 and 21 nodes per axis.
Every case compares full-flight endpoint and detector TOF with the exact
field. The `BFIELD_CONVERGENCE` records report spacing, endpoint/TOF errors,
observed endpoint and TOF convergence orders, and measured preparation/compute
and exact-field flight time. `Auto` in these records denotes COMSOL Grid;
`Comsol` denotes Spreadsheet. Axisymmetric controls separately export and fly
the uniform, loop and solenoid fields at the same three resolutions.
These are manufactured-format fixtures, not a certification of every export
variant or a comparison with the originating programs' solver accuracy.
`bench/bfield-import.md` records the measured errors, observed orders and
explicitly contended timing samples.

The same test file checks axis limits, coordinate rotation/translation/scale,
mirror parity, superposition, divergence and all outside policies.
`gpu_imported_magnetic_maps_precise_and_preview` compares CPU and GPU
flights for 3D and axisymmetric maps, transformed coordinates, multiple maps
plus coils/uniform B and all outside policies, in both precisions. Hardware
checks follow `docs/dev/windows-gpu.md`; Linux software Vulkan does not count
as Windows hardware validation.

## Rust and proposed Python API

`simrep_io::bfield::{read,parse,ReadOptions,MagneticGrid}` returns normalized
nodal vectors and grid metadata. Build
`simrep_fly::physics::magnetic::ImportedMagneticMap::from_nodes` using
`MapPlacement`, and push maps onto `MagneticField::maps`. Axisymmetric Rust
maps use local X=axial and Y=radius like existing SIMION grids; callers rotate
that frame to source Z if desired. `divergence_report()` returns the optional
diagnostic. All structures own their data; prepared coil-map wrappers borrow
the combined field, preserving its imported maps.

No Python binding is changed here. The additive binding proposed for the
parity worker is:

```python
MagneticMap.read(path, format="auto", units=None, columns=None,
                 delimiter=None, origin_mm=(0, 0, 0),
                 axes=((1, 0, 0), (0, 1, 0), (0, 0, 1)), scale=1,
                 outside="error", mirrors=())
MagneticMap.from_nodes(values_gauss, spacing_mm, origin_mm=(0, 0, 0),
                      axisymmetric=False, axes=..., scale=1,
                      outside="error", mirrors=())
map.divergence_report()
fly(..., magnetic_maps=[map])
```

Bindings should retain the same finite grid/placement validation, units,
outside behavior and CPU/GPU superposition, and include maps when deciding
whether a flight is magnetic. CLI format names and Python source-Z
axisymmetry should agree; the low-level Rust axial-X convention remains
explicit.
