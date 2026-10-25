"""SIMION-compatible arrays and native particle flight (mm, us, V, eV, amu)."""

from dataclasses import dataclass, field
import copy
import json
import os
from pathlib import Path
from tempfile import TemporaryDirectory
from time import perf_counter

import numpy as np

from . import _native

__all__ = [
    "PA", "Field", "FlightResult", "read_pa", "write_pa", "read_particles",
    "load_field", "refine", "refine_geometry", "fast_adjust", "fly",
    "particles_from_energy",
    "gem2pa", "mesh_import", "export_geometry", "export_grid", "export_isosurface",
    "export_trajectories", "pa_info", "gpu_info", "gpu_smoke", "fly_bench",
    "refine_bench", "refine_sdf",
]


def _json(value):
    return json.dumps(value, allow_nan=False)


def _grid(values, dtype):
    array = np.asarray(values, dtype=dtype)
    if array.ndim == 2:
        array = array[np.newaxis, :, :]
    if array.ndim != 3 or 0 in array.shape:
        raise ValueError("PA arrays need a nonempty (nz, ny, nx) or (ny, nx) shape")
    return np.ascontiguousarray(array)


@dataclass
class PA:
    """Decoded potential and electrode arrays, in C-order (nz, ny, nx).

    ``metadata`` preserves the PA header and optional PA0 trailer. Native output
    arrays own their Rust allocation through NumPy; they outlive this object.
    """

    potentials: np.ndarray
    electrodes: np.ndarray
    metadata: dict = field(default_factory=dict)

    def __post_init__(self):
        self.potentials = _grid(self.potentials, np.float64)
        self.electrodes = _grid(self.electrodes, np.bool_)
        if self.potentials.shape != self.electrodes.shape:
            raise ValueError("potential and electrode shapes differ")

    @property
    def dims(self):
        """Node counts in physical (nx, ny, nz) order."""
        return self.potentials.shape[::-1]

    def write(self, path):
        write_pa(path, self)


def _pa(native):
    potentials, electrodes, metadata = native
    return PA(potentials, electrodes, json.loads(metadata))


def _args(pa):
    return pa.potentials, pa.electrodes, _json(pa.metadata)


def read_pa(path):
    """Read .pa, .pa#, .pa0 or a numbered basis without running a solver."""
    return _pa(_native.read(os.fspath(path)))


def write_pa(path, pa):
    """Write a PA with its header/trailer; encoding errors raise ValueError."""
    _native.write(os.fspath(path), *_args(pa))


@dataclass
class Field:
    """A refined PA and reusable *unit-voltage* fast-adjust basis arrays."""

    pa: PA
    bases: dict = field(default_factory=dict)
    voltages: dict = field(default_factory=dict)
    report: dict = field(default_factory=dict)
    origin_mm: tuple = (0.0, 0.0, 0.0)
    _background: np.ndarray | None = field(default=None, repr=False)
    _cuts: dict | None = field(default=None, repr=False)

    def fast_adjust(self, voltages):
        """Return a field at new voltages, retaining unspecified voltages."""
        start = perf_counter()
        requested = dict(self.voltages)
        for number, voltage in voltages.items():
            number = int(number)
            if number not in self.bases:
                raise ValueError(f"electrode {number} has no fast-adjust basis")
            if not np.isfinite(voltage):
                raise ValueError("electrode voltages must be finite")
            requested[number] = float(voltage)
        background = self._background
        if background is None:
            background = self.pa.potentials.copy()
            for number, basis in self.bases.items():
                background -= self.voltages.get(number, 0.0) * basis
        values = background.copy()
        for number, basis in self.bases.items():
            values += requested.get(number, 0.0) * basis
        metadata = copy.deepcopy(self.pa.metadata)
        # New fields use the ordinary 10000 V basis convention on disk.
        if metadata.get("pa0"):
            metadata["pa0"]["scalable"] = None
            metadata["pa0"]["reference_voltage"] = 10000.0
        report = dict(self.report, compute_ms=(perf_counter() - start) * 1000)
        return Field(PA(values, self.pa.electrodes, metadata), self.bases,
                     requested, report, self.origin_mm, background, self._cuts)

    def write(self, path):
        """Write .pa0 plus .paN siblings, ready for the CLI's --voltage."""
        path = Path(path)
        if path.suffix.lower() != ".pa0":
            raise ValueError("a fast-adjust field must be written to a .pa0 path")
        adjusted = self.fast_adjust({})
        metadata = copy.deepcopy(adjusted.pa.metadata)
        metadata["pa0"] = None
        for number, values in adjusted.bases.items():
            suffix = ".pa_" if number == 0 else f".pa{number}"
            write_pa(path.with_suffix(suffix),
                     PA(values * 10000, self.pa.electrodes, metadata))
        write_pa(path, adjusted.pa)
        if self._cuts is not None:
            path.with_suffix(".cuts.json").write_text(_json(self._cuts), encoding="utf-8")


def load_field(path, *, no_cuts=False):
    """Load PA0 siblings and cuts once, preserving scalable/background rules.

    ``no_cuts=True`` bypasses sibling sidecar I/O, matching ``fly --no-cuts``.
    """
    path = Path(path)
    cuts_path = path.with_suffix(".cuts.json")
    cuts = (json.loads(cuts_path.read_text(encoding="utf-8"))
            if not no_cuts and cuts_path.is_file() else None)
    path = os.fspath(path)
    pa = read_pa(path)
    trailer = pa.metadata.get("pa0")
    if not trailer:
        return Field(pa, _cuts=cuts)
    indices = trailer["electrode_indices"]
    numbers = [n for n, index in enumerate(indices) if index != -1]
    background = _pa(_native.adjust(path, [(n, 0.0) for n in numbers]))
    bases = {}
    voltages = {}
    flat = pa.potentials.ravel()
    scalable = trailer["scalable"]
    for number in numbers:
        zero = _pa(_native.adjust(path, [(number, 0.0)]))
        unit = _pa(_native.adjust(path, [(number, 1.0)]))
        bases[number] = unit.potentials - zero.potentials
        if isinstance(scalable, list):
            voltages[number] = scalable[number]
        else:
            voltages[number] = float(flat[indices[number]])
    return Field(pa, bases, voltages, _background=background.potentials, _cuts=cuts)


def fast_adjust(source, voltages):
    """Adjust a Field in memory or load/adjust a PA0 and its siblings."""
    if isinstance(source, Field):
        return source.fast_adjust(voltages)
    return load_field(source).fast_adjust(voltages)


def refine(source, *, device="auto", backend="vulkan", tolerance=1e-10,
           max_cycles=100, adjustable=None, permittivity=None):
    """Refine a PA or .pa/.pa# path, returning a reusable Field.

    A .pa# path implies adjustable electrode numbers. For an in-memory PA,
    pass adjustable=True explicitly. Ordinary refinement never writes a file.
    """
    if not isinstance(source, PA):
        path = Path(source)
        if path.suffix.lower() not in (".pa", ".pa#"):
            raise ValueError("refine input must have a .pa or .pa# extension")
        if adjustable is None:
            adjustable = path.suffix.lower() == ".pa#"
        source = read_pa(path)
    settings = _json(dict(device=device, backend=backend,
                          tolerance=tolerance, max_cycles=max_cycles,
                          adjustable=bool(adjustable),
                          permittivity=os.fspath(permittivity) if permittivity is not None else None))

    def solve(pa):
        native, report = _native.refine(*_args(pa), settings)
        return _pa(native), json.loads(report)

    if not adjustable:
        pa, report = solve(source)
        return Field(pa, report=report)
    if source.metadata.get("pa0"):
        raise ValueError("adjustable refinement needs electrode-number PA#, not PA0")
    labels = source.potentials[source.electrodes]
    if labels.size == 0:
        raise ValueError("PA# has no electrodes")
    numbered = (labels >= 1) & (labels <= 1000) & (labels == np.floor(labels))
    numbers = sorted(set(labels[numbered].astype(int)))
    scalable = labels[~numbered]
    peak = float(scalable[np.argmax(np.abs(scalable))]) if scalable.size else None
    if peak is not None:
        numbers.insert(0, 0)
    indices = [-1] * max(32, max(numbers) + 1)
    all_labels = source.potentials
    numbered_grid = ((all_labels >= 1) & (all_labels <= 1000)
                     & (all_labels == np.floor(all_labels)))
    bases = {}
    compute_ms = 0.0
    for number in numbers:
        selected = source.electrodes & (
            ~numbered_grid if number == 0 else all_labels == number)
        candidates = np.flatnonzero(selected)
        index = candidates[np.argmax(np.abs(all_labels.ravel()[candidates]))] if number == 0 else candidates[0]
        indices[number] = int(index)
        values = np.zeros_like(source.potentials)
        if number == 0 and peak != 0:
            values[selected] = all_labels[selected] / peak * 10000
        elif number != 0:
            values[selected] = 10000
        metadata = copy.deepcopy(source.metadata)
        metadata["pa0"] = None
        basis, report = solve(PA(values, source.electrodes, metadata))
        bases[number] = basis.potentials / 10000
        compute_ms += report["compute_ms"]
    background = np.zeros_like(source.potentials)
    voltages = {number: 0.0 for number in numbers}
    if peak is not None:
        voltages[0] = peak
    metadata = copy.deepcopy(source.metadata)
    metadata["pa0"] = dict(reference_voltage=10000.0,
                           electrode_indices=indices, scalable=None)
    result = Field(PA(background, source.electrodes, metadata),
                   bases, {}, dict(compute_ms=compute_ms, device=report["device"]),
                   _background=background)
    adjusted = result.fast_adjust(voltages)
    adjusted.report = dict(compute_ms=compute_ms, device=report["device"])
    return adjusted


def refine_geometry(source=None, *, electrodes, dims=None, spacing_mm=None,
                    origin_mm=(0.0, 0.0, 0.0), symmetry="planar", device="cpu",
                    backend="vulkan", tolerance=1e-10, max_cycles=100,
                    units=None, repair=False, out=None, sidecar_dense_sdf=False,
                    surface_supersampling=2, cad_mode="auto", cad_tolerance=None):
    """Cut-cell refine GEM, mesh, or a list of named analytic SDF dictionaries.

    electrodes maps source part name/index to (adjustable_number, voltage).
    Analytic input: [{"part": "left", "shape": {"kind": "plane", ...}}, ...].
    File inputs call the CLI's Rust geometry pipeline, including exact v3
    surfaces and optional dense SDF sidecars. ``out`` writes its PA#/PA0/bases
    and cuts; omitted output uses temporary files and returns an owned Field.
    CAD input requires a wheel built with the ``cad`` Cargo feature.
    """
    analytic = isinstance(source, (list, tuple))
    if not analytic:
        def solve(stem):
            start = perf_counter()
            _command("refine-geometry", source, grid=dims, spacing=spacing_mm,
                     origin=origin_mm, electrode=[
                         f"{part}={number}:{voltage}" for part, (number, voltage) in electrodes.items()],
                     device=device, backend=backend, tolerance=tolerance,
                     max_cycles=max_cycles, out=stem, units=units, repair=repair,
                     sidecar_dense_sdf=sidecar_dense_sdf,
                     surface_supersampling=surface_supersampling,
                     cad_mode=cad_mode, cad_tolerance=cad_tolerance)
            result = load_field(Path(stem).with_suffix(".pa0"))
            result.origin_mm = tuple(origin_mm)
            result.report = dict(compute_ms=(perf_counter() - start) * 1000, device=device)
            return result
        if out is not None:
            return solve(out)
        with TemporaryDirectory(prefix="simrep-geometry-") as directory:
            return solve(Path(directory) / "field")
    if out is not None or sidecar_dense_sdf or cad_mode != "auto" or cad_tolerance is not None:
        raise ValueError("out and sidecar/CAD options require a geometry file")
    settings = dict(solve=dict(device=device, backend=backend,
                              tolerance=tolerance, max_cycles=max_cycles),
                    dims=dims, spacing_mm=spacing_mm, origin_mm=origin_mm,
                    symmetry=symmetry, units=units, repair=repair)
    assignments = [(str(part), int(number), float(voltage))
                   for part, (number, voltage) in electrodes.items()]
    native, native_bases, report = _native.refine_geometry(
        "" if analytic else os.fspath(source),
        _json(source if analytic else []), assignments, _json(settings))
    pa = _pa(native)
    bases = {n: _pa(b).potentials / 10000.0 for n, b in native_bases}
    voltages = {n: v for _, n, v in assignments}
    return Field(pa, bases, voltages, json.loads(report), tuple(origin_mm),
                 np.zeros_like(pa.potentials))


def read_particles(path):
    """Return energy rows: [birth,mass,q,x,y,z,dx,dy,dz,KE,CWF]."""
    return _native.read_particles(os.fspath(path))


def particles_from_energy(position_mm, direction, energy_ev, *, mass_amu=1.0,
                          charge_e=1.0, birth_time_us=0.0, charge_weight=1.0):
    """Broadcast beam columns into native energy rows, without angle ambiguity."""
    position = np.asarray(position_mm, dtype=float)
    direction = np.asarray(direction, dtype=float)
    if position.shape[-1:] != (3,) or direction.shape[-1:] != (3,):
        raise ValueError("position_mm and direction must end in three coordinates")
    columns = np.broadcast_arrays(
        birth_time_us, mass_amu, charge_e, *np.moveaxis(position, -1, 0),
        *np.moveaxis(direction, -1, 0), energy_ev, charge_weight)
    return np.ascontiguousarray(np.stack(columns, axis=-1).reshape(-1, 11))


@dataclass
class FlightResult:
    arrays: dict
    trajectories: list | None
    report: dict

    @property
    def records(self):
        """A NumPy structured array; arrays is the zero-copy columnar form."""
        dtype = [(name, values.dtype) for name, values in self.arrays.items()]
        result = np.empty(len(self.arrays["ion"]), dtype=dtype)
        for name, values in self.arrays.items():
            result[name] = values
        return result


def fly(source, particles, *, voltages=None, voltage_expr=None, parameters=None,
        trajectories=False, sample_every=1, cuts=None, no_cuts=False, run_file=None,
        output=None, allow_time_limit=False, **options):
    """Fly NumPy rows or .ion/.fly2 particles through a PA, PA0, or Field.

    Native options use SIMION physical units. Long reconstruction/integration
    releases the GIL. Failed terminal events raise ValueError, as in the CLI.
    ``charge_mode="beam"`` requires ``beam_current_a`` and defaults to a 1.0
    space-charge step fraction; direct/PIC ensembles default to 0.1.
    ``cuts`` accepts a sidecar path or dictionary; sibling cuts are loaded by
    default, and ``no_cuts=True`` opts out. ``nearest_node_collisions=True``
    selects historical voxel collisions. ``exact_coils=True`` disables CPU maps.
    Magnetic PAs, coils and HS1/SDS support explicit GPU execution; GPU PIC uses
    the shared Poisson solver. Auto selection uses the CLI's measured classes.
    Trajectories support CPU/GPU, ``trajectory_indices`` (ordered zero-based
    indices) and GPU ``max_samples_per_ion=10000`` (overflow raises ValueError).
    ``run_file`` accepts the CLI's JSON path or a Python dictionary. Explicit
    parameters/expressions override it; physics dictionaries have CLI semantics.
    ``output`` writes CLI-format CSV. ``allow_time_limit=True`` treats the
    requested duration as a successful terminal event; the default is False.
    """
    run = {}
    if run_file is not None:
        run = (dict(run_file) if isinstance(run_file, dict) else
               json.loads(Path(run_file).read_text(encoding="utf-8")))
        unknown = run.keys() - {"parameters", "voltage_expr", "physics"}
        if unknown:
            raise ValueError(f"unknown field {next(iter(unknown))!r} in run_file")
    physics = run.get("physics", {})
    unknown = physics.keys() - {"uniform_gauss", "coils", "magnetic_pas", "magnetic_support"}
    if unknown:
        raise ValueError(f"unknown field {next(iter(unknown))!r} in physics")
    if "uniform_gauss" in physics:
        options.setdefault("bfield", physics["uniform_gauss"])
    for key in ("magnetic_pas", "magnetic_support"):
        if key in physics:
            options.setdefault(key, physics[key])
    options["coils"] = list(physics.get("coils", [])) + list(options.get("coils", []))
    parameters = dict(run.get("parameters", {}), **(parameters or {}))
    voltage_expr = dict(run.get("voltage_expr", {}), **{
        str(n): value for n, value in (voltage_expr or {}).items()})
    if cuts is not None and no_cuts:
        raise ValueError("cuts conflicts with no_cuts")
    if cuts is not None and source is None:
        raise ValueError("cuts requires a PA source")
    if options.get("charge_mode") in ("factor", "coulombic") and options.get("space_charge") is None:
        raise ValueError("an explicit factor/coulombic charge_mode requires space_charge")
    if isinstance(source, Field):
        source_field = source
    elif isinstance(source, PA):
        source_field = Field(source)
    elif source is None:
        source_field = Field(PA(np.zeros((1, 1, 1)), np.zeros((1, 1, 1), bool)))
    else:
        source_field = load_field(source, no_cuts=no_cuts or cuts is not None)
    expressions = {int(n): str(v) for n, v in (voltage_expr or {}).items()}
    adjustments = dict(voltages or {})
    adjustments.update({n: 0.0 for n in expressions})
    if adjustments:
        source_field = source_field.fast_adjust(adjustments)
    settings = dict(options, voltage_expr=expressions, parameters=parameters or {},
                    sample_every=sample_every if trajectories else None,
                    has_pa=source is not None)
    if not no_cuts:
        surface = source_field._cuts if cuts is None else cuts
        if surface is not None:
            settings["cuts_json" if isinstance(surface, dict) else "cuts"] = (
                surface if isinstance(surface, dict) else os.fspath(surface))
    for key in ("magnetic_pa",):
        if key in settings:
            settings[key] = [os.fspath(path) for path in settings[key]]
    if settings.get("mbmr") is not None:
        settings["mbmr"] = os.fspath(settings["mbmr"])
    if "magnetic_pas" in settings:
        settings["magnetic_pas"] = [dict(pa, path=os.fspath(pa["path"]))
                                    for pa in settings["magnetic_pas"]]
    if settings.get("magnetic_support") is not None:
        pa = settings["magnetic_support"]
        settings["magnetic_support"] = dict(pa, path=os.fspath(pa["path"]))
    settings.setdefault("origin_mm", source_field.origin_mm)
    dynamic_bases = [(n, source_field.bases[n]) for n in expressions
                     if n in source_field.bases]
    if isinstance(particles, (str, os.PathLike)):
        if os.path.splitext(os.fspath(particles))[1].lower() == ".ion":
            particles = read_particles(particles)
        else:
            particles = _native.read_particles(os.fspath(particles),
                                              *_args(source_field.pa),
                                              _json(settings), dynamic_bases)
        if options.get("particle_format", "energy") != "energy":
            raise ValueError("particle files contain energy rows, not velocities")
    rows = np.ascontiguousarray(particles, dtype=np.float64)
    if rows.ndim != 2:
        raise ValueError("particles must be a two-dimensional NumPy array")
    arrays, paths, report = _native.fly(
        *_args(source_field.pa), rows, dynamic_bases, _json(settings))
    arrays["reason"] = np.asarray(arrays["reason"], dtype=str)
    result = FlightResult(arrays, paths if trajectories else None, json.loads(report))
    if output is not None:
        _write_flight_csv(output, result)
    good = np.isin(arrays["reason"], ["electrode", "outside", "plane"])
    if allow_time_limit:
        good |= arrays["reason"] == "time_limit"
    if len(good) and good[0]:
        good |= arrays["reason"] == "beam_leader_stopped"
    if not good.all():
        raise ValueError("ions did not terminate at an electrode, boundary or detector; "
                         "use allow_time_limit=True for a duration-limited experiment")
    return result


def _command(name, *positionals, **options):
    arguments = name.split() + [os.fspath(value) for value in positionals]
    for key, value in options.items():
        if value is None or value is False:
            continue
        flag = "--" + key.replace("_", "-")
        if value is True:
            arguments.append(flag)
        elif isinstance(value, (tuple, list, np.ndarray)):
            if key == "electrode":
                for item in value:
                    arguments.append(f"{flag}={item}")
            else:
                arguments.append(f"{flag}=" + ",".join(map(str, np.asarray(value).ravel())))
        else:
            arguments.append(f"{flag}={value}")
    _native.command(arguments)


def gem2pa(source, output=None):
    """Compile GEM/Lua to PA#, including requested fractional .pa-surf output."""
    destination = Path(source).with_suffix(".pa#") if output is None else Path(output)
    _command("gem2pa", source, destination)
    return read_pa(destination)


def mesh_import(source, output, *, dims, spacing_mm=(1, 1, 1), origin_mm=(0, 0, 0),
                units=None, transform=(1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0),
                electrodes=None, repair=False, allow_defects=False,
                sdf_prefix=None, voltages=None):
    """Voxelize grouped mesh parts to PA# using CLI topology/units validation.

    ``transform`` is a row-major 3x4 affine matrix after unit conversion.
    ``sdf_prefix`` also writes per-part VTI/JSON signed distances; voltages apply
    to those metadata only. Missing electrode numbers use 1,2,... part order.
    """
    _command("mesh-import", source, output, grid=dims, spacing=spacing_mm,
             origin=origin_mm, units=units, transform=transform, electrodes=electrodes,
             repair=repair, allow_defects=allow_defects, sdf_prefix=sdf_prefix,
             voltages=voltages)
    return read_pa(output)


def export_geometry(source, output, *, origin_mm=(0, 0, 0), electrode=None,
                    angular_segments=64, sweep_degrees=360, extrusion_depth=1):
    """Export PA electrode surfaces to STL/OBJ/glTF/GLB using CLI defaults."""
    _command("export geometry", source, output, origin=origin_mm, electrode=electrode,
             angular_segments=angular_segments, sweep_degrees=sweep_degrees,
             extrusion_depth=extrusion_depth)


def export_grid(source, output, *, origin_mm=(0, 0, 0), no_field=False,
                revolve=False, angular_segments=64, sweep_degrees=360):
    """Export PA potential/mask/field to VTI, or cylindrical revolution to VTS."""
    _command("export grid", source, output, origin=origin_mm, no_field=no_field,
             revolve=revolve, angular_segments=angular_segments, sweep_degrees=sweep_degrees)


def export_isosurface(source, output, *, potential, origin_mm=(0, 0, 0),
                      angular_segments=64, sweep_degrees=360, extrusion_depth=1):
    """Export a potential contour with the CLI's marching/swept mesh pipeline."""
    _command("export isosurface", source, output, origin=origin_mm, potential=potential,
             angular_segments=angular_segments, sweep_degrees=sweep_degrees,
             extrusion_depth=extrusion_depth)


def export_trajectories(source, output):
    """Convert numeric trajectory CSV to VTP polylines or normalized CSV."""
    _command("export trajectories", source, output)


def pa_info(source):
    """Print the same PA header and potential statistics as ``simrep pa-info``."""
    _command("pa-info", source)


def gpu_info(*, backend="auto"):
    """Print selected adapter/features (auto searches DX12 and Vulkan)."""
    _command("gpu-info", backend=backend)


def gpu_smoke(*, backend="auto"):
    """Run and print the CLI's GPU residual checks against CPU f64."""
    _command("gpu-smoke", backend=backend)


def fly_bench(*, ions=10000):
    """Run and print the synthetic CPU einzel benchmark and deterministic checksum."""
    _command("fly-bench", ions=ions)


def refine_bench(*, device="cpu", backend="vulkan", n=128, tolerance=1e-10, max_cycles=100):
    """Run and print the CLI's harmonic 3D multigrid benchmark."""
    _command("refine-bench", device=device, backend=backend, n=n,
             tolerance=tolerance, max_cycles=max_cycles)


def refine_sdf(*, geometry="sphere", n=65, device="cpu", backend="vulkan",
               tolerance=1e-10, max_cycles=100, compare_staircase=False):
    """Run the sphere/cylinder/cylindrical cut-cell validation benchmark."""
    _command("refine-sdf", geometry=geometry, n=n, device=device, backend=backend,
             tolerance=tolerance, max_cycles=max_cycles, compare_staircase=compare_staircase)


def _write_flight_csv(path, result):
    import csv
    names = ["ion", "tof_us", "x_mm", "y_mm", "z_mm", "ke_ev", "reason",
             "electrode_number", "vx_mm_per_us", "vy_mm_per_us", "vz_mm_per_us"]
    with Path(path).open("w", newline="", encoding="utf-8") as handle:
        writer = csv.writer(handle, lineterminator="\n")
        writer.writerow(names)
        for index in range(len(result.arrays["ion"])):
            row = []
            for name in names:
                value = result.arrays[name][index]
                if name == "electrode_number":
                    value = int(value) if value else ""
                elif name == "ion":
                    value = int(value)
                elif name != "reason":
                    value = format(value, ".15e")
                row.append(value)
            writer.writerow(row)
