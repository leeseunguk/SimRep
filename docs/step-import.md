# STEP / IGES electrodes

`simrep-cad` imports located solid bodies from STEP AP203, AP214 and the
geometric B-rep subset of AP242, and IGES solid/closed-shell B-reps.
It implements the same millimeter, negative-inside `SignedDistance` contract
as `simrep-geom`. It does not depend on the solver. Nothing in `simrep-fly`,
`simrep-gem` or `app/` is changed.

## Kernel decision, October 23, 2026

Use **dynamically linked OCCT 7.6/7.7**, through a small owned Rust/C++ binding.
The SDK is an explicit optional dependency, not downloaded or compiled by the
default workspace build. The measured SDK here is Ubuntu's OCCT 7.6.3.
Newer OCCT releases reorganize exchange toolkit names; use the documented SDK,
not an arbitrary collection of runtime DLLs.

| Candidate | Measured / inspected evidence | Decision |
| --- | --- | --- |
| `opencascade` 0.3 / `occt-sys` 7.8.1 | Sources inspected; STEP/IGES readers exist, but the high-level API does not expose XCAF product/body names, solid classification, or trimmed-face closest-distance queries. `occt-sys::build_occt` explicitly requests a static OCCT build. The 13.7 MiB source crate downloaded in 1.58 s; CMake was absent on WSL. A full bundled-kernel build was **not measured**. | Would still require a native extension, and changes the linking/distribution contract. Do not ship the full static build. |
| Pure Rust `truck-stepio` 0.3 | Apache-2.0. Clean release dependency compilation took 28.85 s; fixing the evaluation program's `Option` handling added 1.30 s. Parsed/converted the installed OCCT `linkrods.step` and `screw.step` shells successfully (496.103 / 10.186 ms). Its public input table exposes shells, not the XCAF assembly/name/unit or IGES reader and solid-classification path needed here. | A useful STEP-to-mesh option, but not a replacement for both required formats and trimmed-solid distances. No claim that it failed to read these STEP files. |
| Owned narrow OCCT binding | Installing `libocct-data-exchange-dev` and `libocct-modeling-algorithms-dev` took 58.10 s. First `cargo check -p simrep-cad --features occt`, including Rust mesh dependencies, took 11.86 s. Initial release accuracy-example build including solver dependencies took 42.25 s. | Selected. The same installed kernel provides exchange, validity checks, classification, extrema and tessellation. |
| OCCT tessellation + `simrep-mesh` | Original sphere, cylinder and ring fixtures successfully tessellate and feed the existing repaired closed-manifold BVH SDF. Comparison below measures the approximation rather than assuming a fine mesh is exact. | Available explicitly, and automatically selected for invalid B-reps whose mesh remains a closed manifold. |

The two installed OCCT sample files are read-only robustness probes, not test
fixtures copied into this repository. The committed tests generate original
small models with the kernel. This is a bounded corpus, not a claim that all
industrial STEP/IGES files or AP242 PMI/tessellation entities are supported.

| Installed sample | OCCT bodies | Failed exact queries / 100 | OCCT preparation ms | OCCT queries ms |
| --- | ---: | ---: | ---: | ---: |
| `linkrods.step` | 1 | 0 | 923.363 | 1029.800 |
| `screw.step` | 1 | 0 | 34.523 | 196.895 |

Both retain their source product name, `"the product name"`. Query points are
deterministically distributed through the geometric bounds, not selected only
from easy far-field points. The pure-Rust measurements above stop after shell
conversion; their times are not compared as equivalent SDF workloads.

Primary references:

* OCCT STEP/XDE format coverage and names/assembly translation:
  <https://dev.opencascade.org/doc/overview/html/occt_user_guides__step.html>.
* OCCT LGPL 2.1 with additional exception:
  <https://github.com/Open-Cascade-SAS/OCCT/blob/V7_6_3/LICENSE_LGPL_21.txt>
  and <https://github.com/Open-Cascade-SAS/OCCT/blob/V7_6_3/OCCT_LGPL_EXCEPTION.txt>.
* Compared Rust binding: <https://github.com/bschwind/opencascade-rs>.
* Compared Rust reader: <https://github.com/ricosjp/truck/tree/master/truck-stepio>.

OCCT is not relicensed by this crate. The bridge is linked into the executable,
but OCCT itself is dynamically linked. When distributing a CAD-enabled binary,
ship the matching OCCT shared libraries, their license/exception notices and
the corresponding OCCT source availability information, and preserve users'
ability to replace the libraries. A full static OCCT distribution has additional
relinking obligations and is deliberately not the build selected here.

## Build and use

WSL / Ubuntu:

```sh
sudo apt-get install --no-install-recommends \
  libocct-data-exchange-dev libocct-modeling-algorithms-dev
cargo build --release -p simrep-cli --features cad
target/release/simrep refine-geometry electrodes.step \
  --grid 81,81,81 --spacing 0.25,0.25,0.25 --origin -10,-10,-10 \
  --electrode 'assembly/anode=1:120' --electrode 'assembly/ground=2:0' \
  --cad-mode exact --out target/electrodes
```

`.step`, `.stp`, `.iges`, and `.igs` are case-insensitive input extensions.
CAD requires a planar 3D grid. Names and zero-based body indices use the
existing `--electrode part=number:voltage` assignment rules; no implicit
combining of distinct bodies occurs. XCAF product/instance paths are retained,
with named sub-bodies where present and `/body-N` suffixes for otherwise
indistinguishable solids. Repeated assembly instances remain distinct and
have their accumulated source placements applied.

Use `--cad-mode tessellated --cad-tolerance 0.001` to reuse the mesh path.
Tolerance is absolute chordal deflection in mm, not a relative percentage.
Its default is `h_min/100`. OCCT's angular deflection is 0.35 radians.
`auto` (the default) uses exact queries for valid B-reps and tessellation for
invalid B-reps. If an OCCT distance fails at a node or during edge-root
tracing, the CLI records that failure and restarts the entire solve in
tessellated mode rather than mixing two surfaces pointwise.
`exact` requires a valid B-rep. Open or non-manifold tessellations still fail:
a fallback cannot invent a closed electrode from arbitrary open surfaces.
An isolated runtime query failure is exposed by `CadPart::try_distance`;
the infallible SDF adapter reports a nonfinite value so the cut-cell validator
can reject it. No distance failure is silently replaced with zero.

GEM/mesh command behavior and outputs remain the same. CAD produces `.pa0`,
per-electrode basis siblings, the physical `.pa#` mask and `.cuts.json`.
All reported `compute_ms` values are solver compute only. Exchange, validity,
tessellation/BVH construction, SDF/cut queries and hierarchy preparation are
reported in `preparation_ms`, outside the speed denominator.

### Units

By default OCCT converts each declared representation's source lengths to mm;
STEP mixed-unit assemblies therefore keep their physical scale. IGES uses its
global unit metadata. `--units` overrides the declared source unit (the mesh
command's meaning), not an extra scale applied after conversion. For example,
overriding an inch file with `--units mm` shrinks its physical geometry by 25.4.
A mixed-unit STEP file is rejected when an override would be ambiguous.
Millimeter, meter and inch exchange round trips and overrides are tested.
Translations in the imported assembly are scaled together with the bodies.

### Guarded face-set gap measurement

The `cad` feature also provides a grid-independent measurement command:

```sh
simrep cad-measure pair.step --gap pair/left pair/right --json
simrep cad-measure pair.step --gap '0:faces=0' '1:faces=0' \
  --touch-tolerance 1e-7 --cad-tolerance 0.01
simrep cad-measure assembly.iges --all-pairs --json
```

Selectors follow `refine-geometry`: full XCAF part paths or zero-based body
indices. Omitting `:faces=...` measures the **complete** trimmed-face set of
each placed body, not a representative edge, section, or bounding box.
An explicit face subset must be nonempty, duplicate-free, and in range.
Face indices are zero-based OCCT face-map indices within that imported file;
they are not persistent identifiers across CAD edits or re-exports.
The API accepts valid B-reps only and preserves the declared source units and
assembly locations through the same STEP/IGES reader as refinement.

OCCT `BRepExtrema_DistShapeShape` measures the two face compounds in 3D,
including their boundary edges and vertices. Coverage checks ensure that every
selected face is present, tessellated, and that each closest-point support
belongs to a selected face. An independent Euclidean triangle-to-triangle
minimum includes face-interior crossings and edge-to-edge minima; a BVH prunes
triangle pairs. Each side is tessellated at `cad_tolerance/4` mm absolute
deflection. The actual discrepancy, not merely the requested deflection, must
be finite and at most `--cad-tolerance` (default 0.01 mm), or the command fails
without a success summary. Distances are boundary-to-boundary separations:
intersecting boundaries have zero gap; one body wholly inside another can
have a positive boundary gap. This is not a solid-overlap or penetration test.

`touching` means `min_distance_mm <= --touch-tolerance` (default 1e-7 mm,
nonnegative). It is a proximity classification, **not proof of electrical
contact**. Closest points and distances use mm. `--all-pairs` measures each
distinct pair once and returns symmetric matrices in the `parts` order, with
zero/true diagonals.

JSON uses the standard schema version 1 envelope. It includes `file`, `parts`
(`index`, `name`, `face_count`), `measurements` (`part_a`, `part_b`, `name_a`,
`name_b`, `faces_a`, `faces_b`, `min_distance_mm`, `closest_points_mm`,
`closest_faces`, `touching`, `tessellated_distance_mm`, `cross_check_error_mm`),
`touch_tolerance_mm`, and `cad_tolerance_mm`. `distance_matrix_mm` and
`touching_matrix` are square for `--all-pairs`, null for `--gap`. Failures use
the usual `error.message`/`error.kind` object and exit status 1.

Rust callers use `simrep_cad::MeasurementModel::read(path)`, `parts()`,
`gap(selector_a, selector_b, MeasureOptions::default())`, or `all_pairs`.
The MCP equivalent is `cad_measure` with `file`, either a two-string `gap`
array or `all_pairs: true`, and optional `cad_tolerance`/`touch_tolerance`.

`tools/cad-fixtures` generates the original `pair.step` fixture: two spheres
of radius 0.31 mm, with centers at x=-0.6 and +0.6 mm, so the analytic gap is
`1.2 - 2*0.31 = 0.58 mm`. Its `touching.step` uses centers at +/-0.31 mm and
has zero analytic gap. Both STEP and IGES round trips are checked against
these values. The complete-face guards are also tested on a three-face
cylinder and explicit end-cap subsets.

### Windows / MSVC

Use a complete OCCT 7.6/7.7 **MSVC x64 development SDK** matching the Rust
`x86_64-pc-windows-msvc` target and release CRT. In a Developer PowerShell:

```powershell
$env:OCCT_INCLUDE_DIR = 'C:\OCCT-7.6.3\inc'
$env:OCCT_LIB_DIR = 'C:\OCCT-7.6.3\win64\vc14\lib'
$env:PATH = 'C:\OCCT-7.6.3\win64\vc14\bin;' + $env:PATH
cargo.exe build --release -p simrep-cli --features cad
```

On this workstation, MSVC 2022 14.42.34433 and Windows `cargo.exe` were found.
The measured Windows feature check reached the native build script, then
failed with `Set OCCT_LIB_DIR to the MSVC OCCT import-library directory`.
FreeCAD 1.0 provides OCCT runtime DLLs but no matching headers/import-library
SDK, so a native Windows CAD executable is **not verified**. WSL libraries
cannot be linked into an MSVC executable. Installing a Windows SDK is the
remaining prerequisite; the default Windows build does not need it.

The probe ran in the foreground from `/mnt/c`, with closed stdin and a
45-second timeout. `tools/win/winproc.ps1 cleanup` reported 45 conhosts,
zero orphan conhosts and zero stopped orphans afterward.

The subsequent default-feature Windows check
`cargo.exe check -p simrep-cad` passed in 24.89 s without an OCCT SDK.
Final cleanup reported 48 conhosts, zero orphans and zero stopped orphans;
other sessions' active Windows tool processes were left alone.


## Exact distance and conservative far field

For each point, OCCT `BRepExtrema_DistShapeShape` computes distance from a point
vertex to a compound of the solid's **trimmed faces**, including edges and
vertices. The target must not be the solid itself: the distance to a containing
solid would incorrectly be zero. `BRepClass3d_SolidClassifier` supplies the
inside/outside sign, retaining holes and cavities. Queries and OCCT-owned object
destruction are serialized through an owned model mutex. Exchange/writer
settings are separately serialized because OCCT 7.6 uses process-global
`Interface_Static` parameters.

The solver sees `clamp(d_exact, -B, B)` with `B=2*max(spacing)`. This is exact
throughout the required surface band and retains the one-Lipschitz conservative
bound. A geometric OCCT bounding box supplies a coarse far-field test: when
distance to the enclosing box is at least B, the actual distance is at least B,
so the query returns B without extrema or classification. Within the box band,
the exact query is used and then clipped. There is no discontinuous switch
between a mesh distance and a CAD distance, and no interpolated zero set.
Deep interior distances are clipped only after classification.

"Exact" means no tessellation error, not symbolic arithmetic or infinite
precision. Extrema precision is requested at `1e-10` mm, classification at
`1e-12` mm; imported OCCT shape tolerances still apply. Closed IGES surface
groups are sewn at `1e-7` mm before constructing a solid. Open face sets are
not silently capped or converted into electrodes.

## Reproduce validation

```sh
cargo test -p simrep-cad -p simrep-cli --features simrep-cli/cad
cargo build --release -p simrep-cad --features occt --examples
RAYON_NUM_THREADS=16 target/release/examples/cad_accuracy
target/release/examples/cad_inspect \
  /usr/share/opencascade/data/step/linkrods.step \
  /usr/share/opencascade/data/step/screw.step
```

The tests cover three STEP schemas, original sphere/capped-cylinder/torus
surfaces, IGES, meter/inch units, named located assembly instances, negative
interiors, ring holes, finite-distance validation, and the clipped-distance
Lipschitz bound. CLI tests check all four extensions in both exact and fallback
modes, PA0 fast-adjust readback and physical cut fractions.

`cad_accuracy` writes its own AP242 files under `target/cad-accuracy`. The
analytic sphere and coaxial cylinder have inner/outer radii 0.31/0.83 mm at
1/0 V. The cylinder is a central 2D slice of the imported long capped solid,
so caps do not affect the tested annulus. The ring has major/minor radii
0.55/0.19 mm and a grounded spherical boundary at radius 0.95 mm. Its reference
is a 97-cubed analytic-torus-SDF cut-cell solve (four intervals per candidate
interval); identical domain and aligned
reference nodes avoid interpolation error. The exact imported field is also
compared with the analytic-SDF field at the same grid to isolate import error.

Fallback comparisons deliberately use a visible 0.02 mm deflection at an
identical solve grid, not the much finer production default. The assertion is
on RMS error across the physical vacuum. A coarse mesh can accidentally reduce
a particular maximum discretization error; report maxima too, rather than
claiming uniform error dominance.

### Measured accuracy and timing

October 23, 2026, Ryzen 9 5950X, WSL Linux release f64, 16 Rayon threads.
These are single serial runs on a shared workstation, not timing medians.
Both methods use the same grid, grounded exterior, solver and `1e-11`
relative residual tolerance. CAD generation and reference-field computation
are outside the candidate timings. Preparation includes exchange, validity,
SDF/cut construction and the solver hierarchy; compute includes only the
solver's reported solve phase.

| Shape / grid | Method | Triangles | RMS potential error V | Max potential error V | Preparation ms | Compute ms |
| --- | --- | ---: | ---: | ---: | ---: | ---: |
| Sphere / 33 cubed | Exact | 0 | 0.001348309 | 0.012769231 | 3047.130 | 47.677 |
| Sphere / 33 cubed | Tessellated | 646 | 0.005193689 | 0.032344199 | 31.004 | 49.763 |
| Cylinder / 65 squared | Exact | 0 | 0.000221849 | 0.000797456 | 464.119 | 4.005 |
| Cylinder / 65 squared | Tessellated | 140 | 0.001330479 | 0.003756581 | 9.886 | 4.825 |
| Ring / 25 cubed | Exact | 0 | 0.003825231 | 0.040355959 | 6384.288 | 34.857 |
| Ring / 25 cubed | Tessellated | 2592 | 0.005792104 | 0.032102006 | 68.335 | 30.863 |

Exact B-rep distances improve RMS potential error by **3.85x, 6.00x and
1.51x**, respectively. The ring comparison uses the independent 97-cubed
analytic-SDF reference. Its tessellated maximum error happens to be smaller,
even though its RMS error is larger; no maximum-error dominance is claimed.
The exact imported fields differ from the same-grid analytic-SDF fields by
at most `8.88e-16`, `2.88e-12` and `5.87e-12` V, respectively.
Direct B-rep queries cost substantially more preparation time than the mesh
BVH; this cost is reported rather than hidden in solver compute.

## Verification record

The merged workspace passed `cargo fmt --all -- --check`,
`cargo clippy --workspace --all-targets -- -D warnings`, and
`RAYON_NUM_THREADS=16 cargo test --workspace` with default features.
There were 123 passing ordinary tests, zero failures and zero ignored tests,
plus the custom GPU multigrid and f32/f64 smoke harnesses. WSL used software
Vulkan (`llvmpipe`); DX12 checks reported no adapter.

Default and CAD-feature verification use separate `CARGO_TARGET_DIR` values.
Do not build different feature variants into the same target directory while
CLI integration tests are executing: Cargo's shared `simrep` executable path
can otherwise be replaced by the other build between subprocess invocations.
The final verification uses `/tmp/simrep-cad-default-verification` and
`/tmp/simrep-cad-feature-verification`.

Rust LSP diagnostics were unavailable because the daemon timed out.
Cargo check/clippy and the native C++ compiler provided the diagnostics.
OCCT 7.6's external headers emit their existing `std::iterator` deprecation
warning under GCC 13; it is not suppressed. Rust clippy passes with warnings
denied.

The final isolated CAD-feature run passed
`cargo clippy -p simrep-cad -p simrep-cli --all-targets --features simrep-cli/cad -- -D warnings`
and `cargo test -p simrep-cad -p simrep-cli --features simrep-cli/cad`:
26 ordinary tests passed, zero failed and zero were ignored. The CAD CLI test
executes all four extensions in both exact and tessellated modes.
Both `cargo build --release -p simrep-cli --features cad` and
`cargo build --release -p simrep-cad --features occt --examples` exited zero.
The `cad_accuracy` executable exited zero with all same-grid exactness and
RMS-improvement assertions satisfied.
