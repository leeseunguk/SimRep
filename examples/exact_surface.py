"""Exact surfaces vs staircase: refine-geometry on a GEM sphere electrode.

Ions fly through a grounded spherical electrode on their way to a detector.
Exact surface flight reports impacts at the true radius; staircase flight
stops ions at voxel cell faces, roughly half a grid cell short.

Run from the repository root:
    python examples/exact_surface.py

CLI equivalent:
    simrep refine-geometry examples/data/electrodes.gem \
        --electrode 1=1:0 --electrode 2=2:0 --out e
    simrep fly --pa e.pa0 --particles ions.ion --plane 1,0,0,32 \
        --duration-us 3000            # uses e.cuts.json automatically
    simrep fly --pa e.pa0 --particles ions.ion --plane 1,0,0,32 \
        --duration-us 3000 --no-cuts  # staircase collisions
"""

import tempfile
from pathlib import Path

import numpy as np

import simrep

DATA = Path(__file__).resolve().parent / "data"
SPHERE_CENTER = np.array([20.0, 10.0, 10.0])
SPHERE_RADIUS = 8.0
IONS = 21


def hit_radii(result):
    a = result.arrays
    hit = a["reason"] == "electrode"
    points = np.column_stack([a["x_mm"], a["y_mm"], a["z_mm"]])
    return np.linalg.norm(points[hit] - SPHERE_CENTER, axis=1), int(hit.sum())


def main():
    with tempfile.TemporaryDirectory(prefix="simrep-surface-") as work:
        stem = str(Path(work) / "e")
        field = simrep.refine_geometry(
            DATA / "electrodes.gem",
            electrodes={1: (1, 0.0), 2: (2, 0.0)},
            device="cpu", out=stem)
        print(f"refine-geometry compute_ms={field.report['compute_ms']:.1f}")

        positions = np.column_stack(
            [np.full(IONS, 5.0), np.linspace(1.5, 18.5, IONS),
             np.full(IONS, 10.0)])
        ions = simrep.particles_from_energy(
            positions, [1, 0, 0], 200, mass_amu=40)

        exact = simrep.fly(field, ions, device="cpu",
                           plane=[1, 0, 0, 32], duration_us=3000)
        stair = simrep.fly(field, ions, device="cpu",
                           plane=[1, 0, 0, 32], duration_us=3000,
                           no_cuts=True)
        r_e, n_e = hit_radii(exact)
        r_s, n_s = hit_radii(stair)
        print(f"exact surface:   {n_e} electrode hits, "
              f"hit radius {r_e.mean():.4f} mm (surface at {SPHERE_RADIUS})")
        print(f"staircase:       {n_s} electrode hits, "
              f"hit radius {r_s.mean():.4f} mm")
        print(f"KEY staircase_hit_radius={r_s.mean():.4f} "
              f"exact_hit_radius={r_e.mean():.4f} "
              f"offset_mm={r_e.mean() - r_s.mean():.4f}")


if __name__ == "__main__":
    main()
