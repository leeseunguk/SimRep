"""The native settings table accepts and executes the sampled surface backend."""
from pathlib import Path

import pytest
import simrep


@pytest.mark.parametrize("divisor", [1, 2])
def test_surface_sdf_settings_and_execution(tmp_path, divisor):
    source = Path(__file__).resolve().parents[3] / "examples" / "data" / "einzel.gem"
    field = simrep.refine_geometry(
        source, electrodes={1: (1, 0), 2: (2, 60), 3: (3, 0)},
        out=tmp_path / "einzel", device="cpu",
    )
    particles = simrep.particles_from_energy(
        position_mm=[1, 1, 0], direction=[1, 0.5, 0],
        energy_ev=500, mass_amu=40, charge_e=1,
    )
    result = simrep.fly(
        field, particles, device="cpu", surface_backend="sdf",
        surface_sdf_resolution_divisor=divisor, plane=[1, 0, 0, 58],
    )
    assert result.arrays["reason"].tolist() == ["electrode"]
    assert result.arrays["electrode_number"].tolist() == [1]
    assert result.report["executed"] == {
        "device": "cpu", "precision": "f64", "surface_backend": "sdf",
    }
