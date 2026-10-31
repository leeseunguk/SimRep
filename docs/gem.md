# GEM geometry conversion

`simrep-gem` compiles SIMION geometry into an unrefined potential
array. The workspace's `crates/*` membership includes the new crate.

```sh
cargo run --release -p simrep-cli -- gem2pa lens.gem lens.pa#
cargo run --release -p simrep-cli -- gem2pa lens.gem
```

The optional output defaults to the input path with a `.pa#` extension.
Electrode potentials are stored exactly as specified; in a fast-adjustable
`.pa#` they are electrode numbers. No refinement or solver behavior changes.
The reported `compute_ms` measures only point classification and assignment:
file reading, parsing, allocation, and PA writing are excluded.

## Supported input

- Traditional case-insensitive commands and synonyms, comma or space argument
  separators, omitted positional arguments, `;` and `--` line comments, and
  `--[[...]]` comments.
- Positional `pa_define(...)`, named `pa_define{...}`, dimensions in points,
  `gx/gy/gz` grid units, `wx/wy/wz` lengths, and `5*mm` length dimensions.
  Planar/cylindrical symmetry, axis mirroring, electric/magnetic type,
  non-refinable arrays, `ng`, anisotropic `dx/dy/dz`, and `unit='gu'`.
- `electrode`/`e` and `non_electrode`/`n` blocks, including the traditional
  aliases. Later fills overwrite earlier fills, including non-electrode
  potentials.
- `fill`, `edge_fill`, `rotate_fill`, `rotate_edge_fill` and their `_volume`
  aliases. Edge fills retain grid points with a six-connected neighbor outside
  the volume; 2D arrays use four neighbors.
- `within`, `notin`, their `_inside` and `_inside_or_on` variants, and
  `intersect`. Tests within a within block intersect; within blocks union;
  exclusion blocks subtract. The 8.2 implicit fill/within shorthand preserves
  fill ordering, including a new fill after a notin followed by another shape.
- `box`/`box2d`, `box3d`, centered and corner boxes, `circle`/`ellipse`,
  `sphere`/`ellipsoid`, `cylinder`, `cylinder3d`, `polyline`, `points`/`points3d`,
  `hyperbola`, `parabola`, and `half_space`. Polyline directions at infinity
  use homogeneous edge equations; axis-aligned infinite cylinders have
  unbounded axial intervals rather than large finite replacement endpoints.
- Binary and ASCII `stl` imports, resolved relative to the containing GEM.
  The optional `(x,y,z)` selector retains the nearest connected component;
  triangles sharing any vertex are connected.
- `locate`/`project`/`project_it`/`transform`, uniform and per-axis scaling,
  `scale`, `rotate_x/y/z`, `revolve_xy/yz/zx`, and `extrude_xy/yz/zx`.
  Locate applies rotation about X, elevation about Z, azimuth about Y,
  scaling, then translation. Nested inverse transforms classify grid points.
- Relative `include`/`include_file`, with optional `.gem` suffix, relative to
  the including file. Recursion and include nesting beyond 15 levels are errors.
- Lua 5.1 preprocessing: `#` lines execute Lua statements and `$(...)`
  executes expressions or statements. Expressions emit their results;
  `_put` emits GEM text. Functions may emit geometry from within loops and
  retain their lexical variables. Strings, tables, varargs, conditionals,
  math and string methods are evaluated by Lua rather than approximated.
- Modern 8.2 Lua functions, loops and geometry scopes, including the
  parameter-generating loop used by `polygon_regular.gem`, recursive table
  geometry, `shape(function(x,y,z)...)`, and `e`/`n` voltage callbacks.
  Shape callbacks receive local shape coordinates. Voltage callbacks are
  transformed only by scopes outside their electrode declaration.
- Empty `#` lines and `# -- comment` lines are accepted. Dimensions tolerate
  floating-point roundoff around integers. An `ng` below 1 retains the
  default 100; electric arrays ignore `ng`.

Traditional `within` expands boundaries by half a grid unit; `_inside`
excludes the boundary and `_inside_or_on` includes it without expansion.
`surface='auto'` uses signed 0.35-grid-unit adjustments for explicit inclusion
versus removal. It is the default for the modern table definition;
traditional definitions default to `surface=none`. Bare 8.2 shapes inherit
their enclosing test, while explicit `within`/`intersect` groups establish
their own inclusion mode. This distinction matters for nested cuts and bare
non-electrode fills. Boundary modes retain SIMION's small inside/on tolerance.
Polygon edges and vertex joins use the installed compiler's offsets rather
than a generic Euclidean dilation. Partial revolutions exclude their axis.
Rotated anisotropic cell sizes are measured along transformed local axes.

The signed-axis `half_space` shorthand follows the installed implementation:
`half_space('-x', a)` fills decreasing x, while the six-number form fills
opposite its supplied surface normal. The tetrahedron oracle covers this
distinction.

`surface='fractional'` produces the PA and, when offsets exist, its `.pa-surf`
companion. Empty companions are omitted and stale companions are removed.
The companion uses SIMION's sparse `SIMEXP`/`FHDR` version 1 format with
`XOFF`, `YOFF`, and `ZOFF` edge intersections. The CLI and conversion example
write it automatically. Rust callers writing the PA directly must also call
`result.write_surfaces(output_path)`. Fractional records are validated
independently of grid flags and potentials; a matching staircase alone is
not treated as a fractional-surface pass.

## Lua version and sandbox

The embedded interpreter is vendored PUC Lua 5.1.5 through `mlua` (`lua51`,
`vendored`, `send`), matching the version specified by SIMION's local
`lua.html` reference. It does not launch `lua.exe` or SIMION. Legacy macro
programs produce expanded GEM text for the existing parser; modern Lua
records the same ordered geometry nodes. Each compilation has its own Lua
environment, a 256 MiB Lua allocation limit and a 5 billion instruction
budget shared by preprocessing and callback evaluation.

The base, math, string and table libraries are available, with GEM math
aliases, `inf`, `mm`, and rectangular/polar coordinate helpers. There is no
`os`, `package`, `debug`, module loading, `dofile`, `loadfile`, `loadstring`,
or process execution. `include` and STL remain explicit geometry dependency
reads. Parameter-file writes used by `fe_geom.gem` and `sector_3dp.gem`
run against compilation-local virtual `io.open(...,'w'/'wb')` handles
supporting `:write` and `:close`; they do not create host files. Other IO
modes are rejected. `print` writes diagnostic output. Lua callback errors
are returned as conversion errors, not treated as an empty shape.

## Explicit limitations

The compiler rejects unsupported input rather than dropping it:

- Host Lua module imports, filesystem IO and process APIs.
- Signed-distance export and binary CSG serialization of Lua shape or
  voltage callbacks. Rasterization, including fractional surfaces, supports
  these callbacks; a Boolean predicate does not define an analytic SDF.
- Multiple `pa_define` statements and workbench `.wgem` descriptions.
- Include arguments/varargs and infinite `cylinder3d` endpoints that do not
  define an axis.

Unknown commands and named PA parameters identify the unsupported feature in
the error. Include failures include the source filename. An include fragment
without a `pa_define` is not a standalone array: the Rust error has
`is_include_fragment() == true`, and the conversion runner reports an
`include fragment:` status with exit code 0 and emits no PA. Validation counts
these files separately from unsupported sources.

`filename`, `nz_use`, and `refine_convergence` PA parameters are accepted;
they describe workbench/refinement metadata, not the binary PA grid.
The CLI's explicit output path takes precedence over GEM filename metadata.

## Rust API and analytic geometry

```rust
let gem = simrep_gem::read_gem("lens.gem")?;
let result = gem.rasterize()?;
simrep_io::write_pa("lens.pa#", &result.pa)?;
result.write_surfaces("lens.pa#")?;
# Ok::<(), Box<dyn std::error::Error>>(())
```

`parse_gem` also accepts an in-memory string. `Gem::fills` retains ordered,
public `Fill`, `Shape`, `Primitive`, `Region`, and `Transform` descriptions,
including electrode values and exclusions. No solver crate depends on it.

`Gem::sample_electrode_sdfs(dims, origin, spacing)` exports one grid per
electrode potential/number. Each `SampledElectrodeSdf` has exactly
`voltage: f64`, `dims: [usize;3]`, `origin: [f64;3]`, `spacing: [f64;3]`,
and `values: Vec<f64>`. Coordinates, spacing and distances are in millimeters;
negative values are inside conductor material. Values are finite and x-fastest
(`x + nx*(y + ny*z)`); `sample(point)` uses trilinear interpolation.
The physical CSG surface is sampled, without half-grid raster expansion.
Later electrode fills and non-electrode cuts retain their overwrite order.
Affine transforms and ellipsoids use conservative 1-Lipschitz distance bounds.
Hyperbola/parabola SDF sampling is explicitly unsupported; their GEM
rasterization is independent and remains available. Edge fills describe
zero-thickness analytic surfaces. This data layout matches the separate
`simrep-geom` cut-cell import contract without adding a dependency on it.

## Oracle validation

Sources are the locally extracted SIMION supplemental help
`gem_geometry_file.html` (8.1 reference and 8.2 additions), indexed by
`docs/spec/INDEX.md`. The SIMION executable is used only as a black-box
`--nogui --noprompt gem2pa` oracle.

```sh
cargo build --release -p simrep-gem --example convert
python3 crates/simrep-gem/tools/check_gem.py
# After changing only native conversion, reuse the recorded oracle outputs:
python3 crates/simrep-gem/tools/check_gem.py --skip-oracle
```

The harness copies GEM files and their text/Lua/STL dependencies to
`%USERPROFILE%\simrep-oracle\_gem\examples`, leaving the install read-only.
Every SIMION invocation is foreground, has a per-call timeout, uses a
Windows-drive working directory, and receives closed stdin. A timed-out
Windows process is tree-killed. Raw results are in `_gem/results.json`.
`docs/gem-validation.md` contains one row per installed GEM, exact mismatch
counts, and sample coordinates with native and SIMION point values.

Byte equality is reported when possible; otherwise the comparison decodes
all points, electrode flags, header metadata, and any trailer independently.
The extended harness also decodes every fractional sidecar and compares its
indexed axis offsets. Unsupported source constructs and SIMION input failures
remain visible in the table. The original 25 mismatch counts are retained
in a before/after regression ledger.

The installed corpus has 261 GEM files. The validation report records exact
matches, boundary mismatches and explicit unsupported features separately;
passing five geometry examples does not imply full-language equivalence.
The 2026-10-24 run has 254 standalone matches and 7 include fragments, with
zero unsupported sources and zero grid or fractional-record mismatches.
Of those matches, 252 use unmodified SIMION inputs. Two shipped inputs
(`einzel/einzel_param.gem` and `geometry_optimization/mesh_size.gem`) fail in
SIMION 2024 itself; their oracle working copies use equivalent table-form
PA definitions. The report preserves the original errors and labels these
two normalized-oracle comparisons. Native conversion uses unmodified input.
The original 70 unsupported inputs included 43 executable Lua programs,
7 include-only fragments without a standalone grid, and 20 Lua-free failures.
SIMION's `gem2pa` emits no PA for those seven fragments either, despite
returning exit code 0. The report records the added Lua support and classifies
these non-standalone inputs separately.

Rasterization intersects analytic row spans and combines them with integer
union/subtraction operations. Conservative fill bounds skip empty rows and
slabs. Large 3D fills use up to 16 scoped slab workers; ordered fills still
overwrite in source order. No per-point CSG allocation is needed. General
non-analytic projections retain a point-classification fallback.

Both sector files have 27,270,901 points. Their native compute times are
reported separately from SIMION's complete process wall time. SIMION does
not expose isolated compute time, so the requested conservative comparison
uses its whole process. The largest-five table includes Lua-generated inputs;
a separate benchmark can measure the five largest converted arrays with
three native samples per file.
The 102,010,000-point `geometry/sphere_mesh.gem` executes recursive Lua
functions and table operations before analytic cylinder rasterization.

```sh
# Run synchronously: each SIMION invocation has a timeout, closed stdin,
# and a Windows-drive working directory.
python3 crates/simrep-gem/tools/benchmark_gem.py --oracle
```

Run the required Windows cleanup after oracle work:

```sh
s="$(wslpath -w tools/win/winproc.ps1)"
(cd /mnt/c && timeout 30 powershell.exe -NoProfile -NonInteractive \
  -ExecutionPolicy Bypass -File "$s" cleanup < /dev/null)
```
