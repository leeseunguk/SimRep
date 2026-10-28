# Python and NumPy API

`crates/simrep-py` is an opt-in PyO3 extension with a small Python package named
`simrep`. Simulation uses the other crates' public APIs. Auxiliary workflows
compile the CLI's Rust frontend into the extension, including its argument
parser: they do not launch a CLI subprocess or duplicate physics. The workspace's existing
`crates/*` member glob includes it. Ordinary `cargo build`, `cargo clippy` and
`cargo test --workspace --release` compile an empty Rust library for this crate;
they do not require Python, NumPy headers or libpython. Maturin enables its
`python` feature and PyO3's `extension-module` feature. Wheels use the stable
CPython ABI (`abi3-py310`, Python 3.10 or newer).

## CLI parity

All user-facing CLI commands are available. "Shared" means the existing CLI Rust
frontend runs in-process, with identical defaults, validation, output formats and
feature gates. Other rows use the same public Rust simulation functions with
owned NumPy inputs/outputs. Paths accept `str` or `Path`; vectors/matrices accept
Python sequences, assignments and run settings accept dictionaries. Diagnostic
and benchmark functions return parsed, versioned dictionaries from the CLI's
global `--json` interface; human diagnostics go to stderr. They do not print
JSON or reports to stdout.

`pa_info()` returns header fields (`dims`, `spacing_mm`, `mode`, `symmetry`,
mirror flags and field type), potential statistics, and `simion_compatible` / `simion_issues` diagnostics (currently the observed cylindrical dy/dz incompatibility; not an exhaustive SIMION validator). The PA reader already rejects invalid or zero spacing. `gpu_info()` returns
`adapter_name`, `adapter_type`, `backend`, `f64_support` and `software_adapter`.
`gpu_smoke()` adds numerical `checks`; `fly_bench()`, `refine_bench()` and
`refine_sdf()` return benchmark dictionaries. All include `schema_version=1`
and `simrep_version`. The four `export_*()` functions now return
`{"output_paths": [...], "schema_version": 1, "simrep_version": "..."}`.
`gem2pa()` and `mesh_import()` still return `PA`; `refine()`, `refine_geometry()`
and `fly()` retain their structured result types. File-based
`refine_geometry().report` contains the CLI's executed device/precision,
cycles, residual, convergence, separate compute/preparation times and paths.
`fly().report` includes `cuts_path` (path or None) and `cuts_source` (`explicit`,
`sibling`, `disabled`, or `none`); a missing sidecar emits a `UserWarning`.
An in-memory cuts dictionary has no path. `no_cuts=True` silences the warning.
Native errors still raise `ValueError`, rather than returning error dictionaries.

| CLI command / option | Python function / argument | Status |
| --- | --- | --- |
| `pa-info FILE` | `pa_info(source)`; `read_pa(source)` for arrays/metadata | Shared |
| `gpu-info --backend auto` | `gpu_info(backend="auto")` | Shared |
| `gpu-smoke --backend auto` | `gpu_smoke(backend="auto")` | Shared |
| `gem2pa INPUT [OUTPUT]` | `gem2pa(source, output=None)` | Shared; Lua preprocessing and requested fractional `.pa-surf`; returns PA |
| `fly-bench --ions 10000` (`--particles` alias) | `fly_bench(ions=10000)` | Shared |
| `refine-bench --device cpu --backend vulkan --n 128` | `refine_bench(device="cpu", backend="vulkan", n=128)` | Shared |
| `refine-bench --tolerance 1e-10 --max-cycles 100` | `refine_bench(tolerance=1e-10, max_cycles=100)` | Shared |
| `refine-sdf --geometry sphere --n 65` | `refine_sdf(geometry="sphere", n=65)` | Shared; also `cylinder`, `cylindrical` |
| `refine-sdf --device cpu --backend vulkan` | `refine_sdf(device="cpu", backend="vulkan")` | Shared |
| `refine-sdf --tolerance 1e-10 --max-cycles 100 --compare-staircase` | `refine_sdf(tolerance=1e-10, max_cycles=100, compare_staircase=False)` | Shared |
| `refine FILE` | `refine(source)`; `Field.write(path)` for output | Supported; PA, PA#, in-memory PA |
| `refine --device auto --backend vulkan` | `refine(device="auto", backend="vulkan")` | Supported; same automatic dielectric class policy |
| `refine --tolerance 1e-10 --max-cycles 100` | `refine(tolerance=1e-10, max_cycles=100)` | Supported |
| `refine --permittivity PATH` | `refine(permittivity=path)` | Supported; cell PA and linear/nonlinear JSON laws; nonlinear PA# rejected |
| `refine-geometry INPUT --electrode PART=N:V` | `refine_geometry(source, electrodes={part: (number, volts)})` | Shared for GEM/mesh/CAD files; analytic lists also supported |
| `refine-geometry --grid --spacing --origin 0,0,0` | `dims=None, spacing_mm=None, origin_mm=(0,0,0)` | Shared; GEM grid/spacing defaults retained |
| `refine-geometry --device cpu --backend vulkan` | `device="cpu", backend="vulkan"` | Shared |
| `refine-geometry --tolerance 1e-10 --max-cycles 100` | `tolerance=1e-10, max_cycles=100` | Shared |
| `refine-geometry --out STEM` | `out=stem` | Shared; PA#, PA0, bases and exact v3 cuts; omitted uses temporary storage |
| `refine-geometry --sidecar-dense-sdf` | `sidecar_dense_sdf=False` | Shared; dense links/sampled fallback are opt-in |
| `refine-geometry --surface-supersampling 2` | `surface_supersampling=2` | Shared; positive integer, used only for dense fallback |
| `refine-geometry --units --repair` | `units=None, repair=False` | Shared; mesh/CAD validation preserved |
| `refine-geometry --cad-mode auto --cad-tolerance` | `cad_mode="auto", cad_tolerance=None` | Shared; `exact`/`tessellated`, requires Cargo `cad` feature |
| `mesh-import INPUT OUTPUT --grid` | `mesh_import(source, output, dims=...)` | Shared; returns PA |
| `mesh-import --spacing 1,1,1 --origin 0,0,0 --units` | `spacing_mm=(1,1,1), origin_mm=(0,0,0), units=None` | Shared |
| `mesh-import --transform` | `transform=(1,0,0,0,0,1,0,0,0,0,1,0)` | Shared; row-major 3x4 |
| `mesh-import --electrodes --voltages` | `electrodes=None, voltages=None` | Shared; defaults sequential numbers and zero SDF voltages |
| `mesh-import --repair --allow-defects --sdf-prefix` | `repair=False, allow_defects=False, sdf_prefix=None` | Shared; same conflicts and topology checks |
| `export geometry INPUT OUTPUT --electrode` | `export_geometry(source, output, electrode=None)` | Shared |
| `export geometry --origin --angular-segments 64 --sweep-degrees 360 --extrusion-depth 1` | `origin_mm=(0,0,0), angular_segments=64, sweep_degrees=360, extrusion_depth=1` | Shared |
| `export grid INPUT OUTPUT --origin --no-field --revolve` | `export_grid(source, output, origin_mm=(0,0,0), no_field=False, revolve=False)` | Shared; VTI or revolved VTS |
| `export grid --angular-segments 64 --sweep-degrees 360` | `angular_segments=64, sweep_degrees=360` | Shared |
| `export isosurface INPUT OUTPUT --potential` | `export_isosurface(source, output, potential=...)` | Shared |
| `export isosurface --origin --angular-segments --sweep-degrees --extrusion-depth` | Same keywords/defaults as `export_geometry` | Shared |
| `export trajectories INPUT OUTPUT` | `export_trajectories(source, output)` | Shared; normalized CSV or VTP |
| `fly --pa --particles` | `fly(source, particles)` | Supported; PA/Field/path/None; NumPy or ION/FLY2 particles |
| `fly --device auto --backend vulkan` | `device="auto", backend="vulkan"` | Supported; same CPU/GPU/auto classes and adapter filters |
| `fly --cuts --no-cuts` | `cuts=None, no_cuts=False` | Supported; sibling pickup, paths or dictionaries, exact surfaces retained by Field |
| `fly --voltage N=V` | `voltages={number: volts}` | Supported; unspecified voltages retained |
| `fly --voltage-expr N=EXPR --param NAME=VALUE` | `voltage_expr={number: expr}, parameters={name: value}` | Supported; same Rust expression parser |
| `fly --run-file` | `run_file=path_or_dict` | Supported; explicit values override run settings |
| `fly --origin --axes --scale 1` | `origin_mm=(0,0,0), axes=((1,0,0),(0,1,0),(0,0,1)), scale=1` | Supported; axes column by column |
| `fly --bounds --plane` | `bounds=[[xmin,ymin,zmin],[xmax,ymax,zmax]], plane=[nx,ny,nz,d]` | Supported; omitted by default |
| `fly --output` | `output=None`; `FlightResult.arrays/records` | Supported; CLI-format CSV or owned NumPy columns |
| `fly --tolerance --max-step-us` | `tolerance=None, max_step_us` omitted | Supported; automatic field-aware tolerance and step ceiling |
| `fly --duration-us 100 --allow-time-limit` | `duration_us=100, allow_time_limit=False` | Supported; duration is only a successful stop when opted in |
| `fly --nonrelativistic --global-cubic` | `relativistic=True, global_cubic=False` | Supported; collisions force nonrelativistic dynamics |
| `fly --nearest-node-collisions` | `nearest_node_collisions=False` | Supported; occupied PA cells remain the default |
| `fly --exact-coils` | `exact_coils=False` | Supported; disables CPU coil maps |
| `fly --magnetic-pa --bfield` | `magnetic_pa=[path], bfield=[0,0,0]` | Supported on CPU/GPU |
| `fly --bfield-coil JSON` | `coils=[{"kind": "loop", ...}]` | Supported on CPU/GPU; segment/loop/solenoid dictionaries |
| Run JSON `physics.magnetic_pas`, `magnetic_support`, `uniform_gauss`, `coils` | `run_file={"physics": {...}}` or `magnetic_pas`, `magnetic_support`, `bfield`, `coils` | Supported; independent placement, ng, polarity and angular sectors |
| `fly --integrator rk45 --step-us 0.001` | `integrator="rk45", initial_step_us=0.001` | Supported; Boris also on CPU/GPU |
| `fly --collision hs1/sds --seed 1` | `collision=None, seed=1` | Supported on CPU/GPU |
| `fly --pressure-pa --temperature-k --gas-mass-amu` | `pressure_pa, temperature_k, gas_mass_amu` omitted | Supported; model-dependent Rust defaults |
| `fly --gas-velocity 0,0,0 --cross-section-m2 2.27e-18` | `gas_velocity=(0,0,0), cross_section_m2=2.27e-18` | Supported |
| `fly --mean-free-path-mm --collision-steps 20` | `mean_free_path_mm=None, collision_steps=20` | Supported |
| `fly --mbmr --gas-diameter-nm 0.366 --ion-diameter-nm --mobility-stp --no-diffusion` | `mbmr=None, gas_diameter_nm=0.366, ion_diameter_nm=None, mobility_stp=None, diffusion=True` | Supported; SDS requires runtime statistics path |
| `fly --space-charge direct/pic` | `space_charge=None` | Supported; direct CPU, PIC CPU/GPU/auto |
| `fly --charge-mode factor/coulombic/beam --charge-amount 1` | `charge_mode="factor", charge_amount=1` | Supported; same CWF normalization |
| `fly --beam-current-a` | `beam_current_a=None` | Supported; beam-only, CPU/auto, incompatible with ensemble space charge |
| `fly --charge-softening-mm 0 --pic-dims 33,33,33` | `charge_softening_mm=0, pic_dims=(33,33,33)` | Supported; PIC requires full Cartesian bounds |
| `fly --space-charge-step-fraction` | `space_charge_step_fraction=None` | Supported; omitted means beam 1.0, ensemble 0.1 |
| Sampled trajectories (native API; CLI terminal CSV unchanged) | `trajectories=False, sample_every=1, trajectory_indices=None, max_samples_per_ion=10000` | CPU/GPU; selected ordered zero-based indices; GPU capacity overflow raises |
| Global `--help` / `--version` | Python `help(simrep.function)` / installed package metadata | Presentation equivalents |

Python also retains library-only `particle_format="velocity"`, `min_step_us`,
`max_steps`, field-aware step-limit keywords and `transparent_faces`. These do
not remove any CLI capability. The auxiliary functions require the repository's
CLI sources when building from source; wheels are self-contained. Build CAD
wheels with `maturin build ... --features cad`; without OCCT, CAD raises the same
feature-gate error as a default CLI build.

## WSL build and checks

From the repository root, with Rust, Python and venv support already installed:

```sh
python3 -m venv /tmp/simrep-python
. /tmp/simrep-python/bin/activate
python -m pip install maturin numpy pytest scipy
maturin develop --release --manifest-path crates/simrep-py/Cargo.toml
cargo build --release -p simrep-cli
python -m pytest crates/simrep-py/tests -q
python crates/simrep-py/examples/einzel_scan.py --ions 128 --iterations 16
python crates/simrep-py/examples/tof_scan.py --ions 128 --points 9
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo clippy -p simrep-py --features python -- -D warnings
cargo test --workspace --release
```

Use `SIMREP_BIN=/absolute/path/to/simrep` when the CLI has a different output
directory. Flight cases using `compare_flight` in `tests/test_cli_parity.py`
invoke the real CLI with matching inputs and settings, then compare every CSV
result column. `test_fly_equals_cli_output` in `tests/test_simrep.py` covers
ION/FLY2 inputs and compares reason, ion, TOF, position and kinetic energy.
GPU numeric parity (PIC, magnetic PA/coils, HS1/SDS and sampled trajectories)
requires a hardware GPU and is explicitly skipped when none is available.
The separately named software-adapter refusal test checks that both CLI and
Python reject explicit GPU flight; it is not a numeric comparison.
No SIMION installation or downloaded example data are needed.
The suite also covers metadata/PA0 trailers, NumPy lifetimes, anisotropic
spacing, cylindrical refinement, fast-adjust disk parity, GEM, OBJ, analytic
cut cells, voltage expressions, trajectories, magnetic fields and direct/PIC
space-charge options.

For a distributable Linux wheel:

```sh
maturin build --release --manifest-path crates/simrep-py/Cargo.toml
```

## Windows build with cargo.exe

Use a Windows Python and the installed Windows Rust/MSVC toolchain, rather
than combining a Linux interpreter with a Windows extension. The exact WSL
invocation verified on Windows 11 (venv at `C:\Users\lsu10\simrep-pyvenv`,
Python 3.12.10, maturin 1.15.0; cargo.exe in `C:\Users\lsu10\.cargo\bin`):

```sh
# Every Windows exe launched from WSL needs stdin closed (< /dev/null),
# a timeout, and a /mnt/c cwd. Env vars crossing into Windows need WSLENV;
# suffix /p translates a WSL path value to a Windows path.
export WSLENV="CARGO/p:CARGO_TARGET_DIR/p"
export CARGO=/mnt/c/Users/lsu10/.cargo/bin/cargo.exe
export CARGO_TARGET_DIR=/mnt/c/Users/lsu10/simrep-target-py
(cd /mnt/c && timeout 1500 \
  /mnt/c/Users/lsu10/simrep-pyvenv/Scripts/maturin.exe build --release \
  --manifest-path '\\wsl.localhost\Ubuntu\home\lsu10\orca\workspaces\SIMION\SIMION_Rep\crates\simrep-py\Cargo.toml' \
  -o 'C:\Users\lsu10\simrep-wheels' < /dev/null)
# cargo.exe and maturin.exe accept the \\wsl.localhost UNC path directly;
# no Windows-side checkout is needed. Observed build time: 4 min 19 s on
# a 16-core Ryzen 5950X with dependencies already compiled.
(cd /mnt/c && timeout 120 /mnt/c/Users/lsu10/simrep-pyvenv/Scripts/python.exe \
  -m pip install --force-reinstall --no-deps \
  'C:\Users\lsu10\simrep-wheels\simrep-0.1.0-cp310-abi3-win_amd64.whl' < /dev/null)
(cd /mnt/c && timeout 30 /mnt/c/Users/lsu10/simrep-pyvenv/Scripts/python.exe \
  -c "import simrep; print(simrep.__file__)" < /dev/null)
```

For the pytest CLI-parity cases, build a Windows `simrep.exe` and pass it
through `WSLENV` as a literal string (no `/p`, the value is already a
Windows path):

```sh
export WSLENV="CARGO/p:CARGO_TARGET_DIR/p:SIMREP_BIN"
export SIMREP_BIN='C:\Users\lsu10\simrep-target-py\release\simrep.exe'
(cd /mnt/c && timeout 900 /mnt/c/Users/lsu10/.cargo/bin/cargo.exe build \
  --release -p simrep-cli \
  --manifest-path '\\wsl.localhost\Ubuntu\home\lsu10\orca\workspaces\SIMION\SIMION_Rep\Cargo.toml' < /dev/null)
# simrep-cli build with warm deps: ~1 min.
(cd /mnt/c && timeout 900 /mnt/c/Users/lsu10/simrep-pyvenv/Scripts/python.exe \
  -m pytest '\\wsl.localhost\Ubuntu\home\lsu10\orca\workspaces\SIMION\SIMION_Rep\crates\simrep-py\tests' \
  -q < /dev/null)
# Without WSLENV the SIMREP_BIN variable silently does
# not reach the Windows process and the parity test falls back to the
# nonexistent <checkout>/target/release/simrep path.
s="$(wslpath -w tools/win/winproc.ps1)"
(cd /mnt/c && timeout 30 powershell.exe -NoProfile -NonInteractive \
  -ExecutionPolicy Bypass -File "$s" cleanup < /dev/null)
# Keep the final process-count and orphan-cleanup output with the test evidence.
```

`maturin` finds `cargo.exe` on Windows (pinned here via `CARGO`). Running
Linux maturin with Linux Python does not create a Windows wheel. No global
Python packages or system installations are needed. For GPU use on Windows,
`backend="vulkan"` is verified working on an RTX 3070 (wgpu Vulkan):
`simrep.refine(pa, device="gpu", backend="vulkan")` solved a 1x7x9 planar
ramp in ~33 ms and `simrep.fly(..., device="gpu", backend="vulkan")` ran on
the same adapter. Windows wheels have a distinct platform tag
(`win_amd64`) from Linux wheels.

## PA I/O and metadata

```python
import numpy as np
import simrep

pa = simrep.read_pa("lens.pa0")
print(pa.dims)                 # (nx, ny, nz)
print(pa.potentials.shape)     # (nz, ny, nx), C-order, x fastest
print(pa.metadata)
simrep.write_pa("copy.pa0", pa)

values = np.zeros((1, 21, 41), dtype=np.float64)
electrodes = np.zeros_like(values, dtype=bool)
electrodes[:, :, 0] = electrodes[:, :, -1] = True
values[:, :, 0], values[:, :, -1] = 1, 2
definition = simrep.PA(values, electrodes, {"spacing_mm": [0.5, 0.5, 1]})
definition.write("plates.pa#")
```

Metadata fields are `mode` (-1/-2), `max_voltage`, `ng`, `symmetry`
(`planar`/`cylindrical`), `mirrors`, `anti_mirrors` (three booleans), `field_type`
(`electrostatic`/`magnetic`), `refinable`, `spacing_mm` (x,y,z), and `pa0`.
Missing metadata uses the Rust PA defaults. The optional `pa0` dictionary holds
`reference_voltage`, `electrode_indices`, and `scalable`: `None` preserves an
omitted flag, `False` an explicit disabled flag, and a list stores voltages.
Electrode values are decoded volts, or electrode numbers for a PA#.
The PA format's offset encoding can round electrode voltages on write; it is
not a bitwise arbitrary-f64 storage format.

Native output buffers transfer ownership to NumPy without a second data copy;
NumPy's owner keeps the Rust allocation alive after the PA object is dropped.
This is zero-copy ownership transfer, not an mmap of the encoded PA file.
Two-dimensional input is promoted to a singleton z axis. Input dtypes and
strides are normalized, and Rust then takes an owned snapshot before releasing
the GIL so another Python thread cannot mutate a running simulation's input.
Do not mutate the same arrays concurrently while that initial snapshot is taken.

## Refinement and voltage scans

```python
field = simrep.refine(definition, adjustable=True, device="cpu", tolerance=1e-10)
field = field.fast_adjust({1: 100, 2: 0})
field.write("plates.pa0")      # also writes plates.pa1 and plates.pa2
loaded = simrep.load_field("plates.pa0")
adjusted = simrep.fast_adjust(loaded, {1: 200})
print(adjusted.report["compute_ms"])
```

`refine` accepts an in-memory PA or a path. A `.pa#` path automatically enables
adjustable refinement; an ordinary `.pa` or in-memory PA is solved as volts
unless `adjustable=True`. CPU and GPU refinement use the same solver and
anti-mirror/cylindrical adapters as the CLI. `device` accepts `cpu`, `gpu`, or
`auto`; refinement's `auto` uses the CLI's dielectric class policy and falls back
to CPU when an eligible hardware adapter is unavailable. Ordinary vacuum
refinement uses CPU by default. `backend` accepts `auto`, `vulkan`, `dx12`,
`metal`, or `all`. The default device is `auto`. A failed solve raises `ValueError` with its cycle
count; raise `max_cycles` or check the electrode constraints.

Fast adjustment retains unspecified voltages and any fixed background.
`load_field` uses `read_fast_adjust` to derive unit-voltage bases, including
PA0's ordinary and extended scalable conventions. It loads once, so an
optimization loop should keep the returned `Field`, not reopen the file on
each iteration. Fast adjustment uses NumPy vector arithmetic; it does not run
the solver again. `Field.write` emits normalized 10000 V siblings and PA0.

```python
gem = simrep.refine_geometry(
    "einzel.gem", electrodes={1: (1, 0), 2: (2, 110), 3: (3, 0)})
mesh = simrep.refine_geometry(
    "lens.stl", dims=(81, 81, 81), spacing_mm=(0.5, 0.5, 0.5),
    origin_mm=(-20, -20, -20), electrodes={0: (1, 0), 1: (2, 110)},
    repair=True, units="mm", device="gpu")
```

`refine_geometry` uses continuous cut cells, not staircase rasterization.
GEM defaults to its dimensions, spacing, symmetry and mirrors. Mesh formats,
units, part names/indices and repair follow `docs/mesh-io.md`; every part needs
exactly one assignment. STEP/IGES use the CLI's CAD pipeline when the wheel was
built with the `cad` Cargo feature.
An in-memory list supplies analytic SDFs with the same negative-inside
convention:

```python
shapes = [
    {"part": "left", "shape": {"kind": "plane", "normal": [1, 0, 0], "offset": 0}},
    {"part": "right", "shape": {"kind": "plane", "normal": [-1, 0, 0], "offset": -8}},
]
field = simrep.refine_geometry(
    shapes, dims=(9, 7, 1), electrodes={"left": (1, 3), "right": (2, 11)})
```

Available shapes are `sphere(center,radius)`, `cylinder(center,radius,axis)`,
`box(center,half_extent)`, `plane(normal,offset)` with a unit normal, and
`union`, `intersection`, `difference` with nested `a`/`b` shape dictionaries.
Keys are named fields in each dictionary, not positional arguments.
Planar and cylindrical analytic grids are supported. Each electrode must
include an inside/on-surface grid node. GEM sampling and physical-mask versus
solver-snap conventions follow `docs/geometry-pipeline.md`. A `Field` retains
its physical origin for flying; PA files cannot store that origin.

## Particle input, results and trajectories

```python
ions = simrep.particles_from_energy(
    position_mm=[[1, 2, 0], [1, 3, 0]], direction=[1, 0, 0],
    energy_ev=[2, 3], mass_amu=39, charge_e=1)
result = simrep.fly(
    field, ions, device="cpu", duration_us=5, plane=[1, 0, 0, 6],
    tolerance=1e-12, max_steps=1_000_000, trajectories=True, sample_every=10)
print(result.arrays["tof_us"])  # dictionary of owned NumPy columns
print(result.records)          # a structured NumPy array (copies columns)
print(result.report)           # device, compute_ms, setup_ms, total_ms
```

`read_particles("beam.ion")` and `read_particles("beam.fly2")` return `(N,11)`
float64 energy rows: `birth_us,mass_amu,charge_e,x,y,z,dx,dy,dz,energy_ev,CWF`.
Directions are normalized natively. The CWF column is optional for array input.
FLY2 preprocessing and ION azimuth/elevation rules use the shared Rust/CLI
reader, not a separate Python parser. No Python code from those input files is
executed.
`fly` also accepts a particle filename directly. With
`particle_format="velocity"`, NumPy rows instead contain
`mass_amu,charge_e,x,y,z,vx,vy,vz,birth_us[,CWF]`, velocities in mm/us.
Position is mm, time us, potential V, energy eV, mass amu and B gauss.
Relativistic dynamics default to true, matching the CLI; set
`relativistic=False` to opt out. Collisions use the CLI's nonrelativistic mode.

Result columns include the CLI's `ion,tof_us,x_mm,y_mm,z_mm,ke_ev,reason` plus
`vx_mm_per_us,vy_mm_per_us,vz_mm_per_us,time_us,accepted_steps,rejected_steps`.
`ion` is one-based; order matches input. Reasons are `electrode`, `outside`,
`plane`, `time_limit`, `step_limit`, `step_underflow`, or `nonfinite`.
As with the CLI, duration limits require `allow_time_limit=True`; other failed
terminal events raise `ValueError`. Beam results additionally distinguish
`beam_diverged` and `beam_leader_stopped`. `electrode_number` is zero when no
numbered exact surface was hit (the CLI writes an empty CSV cell).
Optional trajectories are one `(samples,7)` array per input particle:
`time_us,x,y,z,vx,vy,vz`. Initial and final states are retained. Trajectories
are supported on CPU and GPU, including Boris integration. `trajectory_indices`
selects ordered zero-based input indices; the returned paths follow that order.
GPU sampling uses `max_samples_per_ion=10000` by default and reports capacity
overflow rather than dropping samples. Sampling preserves terminal dynamics.

### Flight options

`device="auto"` uses the CLI's 4096-ion crossover, rejects software/f32 adapters
for automatic selection and uses CPU for CPU-only physics. Explicit `gpu`
reports missing adapters or unsupported physics instead of silently changing
the requested device. GPU has the same native precision/feature limits as
`simrep-fly`; no claim of bitwise CPU/GPU equality is made.

Keyword options are checked strictly; unknown names raise `ValueError`:

* Integration: `duration_us=100`, `tolerance=None` (automatic field-aware tolerance),
  `initial_step_us=0.001`, `min_step_us=1e-14`, `max_step_us` (automatic if
  omitted), `max_steps=1000000`, `integrator="rk45"|"boris"`,
  `max_grid_cells=4`, `max_cyclotron_radians=0.2`,
  `max_voltage_phase_radians=0.2`, `global_cubic=False`.
* Placement: `origin_mm`, `axes` (three PA axis vectors, column by column),
  `scale=1`, `bounds=[[xmin,ymin,zmin],[xmax,ymax,zmax]]`,
  `plane=[nx,ny,nz,d]` for `normal.position=d`.
* Voltages: `voltages={2:110}`, `voltage_expr={2:"offset+amplitude*sin(2*pi*f*t)"}`,
  `parameters={"offset":100,"amplitude":10,"f":0.14}`. Expressions use absolute
  time in us and the same Rust DSL as the CLI, with named values bound before
  flight. Expressions replace the specified electrode's static voltage.
* Magnetic: `bfield=[Bx,By,Bz]`, `magnetic_pa=["field.pa"]` with header `ng`
  and the electric PA's placement, `coils=[{"kind":"loop",...}]`. Coil variants
  are `segment(first_mm,last_mm,current_a)`,
  `loop(center_mm,normal,radius_mm,current_a)`, and
  `solenoid(first_mm,last_mm,radius_mm,current_a,turns)` as dictionaries.
* Collisions: `collision="hs1"|"sds"`, `seed=1`, `pressure_pa`,
  `temperature_k`, `gas_mass_amu`, `gas_velocity=[0,0,0]`,
  `cross_section_m2=2.27e-18`, `mean_free_path_mm`, `collision_steps=20`.
  SDS also accepts `mbmr="/path/to/mbmr.dat"`, `gas_diameter_nm=0.366`,
  `ion_diameter_nm`, `mobility_stp`, and `diffusion=True`. Its empirical data
  file is not bundled. Defaults and formulas match the CLI.
* Space charge: `space_charge="direct"|"pic"`, `charge_mode="factor"|"coulombic"`,
  `charge_amount=1`, `charge_softening_mm=0`, `pic_dims=[33,33,33]`,
  `space_charge_step_fraction=None` (ensemble default 0.1). PIC requires full Cartesian `bounds`.
  The same ensemble integrator, CWFs and restrictions as the CLI apply.
  Space charge and collisions cannot be combined.
  `charge_mode="beam", beam_current_a=...` instead uses the shared leader-plane
  beam integrator without `space_charge`; its default step fraction is 1.0.
* The library's `transparent_faces=[xmin,xmax,ymin,ymax,zmin,zmax]` boolean mask
  represents ideal wire-grid entrance/exit electrodes; it is opt-in and not
  inferred from voltage. This extra CPU-only library option is used by the TOF example.

Use `source=None` for field-free/magnetic experiments (supply bounds when a
finite workbench is needed). Magnetic arrays/coils, HS1/SDS and PIC support GPU
execution; direct Coulomb, beam and exact-surface collisions are CPU paths
(GPU PIC may use CPU exact-surface tracking). JSON run files can be passed as
paths or directly as `run_file` dictionaries, in addition to native keywords.
File/validation/solver failures raise `ValueError` with their underlying Rust
message and filenames where applicable. Native PA I/O, geometry sampling,
refinement, field reconstruction and integration release the GIL.

## Executable scan examples

`crates/simrep-py/examples/einzel_scan.py` generates an analytic three-ring
cylindrical einzel lens, refines its reusable bases once, and runs
`scipy.optimize.minimize_scalar` over the center voltage. Its objective
penalizes transmission loss through a 2 mm detector aperture and minimizes
detected RMS radius. This is a generated example, not the installed SIMION
einzel geometry or an accuracy/speed benchmark against SIMION.

`crates/simrep-py/examples/tof_scan.py` scans extraction voltage with finite
source-position/energy width, a transparent exit grid, and field-free drift.
It prints transmission, mean TOF, sample standard deviation and one-sigma
mass resolving power `mean_tof / (2*sigma_tof)`; this is not FWHM resolution.

Both scripts print a CSV row per iteration with `compute_ms` (fast-adjust
vector arithmetic plus native integration; no field reconstruction) and
`total_ms` (the complete Python iteration). Initial refinement compute time
is printed separately. Native reports also separate setup/reconstruction
from compute. Timings are observations, not assertions or benchmark thresholds.
