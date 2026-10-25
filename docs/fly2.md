# FLY2 particle definitions

`simrep-io` executes FLY2 in vendored Lua 5.1, using the same bounded runtime as
GEM. It does not execute FLY2 through SIMION, a shell, or the flight engine.
`read_particles(path)` loads ION CSV or a context-free FLY2 file.
`read_particles_in(path, &Fly2Context)` additionally supplies a workbench.

```rust
use simrep_io::{Fly2Context, Fly2Instance, read_particles_in};

let context = Fly2Context {
    instances: vec![Fly2Instance {
        origin_mm: [10.0, 20.0, 0.0],
        scale: 2.0,
        spacing_mm: [0.5, 0.5, 0.5],
        dimensions: [101, 101, 1],
        ..Default::default()
    }],
    seed: 42,
    ..Default::default()
};
let particles = read_particles_in("source.fly2", &context)?;
# Ok::<(), std::io::Error>(())
```

## Coordinates and units

`coordinates = 0` means workbench millimeters. A positive integer selects that
one-based PA instance: positions are PA grid units relative to its working
origin, and directions use its rotated axes. Grid spacing, instance scale and
rotation are applied once, producing physical workbench millimeters and unit
directions. TOB stays in microseconds; mass is amu, charge is elementary charges,
KE is eV, and speed/velocity is mm/us. Display color and signed charge weighting
factor are retained by `ParticleDefinition`.

`Fly2Instance.origin_mm` is the workbench location of the instance working
origin. `working_origin_mm` is that origin's offset from the PA array origin,
in unscaled PA millimeters. With the CLI's default zero working-origin offset,
this is exactly its `--origin`, `--axes`, `--scale` placement. Axes are columns,
the PA unit x/y/z directions expressed in workbench coordinates. Anisotropic PA
spacing is supported.

Missing instance context is an error naming `coordinates=N` or the attempted
`simion.wb.instances` access. A context-free read never treats PA grid units as
millimeters, including an empty PA-coordinate particle definition.

Defaults follow SIMION's executable FLY2 constructors: one particle, TOB zero,
electron mass `0.00054857990946`, charge -1, KE 0.1 eV, position at zero, +X
direction, CWF one and color zero. A zero direction is treated as +X, as by
SIMION's ION export. These constructor defaults differ from the former literal
parser's mass-100/charge-1 defaults. ION values are always explicit.

## Constructors and Lua

The environment supplies `particles`, `standard_beam`, `vector`, finite and
unbounded `arithmetic_sequence`/`line_sequence`, `sequence`,
`concatenated_sequence`, `circle_sequence`, `cone_direction_sequence`,
`uniform_distribution`, `gaussian_distribution`, `gaussian3d_distribution`, `line_distribution`,
`circle_distribution`, `cone_direction_distribution`, `sphere_distribution`,
and custom `distribution(function)` values. Distribution objects provide
`:iterator()`. Standard beams accept scalar position coordinates, vector
position/direction, azimuth/elevation, pupil/window points, speed or velocity,
TOB, mass, charge, KE, CWF and color.

FLY2 colors use SIMION's 16-entry palette (integer conversion and modulo 16).
Speed/velocity conversion uses SIMION's 2010 CODATA constants rather than
silently changing an existing source's KE to a newer constants edition.

Finite sequences expand in a Cartesian product, with TOB varying fastest.
Explicit beam `n` limits the total number of particles; without it, finite
sequence lengths determine that total. Unbounded sequences advance with the
particle index. Custom Lua expressions, local variables, functions, loops,
tables and coroutines are available. Position is evaluated before direction.
The pure math library is also exposed through SIMION's global math names and
its polar-coordinate/azimuth/elevation helpers.

Common expression unit constants are `mm`, `cm`, `m`, `um`, `nm`, `us`, `ns`,
`ms`, `s`, `eV`, `keV`, `MeV` and `amu`. They scale numbers into the units above;
they do not change the file's coordinate mode.

## Workbench and field access

`simion.wb.instances` exposes the supplied instance placements, grid dimensions,
spacings, working-origin offsets, coordinate transformations, and read-only
field callbacks. No context objects or host arrays are mutated by Lua.

`Fly2Context.field` is an optional borrowed, thread-safe callback receiving an
instance number and a point in workbench millimeters. Instance zero denotes a
workbench-level query. It returns `Fly2Field`: potential in volts, E in V/mm, B
in gauss, and electrode membership. `simion.wb:epotential`, `:efield`, `:bfield`,
instance `:potential_wc`, `:field_wc`, and PA `:potential_vc`, `:field_vc`,
`:electrode` use it. PA methods receive grid coordinates and return the
appropriate PA-axis grid-scaled electric field. A field query without a
provider is an error naming the missing field provider context.

Instances can supply an `electrodes` node mask. In that case `pa:electrode`
reads integer PA node indices directly, rather than confusing an isolated
electrode node with the flight engine's fully occupied collision cells. Without
a mask, electrode membership is requested from the field provider.

The CLI supplies its loaded PA placement and initial electric/magnetic fields.
Voltage expressions are evaluated at time zero for source generation. Python's
`fly(source, "particles.fly2", **options)` supplies the actual adjusted PA,
placement, initial fields and voltage bases, including in-memory `Field`
objects. The existing `read_particles(path)` call remains context-free. The
native binding additionally accepts optional PA arrays, metadata, settings
and bases, using the same data representation as its flight entry point.

## Includes, resource bounds and random streams

`require`, `dofile` and `simion.import` can read Lua/FLY2 source alongside the
input or inside explicit `Fly2Context.lua_roots`. For files inside a SIMION
`examples` tree, that tree and its sibling `lib/lua` directory are allowed too.
Paths are canonicalized and checked against these roots, including symlinks.
`require` caches loaded modules. Native shared-library modules are not loaded.
Files are never written; `io`, `os`, package loading and debug facilities are
not exposed. Diagnostic `print` calls are discarded. GEM keeps its own existing
virtual text IO and geometry layer on top of the shared sandbox.

The runtime bounds Lua to 256 MiB, five billion instructions and ten million
output particles. `Fly2Context.seed` selects a platform-independent SplitMix64
stream; `rand`, `simion.rand` and `math.random` use it. `simion.seed` and
`math.randomseed` reset it. Repeated reads with the same context reproduce the
same native particles. Native and SIMION random streams are not promised to
match sample-for-sample; comparisons therefore use distributions and moments.

The deliberately unsupported surface includes binary FLY files, native Lua
extensions, unrestricted filesystem/process access, negative coordinate modes,
and filled deterministic circle/cone sequences. Unknown constructors and
standard-beam fields fail explicitly. This is a particle-source environment:
workbench programs, GUI operations, PA mutation, refinement and segment
callbacks are not executed inside FLY2.

## Oracle comparison

`tools/simion-oracle/check_fly2.py` compares every installed example FLY2 with
SIMION's direct `simion.iob2.load_fly2_file(path):create_ionfile(fh)` export.
It never flies particles. Deterministic rows use ordered numeric comparison;
random rows aggregate explicit seeds and use two-sample KS plus first/second
moments. Results and limitations are recorded in `docs/fly2-validation.md`.

The SIMION API and source examples are documented at
`https://simion.com/info/fly2_file.html` and
`https://simion.com/info/lua_simion.wb.html`.
