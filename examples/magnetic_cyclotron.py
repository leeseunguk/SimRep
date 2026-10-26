"""Uniform magnetic field: measured gyroradius vs the analytic value.

A 100 eV, 40 amu singly charged ion in B = 10000 G (1 T) along z circles in
the xy plane. The analytic radius r = m v / (q B) is compared with the mean
distance of trajectory samples from the circle's center. To check an
imported solver map instead of a uniform field, export the map on a regular
grid and pass it with --bfield-map (see docs/bfield-import.md).

Run from the repository root:
    python examples/magnetic_cyclotron.py

CLI equivalent:
    simrep fly --particles ion.ion --bfield 0,0,10000 \
        --bounds -60,-60,-5,60,60,5 --duration-us 6 --allow-time-limit
"""

import numpy as np

import simrep

MASS_AMU, CHARGE_E, ENERGY_EV, B_GAUSS = 40.0, 1.0, 100.0, 10000.0
AMU_KG, E_CHARGE = 1.66053906660e-27, 1.602176634e-19


def main():
    v_m_per_s = np.sqrt(2.0 * ENERGY_EV * E_CHARGE / (MASS_AMU * AMU_KG))
    v_mm_per_us = v_m_per_s * 1e-3
    b_tesla = B_GAUSS * 1e-4
    r_analytic_mm = MASS_AMU * AMU_KG * v_m_per_s / (CHARGE_E * E_CHARGE * b_tesla) * 1e3
    period_us = 2.0 * np.pi * r_analytic_mm / v_mm_per_us
    print(f"analytic: r={r_analytic_mm:.6f} mm, "
          f"v={v_mm_per_us:.6f} mm/us, T={period_us:.6f} us")

    ions = simrep.particles_from_energy(
        [[0.0, 0.0, 0.0]], [1, 0, 0], ENERGY_EV, mass_amu=MASS_AMU)
    # source=None: field-free electric region, magnetic field only.
    # Magnetic flight is a CPU path under device auto; cpu is explicit here.
    result = simrep.fly(
        None, ions, device="cpu", bfield=[0, 0, B_GAUSS],
        bounds=[[-60, -60, -5], [60, 60, 5]],
        duration_us=2 * period_us, allow_time_limit=True,
        trajectories=True, sample_every=4,
        max_step_us=period_us / 200)

    points = result.trajectories[0][:, 1:3]
    center = (points.min(axis=0) + points.max(axis=0)) / 2.0
    r_measured = np.linalg.norm(points - center, axis=1).mean()
    error_ppm = (r_measured / r_analytic_mm - 1.0) * 1e6
    print(f"measured: r={r_measured:.6f} mm from {len(points)} samples")
    print(f"KEY radius_mm analytic={r_analytic_mm:.6f} "
          f"measured={r_measured:.6f} error_ppm={error_ppm:.2f}")


if __name__ == "__main__":
    main()
