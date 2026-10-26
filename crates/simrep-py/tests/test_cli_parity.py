"""End-to-end comparisons with the real CLI, using tiny generated fixtures."""

import csv
import json
import os
from pathlib import Path
import subprocess

import numpy as np
import pytest

import simrep


@pytest.fixture(scope="session")
def cli():
    root = Path(__file__).resolve().parents[3]
    binary = Path(os.environ.get("SIMREP_BIN", root / "target/release/simrep"))
    assert binary.is_file(), f"build CLI first: cargo build --release -p simrep-cli ({binary})"

    def run(*arguments, check=True):
        return subprocess.run([str(binary), *map(str, arguments)], check=check,
                              capture_output=True, text=True, stdin=subprocess.DEVNULL,
                              timeout=900)
    return run


@pytest.fixture(scope="session")
def gpu(cli):
    probe = cli("gpu-info", "--backend", "vulkan", check=False)
    if probe.returncode:
        pytest.skip(f"Vulkan adapter unavailable: {probe.stderr}")
    return probe.stdout


@pytest.fixture(scope="session")
def hardware_gpu(gpu):
    if "Device type: Cpu" in gpu:
        pytest.skip(f"GPU numeric parity requires a hardware GPU; {gpu.splitlines()[0]}")
    return gpu


@pytest.fixture
def particles(tmp_path):
    path = tmp_path / "ions.ion"
    path.write_text("0,39,1,1,2,3,0,0,2,1\n0,39,1,1,4,3,0,0,3,2\n")
    return path


def compare_flight(cli, tmp_path, source, particles, python_options, *arguments):
    output = tmp_path / "cli.csv"
    command = ["fly", "--particles", particles, "--output", output,
               "--allow-time-limit", *arguments]
    if source is not None:
        command += ["--pa", source]
    cli(*command)
    result = simrep.fly(source, particles, allow_time_limit=True, **python_options)
    rows = list(csv.DictReader(output.open()))
    np.testing.assert_array_equal(result.arrays["reason"], [row["reason"] for row in rows])
    for name in rows[0]:
        if name == "reason":
            continue
        expected = [float(row[name]) if row[name] else 0 for row in rows]
        np.testing.assert_allclose(result.arrays[name], expected, rtol=3e-14, atol=3e-14)
    return result


def test_explicit_gpu_refuses_software_adapter_in_cli_and_python(cli, gpu, tmp_path, particles):
    if "Device type: Cpu" not in gpu:
        pytest.skip("Explicit GPU software-adapter refusal requires a software adapter")
    output = tmp_path / "refused.csv"
    completed = cli("fly", "--particles", particles, "--output", output,
                    "--device=gpu", "--backend=vulkan", "--duration-us=0.005",
                    "--allow-time-limit", check=False)
    assert completed.returncode != 0
    assert not output.exists()
    with pytest.raises(ValueError) as error:
        simrep.fly(None, particles, device="gpu", backend="vulkan",
                   duration_us=0.005, allow_time_limit=True)
    for message in (completed.stderr, str(error.value)):
        assert "requires a hardware GPU" in message
        assert "is a CPU/software adapter" in message
    assert "use --device cpu" in completed.stderr
    assert "use device='cpu'" in str(error.value)


@pytest.mark.parametrize("fraction", [None, 0.25])
@pytest.mark.parametrize("device", ["cpu", "auto"])
def test_beam_current_and_optional_step_fraction(cli, tmp_path, particles, fraction, device):
    options = dict(device=device, charge_mode="beam", beam_current_a=1e-9,
                   duration_us=0.01, max_step_us=0.002, tolerance=1e-10,
                   bounds=[[0, 0, 0], [8, 8, 8]], charge_softening_mm=0.05)
    arguments = ["--device", device, "--charge-mode", "beam", "--beam-current-a", "1e-9",
                 "--duration-us", "0.01", "--max-step-us", "0.002", "--tolerance", "1e-10",
                 "--bounds=0,0,0,8,8,8", "--charge-softening-mm", "0.05"]
    if fraction is not None:
        options["space_charge_step_fraction"] = fraction
        arguments += ["--space-charge-step-fraction", str(fraction)]
    compare_flight(cli, tmp_path, None, particles, options, *arguments)


@pytest.mark.parametrize("method", ["direct", "pic"])
@pytest.mark.parametrize("mode,amount", [("factor", 1000), ("coulombic", 1e-16)])
@pytest.mark.parametrize("device", ["cpu", "auto"])
def test_ensemble_charge_modes_and_default_fraction(cli, tmp_path, particles, method,
                                                   mode, amount, device):
    compare_flight(
        cli, tmp_path, None, particles,
        dict(device=device, space_charge=method, charge_mode=mode, charge_amount=amount,
             duration_us=0.005, max_step_us=0.001, tolerance=1e-10,
             bounds=[[0, 0, 0], [8, 8, 8]], pic_dims=[5, 5, 5]),
        "--device", device, "--space-charge", method, "--charge-mode", mode,
        "--charge-amount", str(amount), "--duration-us", "0.005",
        "--max-step-us", "0.001", "--tolerance", "1e-10",
        "--bounds=0,0,0,8,8,8", "--pic-dims=5,5,5")


def test_default_step_ceiling_matches_cli(cli, tmp_path):
    # Neither side passes a step ceiling or tolerance: both resolve the shared
    # automatic defaults, including the four-cell spatial ceiling.
    source = tmp_path / "uniform.pa"
    values = np.broadcast_to(-2.0 * np.arange(7), (1, 5, 7)).copy()
    simrep.PA(values, np.zeros_like(values, bool)).write(source)
    particles = tmp_path / "ions.ion"
    particles.write_text("0,39,1,1,2,0,0,0,2,1\n0.1,39,1,1,3,0,0,0,3,2\n")
    result = compare_flight(cli, tmp_path, source, particles,
                            dict(device="cpu", duration_us=5),
                            "--device=cpu", "--duration-us=5")
    four = simrep.fly(source, particles, device="cpu", duration_us=5,
                      max_grid_cells=4)
    one = simrep.fly(source, particles, device="cpu", duration_us=5,
                     max_grid_cells=1)
    np.testing.assert_array_equal(result.arrays["accepted_steps"],
                                  four.arrays["accepted_steps"])
    for name in ["tof_us", "x_mm", "y_mm", "z_mm"]:
        np.testing.assert_array_equal(result.arrays[name], four.arrays[name])
    assert not np.array_equal(result.arrays["accepted_steps"],
                              one.arrays["accepted_steps"])


def test_gpu_pic_matches_cli(cli, hardware_gpu, tmp_path, particles):
    result = compare_flight(
        cli, tmp_path, None, particles,
        dict(device="gpu", space_charge="pic", charge_amount=1000, duration_us=0.002,
             max_step_us=0.001, tolerance=1e-9, bounds=[[0, 0, 0], [8, 8, 8]],
             pic_dims=[5, 5, 5], space_charge_step_fraction=0.2),
        "--device=gpu", "--space-charge=pic", "--charge-amount=1000",
        "--duration-us=0.002", "--max-step-us=0.001", "--tolerance=1e-9",
        "--bounds=0,0,0,8,8,8", "--pic-dims=5,5,5", "--space-charge-step-fraction=0.2")
    assert result.report["device"] == "gpu"


@pytest.mark.parametrize("device", ["cpu", "gpu"])
@pytest.mark.parametrize("exact", [False, True])
def test_magnetic_pa_coils_and_exact_routing(cli, request, tmp_path, particles, device, exact):
    if device == "gpu":
        request.getfixturevalue("hardware_gpu")
    magnetic = tmp_path / "magnetic.pa"
    values = np.broadcast_to(-np.arange(9), (9, 9, 9)).copy().astype(float)
    simrep.PA(values, np.zeros_like(values, bool),
              dict(field_type="magnetic", ng=1)).write(magnetic)
    coils = [dict(kind="loop", center_mm=[0, 0, 0], normal=[1, 0, 0],
                  radius_mm=20, current_a=0.1)]
    coil_path = tmp_path / "coils.json"
    coil_path.write_text(json.dumps(coils))
    arguments = ["--device", device, "--magnetic-pa", magnetic, "--bfield=0,0,1",
                 "--bfield-coil", coil_path, "--duration-us=0.005",
                 "--max-step-us=0.001", "--tolerance=1e-10"]
    if exact:
        arguments.append("--exact-coils")
    result = compare_flight(
        cli, tmp_path, None, particles,
        dict(device=device, magnetic_pa=[magnetic], bfield=[0, 0, 1], coils=coils,
             exact_coils=exact, duration_us=0.005, max_step_us=0.001, tolerance=1e-10),
        *arguments)
    assert result.report["device"] == device


@pytest.mark.parametrize("kind", ["hs1", "sds"])
def test_gpu_collision_models_match_cli(cli, hardware_gpu, tmp_path, particles, kind):
    options = dict(device="gpu", collision=kind, seed=7, duration_us=0.005,
                   max_step_us=0.001, tolerance=1e-10, pressure_pa=530)
    arguments = ["--device=gpu", "--collision", kind, "--seed=7", "--duration-us=0.005",
                 "--max-step-us=0.001", "--tolerance=1e-10", "--pressure-pa=530"]
    if kind == "sds":
        # Synthetic monotone ICDFs exercise transport without bundling SIMION data.
        statistics = tmp_path / "mbmr.dat"
        statistics.write_text(" ".join(map(str, list(range(1002)) * 5)))
        options.update(mbmr=statistics, diffusion=False)
        arguments += ["--mbmr", statistics, "--no-diffusion"]
    result = compare_flight(cli, tmp_path, None, particles, options, *arguments)
    assert result.report["relativistic"] is False
    assert np.any(np.abs(result.arrays["ke_ev"] - [2, 3]) > 1e-6)


@pytest.fixture
def gem(tmp_path):
    source = tmp_path / "plates.gem"
    source.write_text("""# local width=7.25
pa_define(9,7,1,planar,non_mirrored,surface=fractional)
e(1) { fill { within { box(-2,-2,0,8) } } }
e(2) { fill { within { box($(width),-2,10,8) } } }
""")
    return source


def test_gem2pa_lua_and_default_output(cli, tmp_path, gem):
    expected = tmp_path / "cli.pa#"
    cli("gem2pa", gem, expected)
    result = simrep.gem2pa(gem)
    actual = gem.with_suffix(".pa#")
    assert actual.read_bytes() == expected.read_bytes()
    assert actual.with_suffix(".pa-surf").read_bytes() == expected.with_suffix(".pa-surf").read_bytes()
    np.testing.assert_array_equal(result.potentials, simrep.read_pa(expected).potentials)


@pytest.mark.parametrize("dense", [False, True])
def test_refine_geometry_sidecar_and_all_options(cli, tmp_path, gem, dense):
    expected, actual = tmp_path / "cli", tmp_path / "python"
    arguments = ["refine-geometry", gem, "--out", expected, "--grid=9,7,1",
                 "--spacing=1,1,1", "--origin=0,0,0", "--electrode=1=1:3",
                 "--electrode=2=2:11", "--surface-supersampling=3",
                 "--tolerance=1e-11", "--max-cycles=120", "--cad-mode=auto"]
    if dense:
        arguments.append("--sidecar-dense-sdf")
    cli(*arguments)
    field = simrep.refine_geometry(
        gem, electrodes={1: (1, 3), 2: (2, 11)}, dims=[9, 7, 1], spacing_mm=[1, 1, 1],
        out=actual, sidecar_dense_sdf=dense, surface_supersampling=3,
        tolerance=1e-11, max_cycles=120)
    for suffix in [".pa0", ".pa1", ".pa2", ".pa#", ".cuts.json"]:
        assert actual.with_suffix(suffix).read_bytes() == expected.with_suffix(suffix).read_bytes()
    assert field._cuts["version"] == 3
    field.write(tmp_path / "saved.pa0")
    assert (tmp_path / "saved.cuts.json").is_file()


@pytest.mark.parametrize("nearest", [False, True])
def test_cuts_and_nearest_node_collisions(cli, tmp_path, particles, gem, nearest):
    gem.write_text("""pa_define(9,7,1,planar,non_mirrored)
e(1) { fill { within { box(-2,-2,0,8) } } }
e(2) { fill { within { box(6,-2,10,8) } } }
""")
    pa = tmp_path / "field.pa0"
    simrep.refine_geometry(gem, electrodes={1: (1, 0), 2: (2, 0)}, out=tmp_path / "field")
    options = dict(device="cpu", duration_us=3, max_step_us=0.01,
                   tolerance=1e-10, nearest_node_collisions=nearest)
    arguments = ["--device=cpu", "--duration-us=3", "--max-step-us=0.01",
                 "--tolerance=1e-10"]
    if nearest:
        arguments += ["--nearest-node-collisions", "--no-cuts"]
        options["no_cuts"] = True
    else:
        arguments += ["--cuts", pa.with_suffix(".cuts.json")]
        options["cuts"] = json.loads(pa.with_suffix(".cuts.json").read_text())
    # The flat PA is singleton z, so use planar particles for this fixture.
    particles.write_text("0,39,1,1,2,0,0,0,2,1\n0,39,1,1,4,0,0,0,3,2\n")
    result = compare_flight(cli, tmp_path, pa, particles, options, *arguments)
    assert result.arrays["reason"].tolist() == ["electrode", "electrode"]
    np.testing.assert_allclose(result.arrays["x_mm"], 5.5 if nearest else 6, atol=1e-9)
    np.testing.assert_array_equal(result.arrays["electrode_number"], 0 if nearest else 2)


def test_no_cuts_bypasses_malformed_sibling(cli, tmp_path, particles):
    source = tmp_path / "blank.pa"
    simrep.PA(np.zeros((9, 9, 9)), np.zeros((9, 9, 9), bool)).write(source)
    source.with_suffix(".cuts.json").write_text("not JSON")
    compare_flight(
        cli, tmp_path, source, particles,
        dict(device="cpu", duration_us=0.005, no_cuts=True),
        "--device=cpu", "--duration-us=0.005", "--no-cuts")


@pytest.mark.parametrize("device", ["cpu", "gpu"])
def test_sampled_trajectories_preserve_cli_terminal_results(cli, request, tmp_path, particles, device):
    if device == "gpu":
        request.getfixturevalue("hardware_gpu")
    result = compare_flight(
        cli, tmp_path, None, particles,
        dict(device=device, duration_us=0.005, max_step_us=0.001, tolerance=1e-10,
             trajectories=True, sample_every=2, trajectory_indices=[1], max_samples_per_ion=20),
        "--device", device, "--duration-us=0.005", "--max-step-us=0.001", "--tolerance=1e-10")
    assert result.report["trajectory_indices"] == [1]
    assert len(result.trajectories) == 1
    samples = result.trajectories[0]
    np.testing.assert_array_equal(samples[-1, 1:4],
                                  [result.arrays[name][1] for name in ["x_mm", "y_mm", "z_mm"]])
    assert samples[0, 0] == 0
    assert samples[-1, 0] == result.arrays["time_us"][1]


def test_run_file_placed_magnetic_pa_and_support(cli, tmp_path, particles):
    path = tmp_path / "b.pa"
    values = np.broadcast_to(-np.arange(9), (9, 9, 9)).copy().astype(float)
    simrep.PA(values, np.zeros_like(values, bool), dict(field_type="magnetic")).write(path)
    placement = dict(path=str(path), origin_mm=[0, 0, 0], scale=1, ng=2,
                     polarity_flip_axis=0, angular_sector=[0, 0, 360])
    run = dict(physics=dict(magnetic_pas=[placement], magnetic_support=placement))
    run_path = tmp_path / "run.json"
    run_path.write_text(json.dumps(run))
    compare_flight(
        cli, tmp_path, None, particles,
        dict(device="cpu", run_file=run, magnetic_pa=[path], duration_us=0.005, tolerance=1e-10),
        "--device=cpu", "--run-file", run_path, "--magnetic-pa", path,
        "--duration-us=0.005", "--tolerance=1e-10")


def test_flight_csv_output_matches_cli(cli, tmp_path, particles):
    python_csv = tmp_path / "python.csv"
    compare_flight(
        cli, tmp_path, None, particles,
        dict(device="cpu", duration_us=0.005, output=python_csv),
        "--device=cpu", "--duration-us=0.005")
    expected = list(csv.DictReader((tmp_path / "cli.csv").open()))
    actual = list(csv.DictReader(python_csv.open()))
    for left, right in zip(actual, expected, strict=True):
        for name in left:
            if name in ("reason", "electrode_number"):
                assert left[name] == right[name]
            else:
                assert float(left[name]) == float(right[name])


@pytest.mark.parametrize("nonlinear", [False, True])
def test_refine_material_json_and_auto_match_cli(cli, tmp_path, nonlinear):
    values = np.zeros((1, 7, 9))
    fixed = np.zeros_like(values, bool)
    fixed[:, :, 0] = fixed[:, :, -1] = True
    values[:, :, -1] = 8
    pa = simrep.PA(values, fixed)
    cells = tmp_path / "cells.pa"
    simrep.PA(np.full((1, 6, 8), 3.0), np.zeros((1, 6, 8), bool)).write(cells)
    material = cells
    if nonlinear:
        material = tmp_path / "law.json"
        material.write_text(json.dumps({"nonlinear": {
            "cell_permittivity": "cells.pa", "laws": [{"epsilon0": 3, "law": {
                "kind": "saturating_quadratic", "lambda": 0.02, "minimum": 1}}]}}))
    source = tmp_path / "cli.pa"
    pa.write(source)
    cli("refine", source, "--permittivity", material, "--tolerance=1e-11")
    result = simrep.refine(pa, permittivity=material, tolerance=1e-11)
    np.testing.assert_array_equal(result.pa.potentials, simrep.read_pa(source).potentials)
    assert result.report["device"] == "cpu"


def test_cad_feature_gate_matches_cli(cli, tmp_path):
    path = tmp_path / "not-cad.step"
    path.write_text("not a CAD file")
    expected = cli("refine-geometry", path, "--grid=3,3,3", "--out", tmp_path / "cli",
                   "--electrode=0=1:0", "--cad-mode=exact", "--cad-tolerance=0.01", check=False)
    assert expected.returncode
    with pytest.raises(ValueError):
        simrep.refine_geometry(path, dims=[3, 3, 3], electrodes={0: (1, 0)},
                               cad_mode="exact", cad_tolerance=0.01)


def test_default_time_limit_validation_matches_cli(cli, particles):
    expected = cli("fly", "--particles", particles, "--device=cpu", "--duration-us=0.001",
                   check=False)
    assert expected.returncode
    with pytest.raises(ValueError, match="allow_time_limit"):
        simrep.fly(None, particles, device="cpu", duration_us=0.001)


def test_refine_input_extension_validation_matches_cli(cli, tmp_path):
    source = tmp_path / "input.pa0"
    simrep.PA(np.zeros((3, 3)), np.zeros((3, 3), bool)).write(source)
    assert cli("refine", source, check=False).returncode
    with pytest.raises(ValueError, match="extension"):
        simrep.refine(source)


@pytest.fixture
def mesh(tmp_path):
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
    return path


def test_mesh_import_options_and_sdf_outputs(cli, tmp_path, mesh):
    actual, expected = tmp_path / "python.pa#", tmp_path / "cli.pa#"
    python_sdf, cli_sdf = tmp_path / "python-sdf", tmp_path / "cli-sdf"
    simrep.mesh_import(mesh, actual, dims=(5, 5, 5), units="mm", electrodes=[3],
                       repair=True, sdf_prefix=python_sdf, voltages=[7])
    cli("mesh-import", mesh, expected, "--grid=5,5,5", "--units=mm", "--electrodes=3",
        "--repair", "--sdf-prefix", cli_sdf, "--voltages=7")
    assert actual.read_bytes() == expected.read_bytes()
    for suffix in ["-0.vti", "-0.json"]:
        assert Path(str(python_sdf) + suffix).read_bytes() == Path(str(cli_sdf) + suffix).read_bytes()


@pytest.mark.parametrize("kind,extension", [("geometry", "obj"), ("grid", "vti"),
                                          ("isosurface", "obj"), ("trajectories", "vtp")])
def test_export_commands_match_cli_bytes(cli, tmp_path, capfd, kind, extension):
    source = tmp_path / "input.pa"
    values = np.broadcast_to(np.arange(9), (1, 7, 9)).copy().astype(float)
    fixed = np.zeros_like(values, bool)
    fixed[:, :, 0] = True
    simrep.PA(values, fixed).write(source)
    actual, expected = tmp_path / ("python." + extension), tmp_path / ("cli." + extension)
    options, arguments = {}, []
    if kind == "isosurface":
        options["potential"] = 4
        arguments = ["--potential=4"]
    if kind == "trajectories":
        source = tmp_path / "input.csv"
        source.write_text("trajectory_id,time_us,x_mm,y_mm,z_mm,vx_mm_per_us,vy_mm_per_us,vz_mm_per_us\n"
                          "1,0,0,0,0,1,0,0\n1,1,1,0,0,1,0,0\n")
    summary = getattr(simrep, "export_" + kind)(source, actual, **options)
    assert summary["output_paths"] == [str(actual)]
    assert summary["schema_version"] == 1
    assert capfd.readouterr().out == ""
    cli("export", kind, source, expected, *arguments)
    assert actual.read_bytes() == expected.read_bytes()


def test_pa_info_matches_cli(cli, tmp_path, capfd):
    source = tmp_path / "info.pa"
    simrep.PA(np.zeros((3, 3)), np.zeros((3, 3), bool)).write(source)
    expected = json.loads(cli("--json", "pa-info", source).stdout)
    assert simrep.pa_info(source) == expected
    assert capfd.readouterr().out == ""


def test_gpu_info_matches_cli(cli, gpu, capfd):
    expected = json.loads(cli("--json", "gpu-info", "--backend=vulkan").stdout)
    assert simrep.gpu_info(backend="vulkan") == expected
    assert capfd.readouterr().out == ""


def test_gpu_smoke_matches_cli(cli, gpu, capfd):
    expected = json.loads(cli("--json", "gpu-smoke", "--backend=vulkan").stdout)
    actual = simrep.gpu_smoke(backend="vulkan")
    assert capfd.readouterr().out == ""
    # Compare computed errors, not elapsed wall/GPU timestamp observations.
    assert [check["max_abs_error"] for check in actual["checks"]] == [
        check["max_abs_error"] for check in expected["checks"]]


@pytest.mark.parametrize("name,options,arguments,key", [
    ("fly_bench", dict(ions=2), ["--ions=2"], "checksum"),
    ("refine_bench", dict(n=7), ["--n=7"], "max_analytic_error"),
    ("refine_sdf", dict(n=9), ["--n=9"], "reports"),
])
def test_benchmark_commands_match_cli(cli, capfd, name, options, arguments, key):
    expected = json.loads(cli("--json", name.replace("_", "-"), *arguments).stdout)
    actual = getattr(simrep, name)(**options)
    assert capfd.readouterr().out == ""
    if key == "reports":
        for left, right in zip(actual[key], expected[key], strict=True):
            assert left["max_potential_error"] == right["max_potential_error"]
    else:
        assert actual[key] == expected[key]
    assert actual["schema_version"] == 1


def test_mesh_transform_and_defect_opt_in(cli, tmp_path, mesh):
    actual, expected = tmp_path / "python.pa#", tmp_path / "cli.pa#"
    transform = [[1, 0, 0, -1], [0, 1, 0, -1], [0, 0, 1, -1]]
    simrep.mesh_import(mesh, actual, dims=[5, 5, 5], spacing_mm=[0.5, 0.5, 0.5],
                       origin_mm=[-0.5, -0.5, -0.5], transform=transform, allow_defects=True)
    cli("mesh-import", mesh, expected, "--grid=5,5,5", "--spacing=0.5,0.5,0.5",
        "--origin=-0.5,-0.5,-0.5", "--transform=1,0,0,-1,0,1,0,-1,0,0,1,-1", "--allow-defects")
    assert actual.read_bytes() == expected.read_bytes()


def test_cylindrical_export_options_match_cli_bytes(cli, tmp_path):
    source = tmp_path / "cylindrical.pa"
    values = np.broadcast_to(np.arange(7), (1, 5, 7)).copy().astype(float)
    fixed = np.zeros_like(values, bool)
    fixed[:, :, 0] = True
    simrep.PA(values, fixed, dict(symmetry="cylindrical",
                                 mirrors=[False, True, False])).write(source)
    actual, expected = tmp_path / "python.vts", tmp_path / "cli.vts"
    simrep.export_grid(source, actual, revolve=True, no_field=True,
                       angular_segments=8, sweep_degrees=270)
    cli("export", "grid", source, expected, "--revolve", "--no-field",
        "--angular-segments=8", "--sweep-degrees=270")
    assert actual.read_bytes() == expected.read_bytes()
    actual, expected = tmp_path / "python.obj", tmp_path / "cli.obj"
    simrep.export_geometry(source, actual, electrode=0, origin_mm=[1, 2, 3],
                           angular_segments=8, sweep_degrees=270, extrusion_depth=2)
    cli("export", "geometry", source, expected, "--electrode=0", "--origin=1,2,3",
        "--angular-segments=8", "--sweep-degrees=270", "--extrusion-depth=2")
    assert actual.read_bytes() == expected.read_bytes()


@pytest.mark.parametrize("options,arguments", [
    (dict(charge_mode="beam"), ["--charge-mode=beam"]),
    (dict(beam_current_a=1e-9), ["--beam-current-a=1e-9"]),
    (dict(charge_mode="beam", beam_current_a=1e-9, space_charge="pic"),
     ["--charge-mode=beam", "--beam-current-a=1e-9", "--space-charge=pic"]),
])
def test_beam_validation_matches_cli_failure(cli, particles, options, arguments):
    assert cli("fly", "--particles", particles, *arguments, check=False).returncode
    with pytest.raises(ValueError):
        simrep.fly(None, particles, **options)
