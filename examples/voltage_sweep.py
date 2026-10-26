"""Fast adjust: refine once, fly at several einzel voltages.

Run from the repository root:
    python examples/voltage_sweep.py

CLI equivalent (one fly per voltage; the CLI reuses the .paN bases on disk):
    simrep gem2pa examples/data/einzel.gem einzel.pa#
    simrep refine einzel.pa#
    for V in -400 -200 -100 0 100 200 400; do
        simrep fly --pa einzel.pa0 --particles examples/data/ions.ion \
            --voltage 1=0,2=$V,3=0 --plane 1,0,0,58 --duration-us 4000
    done
"""

import tempfile
from pathlib import Path

import numpy as np

import simrep

DATA = Path(__file__).resolve().parent / "data"
IONS = 32
APERTURE_MM = 1.0


def main():
    with tempfile.TemporaryDirectory(prefix="simrep-sweep-") as work:
        pa_path = Path(work) / "einzel.pa#"
        simrep.gem2pa(DATA / "einzel.gem", pa_path)
        field = simrep.refine(pa_path, device="cpu")
        print(f"refine compute_ms={field.report['compute_ms']:.2f}")

        rng = np.random.default_rng(7)
        positions = np.column_stack(
            [np.full(IONS, 1.0), rng.uniform(0.2, 2.0, IONS), np.zeros(IONS)])
        ions = simrep.particles_from_energy(
            positions, [1, 0, 0], 500, mass_amu=40)

        print("lens_voltage_V,transmission_plane,transmission_aperture")
        best = (None, -1.0)
        for voltage in [-400, -200, -100, 0, 100, 200, 400]:
            # Fast adjust is NumPy vector arithmetic on the unit bases:
            # no solver call per voltage.
            lens = field.fast_adjust({2: float(voltage)})
            result = simrep.fly(lens, ions, device="cpu",
                                plane=[1, 0, 0, 58], duration_us=4000)
            a = result.arrays
            plane = a["reason"] == "plane"
            aperture = plane & (np.hypot(a["y_mm"], a["z_mm"]) < APERTURE_MM)
            print(f"{voltage},{plane.mean():.4f},{aperture.mean():.4f}")
            if aperture.mean() > best[1]:
                best = (voltage, aperture.mean())
        print(f"KEY best_voltage={best[0]} aperture_transmission={best[1]:.4f}")


if __name__ == "__main__":
    main()
