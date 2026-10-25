"""Scan extraction voltage for a finite-source-width linear TOF instrument."""

import argparse
from time import perf_counter

import numpy as np

import simrep


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--ions", type=int, default=128)
    parser.add_argument("--points", type=int, default=9)
    parser.add_argument("--device", choices=["auto", "cpu"], default="cpu")
    args = parser.parse_args()
    if args.ions < 3 or args.points < 1:
        parser.error("--ions must be >=3 and --points >=1")
    # A 10 mm extraction gap followed by 90 mm of field-free drift. The exit
    # Dirichlet plane is explicitly an ideal transparent grid, not solid metal.
    labels = np.zeros((1, 5, 21))
    mask = np.zeros_like(labels, dtype=bool)
    labels[:, :, 0], labels[:, :, -1] = 1, 2
    mask[:, :, 0] = mask[:, :, -1] = True
    source = simrep.PA(labels, mask, {"spacing_mm": [0.5, 1.0, 1.0]})
    extraction = simrep.refine(source, adjustable=True)
    phase = np.linspace(-1, 1, args.ions)
    positions = np.column_stack([1.0 + 0.05 * phase,
                                 np.full(args.ions, 2.0), np.zeros(args.ions)])
    energies = 0.1 + 0.01 * np.cos(np.pi * phase)
    ions = simrep.particles_from_energy(positions, [1, 0, 0], energies, mass_amu=100)
    print(f"# initial refine compute_ms={extraction.report['compute_ms']:.6f}")
    print("voltage_V,transmission,mean_tof_us,sigma_tof_us,resolving_power_sigma,compute_ms,total_ms")
    for voltage in np.linspace(100, 1000, args.points):
        start = perf_counter()
        field = extraction.fast_adjust({1: voltage, 2: 0})
        result = simrep.fly(
            field, ions, device=args.device, duration_us=1000,
            bounds=[[-1, -10, -10], [101, 10, 10]], plane=[1, 0, 0, 100],
            transparent_faces=[False, True, False, False, False, False],
            tolerance=1e-10)
        detected = result.arrays["reason"] == "plane"
        if not detected.any():
            raise RuntimeError("no ions reached the detector; inspect termination reasons")
        times = result.arrays["tof_us"][detected]
        mean, sigma = np.mean(times), np.std(times, ddof=1)
        # Delta(m)/m = 2 Delta(t)/t: report one-sigma, not FWHM resolution.
        resolution = mean / (2 * sigma)
        compute_ms = field.report["compute_ms"] + result.report["compute_ms"]
        print(f"{voltage:.9g},{np.mean(detected):.9g},{mean:.12g},{sigma:.9g},"
              f"{resolution:.9g},{compute_ms:.6f},"
              f"{(perf_counter() - start) * 1000:.6f}", flush=True)


if __name__ == "__main__":
    main()
