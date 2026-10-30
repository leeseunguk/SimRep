"""Opt-in automatic surface selection agrees with the CLI's native policy."""
import json
import os
from pathlib import Path
import subprocess

import numpy as np
import pytest
import simrep


@pytest.fixture
def surface_flight(tmp_path):
    gem = tmp_path / "plates.gem"
    gem.write_text("pa_define(9,5,1,planar)\ne(1){fill{within{box(-1,-1,2,5)}}}\ne(2){fill{within{box(6,-1,9,5)}}}")
    simrep.refine_geometry(gem, electrodes={1: (1, 0), 2: (2, 0)},
                           out=tmp_path / "plates", device="cpu")
    ions = tmp_path / "ions.ion"
    ions.write_text("0,40,1,4,2,0,0,0,1\n")
    return tmp_path / "plates.pa0", ions


@pytest.mark.parametrize("device", ["cpu", "auto", "gpu"])
def test_auto_surface_json_and_stderr_match_cli(surface_flight, capfd, monkeypatch, device):
    monkeypatch.delenv("SIMREP_SDF_GPU_CROSSOVER", raising=False)
    pa, ions = surface_flight
    root = Path(__file__).resolve().parents[3]
    binary = Path(os.environ.get("SIMREP_BIN", root / "target/release/simrep"))
    completed = subprocess.run(
        [str(binary), "--json", "fly", "--pa", str(pa), "--particles", str(ions),
         "--device", device, "--surface-backend", "auto", "--duration-us", "0.01",
         "--allow-time-limit"], capture_output=True, text=True, check=True,
        stdin=subprocess.DEVNULL, timeout=60,
    )
    expected = json.loads(completed.stdout)
    capfd.readouterr()
    result = simrep.fly(pa, ions, device=device, surface_backend="auto",
                        duration_us=0.01, allow_time_limit=True)
    captured = capfd.readouterr()
    assert captured.out == ""
    for key in ["reason", "fallback_reason"]:
        assert result.report[key] == expected[key]
    for key in ["device", "surface_backend"]:
        assert result.report["requested"][key] == expected["requested"][key]
        assert result.report["executed"][key] == expected["executed"][key]
    assert result.report["executed"]["surface_backend"] == "exact"
    assert result.report["fallback_reason"] is None
    assert result.report["reason"] in captured.err
    assert result.report["reason"] in completed.stderr


def test_default_exact_and_opt_in_auto_preserve_numeric_results(surface_flight, monkeypatch):
    monkeypatch.delenv("SIMREP_SDF_GPU_CROSSOVER", raising=False)
    pa, ions = surface_flight
    results = [simrep.fly(pa, ions, duration_us=0.01, allow_time_limit=True, **options)
               for options in [{}, {"surface_backend": "exact"}, {"surface_backend": "auto"}]]
    for result in results[1:]:
        for name, values in results[0].arrays.items():
            np.testing.assert_array_equal(values, result.arrays[name])
    assert results[0].report["requested"]["surface_backend"] == "exact"


def test_software_adapter_is_refused_even_with_override(surface_flight, monkeypatch):
    adapter = simrep.gpu_info(backend="vulkan")
    if not adapter["software_adapter"]:
        pytest.skip("Software-adapter refusal requires a software Vulkan adapter")
    monkeypatch.setenv("SIMREP_SDF_GPU_CROSSOVER", "0")
    pa, ions = surface_flight
    for device in ["auto", "gpu"]:
        result = simrep.fly(pa, ions, device=device, surface_backend="auto",
                            duration_us=0.01, allow_time_limit=True)
        assert result.report["executed"]["device"] == "cpu"
        assert result.report["executed"]["surface_backend"] == "exact"
        assert "software adapters refused" in result.report["reason"]
        assert result.report["fallback_reason"] is None


@pytest.mark.parametrize("physics", [
    {"bfield": [0, 0, 1]},
    {"collision": "hs1"},
    {"space_charge": "direct"},
    {"space_charge": "pic", "bounds": [[0, 0, -1], [8, 4, 1]], "pic_dims": [3, 3, 3]},
    {"charge_mode": "beam", "beam_current_a": 1e-12},
    {"transparent_faces": [True, False, False, False, False, False]},
])
def test_override_cannot_enable_unsupported_sdf_physics(surface_flight, monkeypatch, physics):
    monkeypatch.setenv("SIMREP_SDF_GPU_CROSSOVER", "0")
    pa, ions = surface_flight
    result = simrep.fly(pa, ions, device="gpu", surface_backend="auto",
                        duration_us=0.01, allow_time_limit=True, **physics)
    assert result.report["executed"]["device"] == "cpu"
    assert result.report["executed"]["surface_backend"] == "exact"
    assert result.report["fallback_reason"] is None
    assert "physics unsupported on GPU SDF" in result.report["reason"]
