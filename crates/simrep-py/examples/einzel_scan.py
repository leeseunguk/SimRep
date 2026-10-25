"""Optimize an analytic three-electrode einzel lens with reusable cut-cell bases."""

import argparse
from time import perf_counter

import numpy as np
from scipy.optimize import minimize_scalar

import simrep


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--ions", type=int, default=128)
    parser.add_argument("--iterations", type=int, default=16)
    parser.add_argument("--device", choices=["cpu", "gpu", "auto"], default="cpu")
    args = parser.parse_args()
    if args.ions < 2 or args.iterations < 1:
        parser.error("--ions must be >=2 and --iterations >=1")

    def ring(part, center, width):
        return dict(part=str(part), shape=dict(
            kind="difference",
            a=dict(kind="box", center=[center, 0, 0],
                   half_extent=[width / 2, 12, 12]),
            b=dict(kind="cylinder", center=[0, 0, 0], radius=4, axis=0)))

    # This generated lens needs no SIMION installation or proprietary examples.
    lens = simrep.refine_geometry(
        [ring(1, 14, 8), ring(2, 30, 10), ring(3, 46, 8)],
        dims=(61, 13, 1), symmetry="cylindrical",
        electrodes={1: (1, 0), 2: (2, 0), 3: (3, 0)}, device="cpu")
    radial = np.linspace(0.2, 2.0, args.ions)
    position = np.column_stack([np.ones(args.ions), radial, np.zeros(args.ions)])
    direction = np.column_stack([np.ones(args.ions), radial * 0.02, np.zeros(args.ions)])
    ions = simrep.particles_from_energy(position, direction, 100, mass_amu=40)
    print(f"# initial refine compute_ms={lens.report['compute_ms']:.6f}")
    print("voltage_V,transmission,rms_radius_mm,compute_ms,total_ms")

    def objective(voltage):
        start = perf_counter()
        adjusted = lens.fast_adjust({2: voltage})
        result = simrep.fly(
            adjusted, ions, device=args.device, duration_us=100,
            plane=[1, 0, 0, 58], tolerance=1e-10)
        records = result.records
        reached = records["reason"] == "plane"
        radii = np.hypot(records["y_mm"], records["z_mm"])
        transmission = np.mean(reached & (radii < 2.0))
        rms = np.sqrt(np.mean(radii[reached] ** 2)) if reached.any() else 12.0
        compute_ms = adjusted.report["compute_ms"] + result.report["compute_ms"]
        print(f"{voltage:.9g},{transmission:.9g},{rms:.9g},"
              f"{compute_ms:.6f},{(perf_counter() - start) * 1000:.6f}", flush=True)
        return (1 - transmission) * 10 + rms

    optimum = minimize_scalar(objective, bounds=(10, 95), method="bounded",
                              options={"maxiter": args.iterations, "xatol": 0.1})
    print(f"# best voltage={optimum.x:.6f} V objective={optimum.fun:.9g} "
          f"evaluations={optimum.nfev} converged={optimum.success}")


if __name__ == "__main__":
    main()
