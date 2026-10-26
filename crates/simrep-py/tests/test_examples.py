"""Run the repository examples so they cannot silently rot.

Each script under ``examples/`` is executed as a subprocess from the repo
root and must exit 0 and print its ``KEY`` result line within the time
limit. The result line is then checked against the physics each example
demonstrates (all ions detected, exact hits on the 8 mm sphere, the
analytic gyroradius), with tolerances well above platform rounding.
"""

import re
import subprocess
import sys
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[3]
EXAMPLES = ROOT / "examples"

NUM = r"([-\d.eE+]+)"


def _einzel(m):
    assert float(m[1]) > 0 and float(m[2]) >= 0
    assert int(m[3]) == int(m[4]) > 0, "every ion should reach the detector plane"


def _sweep(m):
    assert 0.0 < float(m[2]) <= 1.0


def _surface(m):
    staircase, exact = float(m[1]), float(m[2])
    assert abs(exact - 8.0) < 1e-3, "exact surfaces must hit the 8 mm sphere"
    assert 8.0 - staircase > 0.05, "staircase hits land inside the sphere"


def _cyclotron(m):
    analytic, measured = float(m[1]), float(m[2])
    assert abs(measured / analytic - 1.0) < 1e-4


CASES = {
    "einzel_tof.py": (rf"KEY tof_us mean={NUM} std={NUM} detected=(\d+)/(\d+)", 30, _einzel),
    "voltage_sweep.py": (rf"KEY best_voltage=(-?\d+) aperture_transmission={NUM}", 30, _sweep),
    "exact_surface.py": (rf"KEY staircase_hit_radius={NUM} exact_hit_radius={NUM}", 60, _surface),
    "magnetic_cyclotron.py": (rf"KEY radius_mm analytic={NUM} measured={NUM}", 30, _cyclotron),
}


@pytest.mark.parametrize("script", sorted(CASES))
def test_example(script):
    pattern, limit, check = CASES[script]
    result = subprocess.run(
        [sys.executable, str(EXAMPLES / script)],
        cwd=ROOT,
        capture_output=True,
        text=True,
        timeout=limit,
    )
    assert result.returncode == 0, (
        f"{script} exited {result.returncode}\nstdout:\n{result.stdout}\n"
        f"stderr:\n{result.stderr[-2000:]}")
    match = re.search(pattern, result.stdout)
    assert match, (
        f"{script} did not print its KEY result line\n"
        f"stdout:\n{result.stdout}")
    check(match)
