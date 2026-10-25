import csv
import gc
import json
import os
from pathlib import Path
import subprocess

import numpy as np
import pytest

import simrep


def plates():
    values = np.zeros((1, 7, 9))
    electrodes = np.zeros_like(values, dtype=bool)
    electrodes[:, :, 0] = True
    electrodes[:, :, -1] = True
    values[:, :, 0] = 1
    values[:, :, -1] = 2
    return simrep.PA(values, electrodes)


def test_nonlinear_material_and_fast_adjust_rejection(tmp_path):
    source = plates()
    source.potentials[:, :, 0] = 0
    source.potentials[:, :, -1] = 8
    cells = simrep.PA(np.full((1, 6, 8), 3.0), np.zeros((1, 6, 8), bool))
    cells.write(tmp_path / "cells.pa")
    spec = tmp_path / "nonlinear.json"
    spec.write_text(json.dumps({"nonlinear": {
        "cell_permittivity": "cells.pa",
        "laws": [{"epsilon0": 3, "law": {
            "kind": "saturating_quadratic", "lambda": 0.02, "minimum": 1}}],
    }}))
    field = simrep.refine(source, permittivity=spec, device="auto", tolerance=1e-11)
    expected = np.broadcast_to(np.arange(9), source.potentials.shape)
    np.testing.assert_allclose(field.pa.potentials, expected, atol=1e-8)
    assert field.report["relative_residual"] <= 1e-11
    assert field.report["relative_field_change"] <= 1e-8
    with pytest.raises(ValueError, match="nonlinear permittivity"):
        simrep.refine(source, adjustable=True, permittivity=spec)


@pytest.mark.parametrize("suffix", [".pa", ".pa#", ".pa0"])
@pytest.mark.parametrize("scalable", [None, False, [0.0, 7.0] + [0.0] * 30])
def test_pa_round_trip_and_numpy_ownership(tmp_path, suffix, scalable):
    values = np.arange(60, dtype=float).reshape(3, 4, 5) / 8
    mask = values % 1 == 0
    metadata = dict(mode=-2, spacing_mm=[0.25, 0.5, 0.75], ng=123,
                    max_voltage=100000.0, mirrors=[True, False, False],
                    anti_mirrors=[False, False, False],
                    symmetry="planar", field_type="electrostatic", refinable=True)
    if suffix == ".pa0":
        metadata["pa0"] = dict(reference_voltage=10000.0,
                               electrode_indices=[-1, 0] + [-1] * 30,
                               scalable=scalable)
    path = tmp_path / ("grid" + suffix)
    simrep.write_pa(path, simrep.PA(values, mask, metadata))
    pa = simrep.read_pa(path)
    np.testing.assert_array_equal(pa.potentials, values)
    np.testing.assert_array_equal(pa.electrodes, mask)
    for name, value in metadata.items():
        assert pa.metadata[name] == value
    output = pa.potentials
    assert output.flags.c_contiguous
    assert output.base is not None
    del pa
    gc.collect()
    np.testing.assert_array_equal(output, values)
    simrep.write_pa(path, simrep.PA(output[:, :, ::-1], mask[:, :, ::-1], metadata))
    np.testing.assert_array_equal(simrep.read_pa(path).potentials, values[:, :, ::-1])


def test_standard_magnetic_and_antimirror_headers(tmp_path):
    pa = simrep.PA(np.zeros((5, 5)), np.zeros((5, 5), bool),
                   dict(mode=-1, field_type="magnetic", ng=250,
                        mirrors=[True, True, False],
                        anti_mirrors=[True, False, False], refinable=False))
    path = tmp_path / "magnetic.pa"
    pa.write(path)
    output = simrep.read_pa(path)
    assert output.metadata["mode"] == -1
    assert output.metadata["field_type"] == "magnetic"
    assert output.metadata["ng"] == 250
    assert output.metadata["anti_mirrors"] == [True, False, False]
    assert not output.metadata["refinable"]


def test_refine_and_fast_adjust_disk_equivalence(tmp_path):
    field = simrep.refine(plates(), adjustable=True, tolerance=1e-12)
    field = field.fast_adjust({1: 3.0, 2: 11.0})
    expected = np.broadcast_to(np.arange(9) + 3.0, field.pa.potentials.shape)
    np.testing.assert_allclose(field.pa.potentials, expected, atol=1e-9)
    path = tmp_path / "plates.pa0"
    field.write(path)
    loaded = simrep.load_field(path)
    adjusted = simrep.fast_adjust(path, {2: 19.0})
    np.testing.assert_allclose(loaded.pa.potentials, field.pa.potentials, atol=1e-9)
    np.testing.assert_allclose(adjusted.pa.potentials,
                               field.fast_adjust({2: 19.0}).pa.potentials, atol=1e-9)
    assert adjusted.voltages[1] == 3.0
    with pytest.raises(ValueError, match="no fast-adjust basis"):
        adjusted.fast_adjust({3: 0.0})


@pytest.mark.parametrize("symmetry", ["planar", "cylindrical"])
def test_refine_ordinary_pa_and_pasharp_path(tmp_path, symmetry):
    pa = plates()
    pa.metadata["symmetry"] = symmetry
    pa.metadata["mirrors"] = [False, symmetry == "cylindrical", False]
    pa.metadata["spacing_mm"] = [0.3, 0.7, 1.0]
    pa.potentials[:, :, 0] = -2
    pa.potentials[:, :, -1] = 6
    result = simrep.refine(pa)
    # CLI auto keeps vacuum refinement on CPU; the shared policy is coordinated
    # with the dielectric worker's change to refine.rs.
    automatic = simrep.refine(pa, device="auto")
    assert automatic.report["device"] == "cpu"
    expected = np.broadcast_to(np.arange(9) - 2.0, pa.potentials.shape)
    np.testing.assert_allclose(result.pa.potentials, expected, atol=1e-8)
    assert result.report["compute_ms"] >= 0
    path = tmp_path / "numbered.pa#"
    source = plates()
    source.metadata["symmetry"] = symmetry
    source.metadata["mirrors"] = [False, symmetry == "cylindrical", False]
    source.write(path)
    field = simrep.refine(path)
    assert set(field.bases) == {1, 2}


def test_fast_scalable_background_preserved(tmp_path):
    pa = plates()
    pa.potentials[:, :, 0] = -2.5
    field = simrep.refine(pa, adjustable=True)
    assert field.voltages == {0: -2.5, 2: 0.0}
    assert field.pa.potentials[0, 3, 0] == pytest.approx(-2.5)
    field.write(tmp_path / "scalable.pa0")
    loaded = simrep.load_field(tmp_path / "scalable.pa0")
    np.testing.assert_allclose(loaded.pa.potentials, field.pa.potentials, atol=1e-9)
    np.testing.assert_allclose(loaded.fast_adjust({0: -5.0, 2: 7.0}).pa.potentials,
                               field.fast_adjust({0: -5.0, 2: 7.0}).pa.potentials,
                               atol=1e-9)


@pytest.mark.parametrize("symmetry", ["planar", "cylindrical"])
def test_dielectric_cell_pa_and_adjustable_basis(tmp_path, symmetry):
    pa = plates()
    pa.metadata["symmetry"] = symmetry
    pa.metadata["mirrors"] = [False, symmetry == "cylindrical", False]
    relative = np.ones((1, 6, 8))
    relative[:, :, 4:] = 3.0
    material = tmp_path / "epsilon.pa"
    simrep.PA(relative, np.zeros_like(relative, bool),
              dict(symmetry=symmetry,
                   mirrors=[False, symmetry == "cylindrical", False])).write(material)
    field = simrep.refine(pa, permittivity=material, adjustable=True, device="auto",
                          tolerance=1e-12).fast_adjust({1: 3.0, 2: 11.0})
    x = np.arange(9) / 8
    expected = 3 + 8 * (np.minimum(x, 0.5)
                        + np.maximum(x - 0.5, 0) / 3) / (2 / 3)
    np.testing.assert_allclose(field.pa.potentials,
                               np.broadcast_to(expected, pa.potentials.shape),
                               atol=1e-8)
    assert field.report["device"] == "cpu"


def geometry_planes():
    return [
        dict(part="left", shape=dict(kind="plane", normal=[1, 0, 0], offset=0)),
        dict(part="right", shape=dict(kind="plane", normal=[-1, 0, 0], offset=-8)),
    ]


def test_analytic_cutcell_geometry(tmp_path):
    field = simrep.refine_geometry(geometry_planes(), dims=(9, 7, 1),
                                   electrodes={"left": (1, 3), "right": (2, 11)})
    np.testing.assert_allclose(field.pa.potentials,
                               np.broadcast_to(np.arange(9) + 3, (1, 7, 9)), atol=1e-8)
    assert field.report["compute_ms"] >= 0
    field.write(tmp_path / "analytic.pa0")
    assert simrep.read_pa(tmp_path / "analytic.pa0").metadata["pa0"]
    sphere = dict(kind="sphere", center=[4, 3, 0], radius=1)
    block = dict(kind="box", center=[4, 3, 0], half_extent=[0.5, 0.5, 1])
    shape = dict(kind="difference", a=sphere, b=block)
    # Exercise native CSG validation, including failure before an invalid solve.
    with pytest.raises(ValueError, match="no inside/on-surface node"):
        simrep.refine_geometry([dict(part="hollow", shape=shape)],
                               dims=(2, 2, 1), electrodes={"hollow": (1, 0)})


def test_gem_cutcell_geometry(tmp_path):
    path = tmp_path / "plates.gem"
    path.write_text("""pa_define(9,7,1,planar,non_mirrored)
e(1) { fill { within { box(-2,-2,0,8) } } }
e(2) { fill { within { box(8,-2,10,8) } } }
""")
    field = simrep.refine_geometry(path, electrodes={1: (1, 3), 2: (2, 11)})
    np.testing.assert_allclose(field.pa.potentials,
                               np.broadcast_to(np.arange(9) + 3, (1, 7, 9)), atol=1e-7)


def test_analytic_numeric_names_do_not_alias_part_indices():
    shapes = geometry_planes()
    shapes[0]["part"] = "1"
    shapes[1]["part"] = "2"
    field = simrep.refine_geometry(shapes, dims=(9, 7, 1),
                                   electrodes={1: (1, 3), 2: (2, 11)})
    np.testing.assert_allclose(field.pa.potentials,
                               np.broadcast_to(np.arange(9) + 3, (1, 7, 9)), atol=1e-8)


def test_mesh_cutcell_geometry(tmp_path):
    path = tmp_path / "cube.obj"
    path.write_text("""o conductor
v 1 1 1
v 3 1 1
v 3 3 1
v 1 3 1
v 1 1 3
v 3 1 3
v 3 3 3
v 1 3 3
f 1 4 3 2
f 5 6 7 8
f 1 2 6 5
f 2 3 7 6
f 3 4 8 7
f 4 1 5 8
""")
    field = simrep.refine_geometry(path, dims=(5, 5, 5), repair=True,
                                   electrodes={"conductor": (1, 9)})
    np.testing.assert_allclose(field.pa.potentials, 9, atol=1e-8)


def beam():
    return simrep.particles_from_energy([[1, 2, 0], [1, 3, 0]],
                                        [1, 0, 0], [2, 3], mass_amu=39)


def test_fly_arrays_trajectories_and_velocity_input():
    pa = simrep.PA(np.zeros((5, 7)), np.zeros((5, 7), bool))
    result = simrep.fly(pa, beam(), device="cpu", plane=[1, 0, 0, 4],
                        duration_us=5, trajectories=True)
    assert result.arrays["reason"].tolist() == ["plane", "plane"]
    np.testing.assert_allclose(result.records["x_mm"], 4, atol=1e-12)
    assert len(result.trajectories) == 2
    for trajectory in result.trajectories:
        assert trajectory.shape[1] == 7
        assert trajectory[0, 0] == 0
        assert trajectory[-1, 1] == pytest.approx(4)
    velocity = np.array([[39, 1, 1, 2, 0, 2, 0, 0, 0]], dtype=float)
    r = simrep.fly(pa, velocity, particle_format="velocity", device="cpu",
                   duration_us=5, plane=[1, 0, 0, 4])
    assert r.arrays["tof_us"][0] == pytest.approx(1.5, abs=1e-12)
    assert r.report["compute_ms"] >= 0


def test_voltage_expression_matches_static_basis():
    field = simrep.refine(plates(), adjustable=True)
    options = dict(device="cpu", duration_us=0.01, max_step_us=0.001, allow_time_limit=True)
    static = simrep.fly(field, beam(), voltages={1: 0, 2: 7}, **options)
    dynamic = simrep.fly(field, beam(), voltage_expr={2: "v"},
                         parameters={"v": 7}, **options)
    for name in ["tof_us", "x_mm", "y_mm", "z_mm", "ke_ev"]:
        np.testing.assert_allclose(dynamic.arrays[name], static.arrays[name],
                                   rtol=1e-12, atol=1e-12)


def test_cpu_coil_map_default_and_exact_opt_out():
    options = dict(device="cpu", duration_us=0.01, allow_time_limit=True,
                   bounds=[[-150, -1, -1], [150, 1, 1]],
                   coils=[dict(kind="solenoid", first_mm=[-50, 0, 0],
                               last_mm=[50, 0, 0], radius_mm=10,
                               current_a=0.7958, turns=100)])
    ions = simrep.particles_from_energy([[0, 0.1, 0], [1, -0.2, 0.1]],
                                        [1, 0, 0], [2, 3], mass_amu=39)
    mapped = simrep.fly(None, ions, **options)
    exact = simrep.fly(None, ions, exact_coils=True, **options)
    assert len(mapped.report["coil_map_calibration"]) == 3
    assert all(c["accepted"] for c in mapped.report["coil_map_calibration"])
    assert exact.report["coil_map_calibration"] == []
    assert mapped.arrays["reason"].tolist() == ["time_limit", "time_limit"]
    np.testing.assert_array_equal(mapped.arrays["reason"], exact.arrays["reason"])
    for name in ["tof_us", "x_mm", "y_mm", "z_mm",
                 "vx_mm_per_us", "vy_mm_per_us", "vz_mm_per_us"]:
        np.testing.assert_allclose(mapped.arrays[name], exact.arrays[name],
                                   rtol=0, atol=2e-9)


@pytest.mark.parametrize("space_charge", ["direct", "pic"])
def test_cpu_physics_flags(space_charge):
    options = dict(device="cpu", integrator="boris", duration_us=0.005, allow_time_limit=True,
                   initial_step_us=0.001, max_step_us=0.001,
                   bfield=[0, 0, 1], space_charge=space_charge, charge_amount=0,
                   bounds=[[-10, -10, -10], [10, 10, 10]], pic_dims=[5, 5, 5])
    r = simrep.fly(None, beam(), **options)
    assert np.isfinite(r.arrays["ke_ev"]).all()
    assert r.arrays["reason"].tolist() == ["time_limit", "time_limit"]
    r = simrep.fly(None, beam(), device="cpu", duration_us=0.005,
                   collision="hs1", pressure_pa=0, seed=7, allow_time_limit=True)
    assert r.report["relativistic"] is False


def test_clear_errors():
    with pytest.raises(ValueError, match="shape"):
        simrep.PA(np.zeros((2, 2)), np.zeros((3, 3), bool))
    with pytest.raises(ValueError, match="non-refinable"):
        simrep.refine(simrep.PA(np.zeros((3, 3)), np.ones((3, 3), bool),
                                {"refinable": False}))
    with pytest.raises(ValueError, match="requires device='cpu'"):
        simrep.fly(None, beam(), device="gpu", charge_mode="beam", beam_current_a=1e-9)
    with pytest.raises(ValueError, match="require device='cpu'"):
        simrep.fly(None, beam(), device="gpu",
                   transparent_faces=[False, True, False, False, False, False])
    with pytest.raises(ValueError, match="unknown field"):
        simrep.fly(None, beam(), typo_option=1)
    with pytest.raises(ValueError, match="positive"):
        invalid = beam()
        invalid[:, 1] = -1
        simrep.fly(None, invalid)
    with pytest.raises(ValueError, match="SDS requires mbmr"):
        simrep.fly(None, beam(), device="cpu", collision="sds")
    with pytest.raises(ValueError, match="limits"):
        simrep.fly(None, beam(), device="cpu", max_steps=0)


@pytest.mark.parametrize("extension", [".ion", ".fly2"])
def test_fly_equals_cli_output(tmp_path, extension):
    root = Path(__file__).resolve().parents[3]
    binary = Path(os.environ.get("SIMREP_BIN", root / "target/release/simrep"))
    assert binary.is_file(), f"build CLI first: cargo build --release -p simrep-cli ({binary})"
    path = tmp_path / "uniform.pa"
    values = np.broadcast_to(-2.0 * np.arange(7), (1, 5, 7)).copy()
    simrep.PA(values, np.zeros_like(values, bool)).write(path)
    particles = tmp_path / ("particles" + extension)
    if extension == ".ion":
        particles.write_text("0,39,1,1,2,0,0,0,2,1\n0.1,39,1,1,3,0,0,0,3,2\n")
    else:
        particles.write_text("""particles { coordinates = 0,
standard_beam { n = 1, tob = 0, mass = 39, charge = 1,
position = vector(1,2,0), direction = vector(1,0,0), ke = 2, cwf = 1 },
standard_beam { n = 1, tob = 0.1, mass = 39, charge = 1,
position = vector(1,3,0), direction = vector(1,0,0), ke = 3, cwf = 2 } }
""")
    output = tmp_path / "cli.csv"
    command = [str(binary), "fly", "--pa", str(path), "--particles", str(particles),
               "--device", "cpu", "--plane", "1,0,0,4", "--duration-us", "5",
               "--max-step-us", "0.03", "--tolerance", "1e-10", "--output", str(output)]
    subprocess.run(command, check=True, capture_output=True, text=True)
    result = simrep.fly(path, particles, device="cpu", plane=[1, 0, 0, 4],
                        duration_us=5, max_step_us=0.03, tolerance=1e-10)
    with output.open() as handle:
        rows = list(csv.DictReader(handle))
    assert result.arrays["reason"].tolist() == [r["reason"] for r in rows]
    for name in ["ion", "tof_us", "x_mm", "y_mm", "z_mm", "ke_ev"]:
        np.testing.assert_allclose(result.arrays[name], [float(r[name]) for r in rows],
                                   rtol=2e-14, atol=2e-14)
    parsed = simrep.read_particles(particles)
    assert parsed.shape == (2, 11)
    np.testing.assert_array_equal(parsed[:, 10], [1, 2])
