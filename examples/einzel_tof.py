"""Einzel lens: GEM file -> PA# -> refine -> fly ions -> TOF statistics.

Run from the repository root with the built simrep package:
    python examples/einzel_tof.py

CLI equivalent (writes results.csv):
    simrep gem2pa examples/data/einzel.gem einzel.pa#
    simrep refine einzel.pa#
    simrep fly --pa einzel.pa0 --particles examples/data/ions.ion \
        --voltage 1=0,2=60,3=0 --plane 1,0,0,58 --duration-us 2000 \
        --output results.csv
"""

import tempfile
from pathlib import Path

import numpy as np

import simrep

DATA = Path(__file__).resolve().parent / "data"
IONS = 32


def main():
    with tempfile.TemporaryDirectory(prefix="simrep-einzel-") as work:
        pa_path = Path(work) / "einzel.pa#"
        simrep.gem2pa(DATA / "einzel.gem", pa_path)
        # A .pa# refines as an adjustable field: one basis per electrode.
        field = simrep.refine(pa_path, device="cpu")
        print(f"refine: {IONS} ions, electrodes {sorted(field.voltages)}, "
              f"compute_ms={field.report['compute_ms']:.2f}")

        rng = np.random.default_rng(7)
        positions = np.column_stack(
            [np.full(IONS, 1.0), rng.uniform(0.2, 2.0, IONS), np.zeros(IONS)])
        ions = simrep.particles_from_energy(
            positions, [1, 0, 0], 1000, mass_amu=40)

        lens = field.fast_adjust({1: 0.0, 2: 60.0, 3: 0.0})
        result = simrep.fly(lens, ions, device="cpu",
                            plane=[1, 0, 0, 58], duration_us=2000,
                            tolerance=1e-10)
        detected = result.arrays["reason"] == "plane"
        tof = result.arrays["tof_us"][detected]
        print(f"KEY tof_us mean={tof.mean():.6f} std={tof.std(ddof=1):.6f} "
              f"detected={detected.sum()}/{IONS}")
        print(f"fly compute_ms={result.report['compute_ms']:.2f} "
              f"setup_ms={result.report['setup_ms']:.2f}")

        try:
            import matplotlib.pyplot as plt
        except ImportError:
            print("matplotlib not installed; skipping histogram")
            return
        fig, ax = plt.subplots()
        ax.hist(tof, bins=16)
        ax.set_xlabel("time of flight (us)")
        ax.set_ylabel("ions")
        out = Path(work) / "tof_hist.png"
        fig.savefig(out, dpi=120)
        print(f"histogram written to {out}")


if __name__ == "__main__":
    main()
