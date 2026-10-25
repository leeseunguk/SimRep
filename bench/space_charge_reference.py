#!/usr/bin/env python3
"""Independent point-Coulomb DOP853 reference for recorded repulsion1 births.

Requires NumPy and SciPy. No production force, push, or event code is used.
All artifacts go to --output, never over the stored SIMION oracle.
"""
import argparse
import csv
import hashlib
import json
import os
from pathlib import Path
import re
import struct
import subprocess

import numpy as np
from scipy.integrate import solve_ivp


ACCELERATION = 1.602176634e-19 / 1.66053906892e-27 * 1e-6
COULOMB = 1 / (4 * np.pi * 8.8541878128e-15)
LOW = np.array([-10., -4., -4.])
HIGH = -LOW


def read_oracle(path):
    starts, finals = {}, {}
    for line in path.read_text().splitlines():
        if line.startswith(("START,", "FINAL,")):
            parts = line.split(",")
            target = starts if parts[0] == "START" else finals
            target[int(parts[1])] = list(map(float, parts[2:]))
    assert sorted(starts) == sorted(finals) == list(range(1, len(starts) + 1))
    initial = np.array([starts[i] for i in sorted(starts)])
    final = np.array([finals[i] for i in sorted(finals)])
    # Absolute time, position, velocity. SIMION records only final KE, not v.
    return initial, final[:, :4]


def reference(initial, charge, dt):
    """Integrate p'=v, v'=k(q/m) sum_j Q_j (p_i-p_j)/|p_i-p_j|^3.

    DOP853 dense output locates the first outward box crossing. At each birth
    or loss, restart the ODE with the new source population. Charges remain
    normalized to the entire input, not to the currently live population.
    """
    states = initial[:, 3:9].copy()
    times = initial[:, 0].copy()
    ratios = initial[:, 2] / initial[:, 1]
    order = np.argsort(times, kind="stable")
    pending, live, evaluations = 0, [], 0
    time = times[order[0]]
    while pending < len(order) or live:
        while pending < len(order) and initial[order[pending], 0] <= time:
            live.append(order[pending])
            pending += 1
        if not live:
            time = initial[order[pending], 0]
            continue
        ids = np.array(live)

        def rhs(_, y):
            state = y.reshape(-1, 6)
            delta = state[:, None, :3] - state[None, :, :3]
            r2 = np.einsum("ijk,ijk->ij", delta, delta)
            np.fill_diagonal(r2, np.inf)
            strength = charge[ids][None, :] / (r2 * np.sqrt(r2))
            acceleration = COULOMB * ACCELERATION * ratios[ids, None] * np.einsum(
                "ij,ijk->ik", strength, delta)
            return np.concatenate((state[:, 3:], acceleration), axis=1).ravel()

        events = []
        for local in range(len(ids)):
            def exit_box(_, y, local=local):
                p = y[6 * local:6 * local + 3]
                return min(np.min(p - LOW), np.min(HIGH - p))

            exit_box.terminal = True
            exit_box.direction = -1
            events.append(exit_box)
        end = initial[order[pending], 0] if pending < len(order) else time + 1.
        solution = solve_ivp(
            rhs, (time, end), states[ids].ravel(), method="DOP853",
            rtol=3e-13, atol=np.tile([1e-12] * 3 + [1e-10] * 3, len(ids)),
            max_step=dt, events=events)
        assert solution.success, solution.message
        evaluations += solution.nfev
        time = solution.t[-1]
        states[ids] = solution.y[:, -1].reshape(-1, 6)
        times[ids] = time
        if solution.status == 1:
            p = states[ids, :3]
            distance = np.minimum(p - LOW, HIGH - p).min(axis=1)
            velocity = states[ids, 3:]
            outward = (((p - LOW <= 1e-10) & (velocity < 0))
                       | ((HIGH - p <= 1e-10) & (velocity > 0))).any(axis=1)
            lost = (distance <= 1e-10) & outward
            assert lost.any()
            live = ids[~lost].tolist()
        elif pending == len(order):
            raise RuntimeError("reference did not leave the box in 1 us")
    return np.column_stack((times, states)), evaluations


def difference(a, b):
    distance = np.linalg.norm(a[:, 1:4] - b[:, 1:4], axis=1)
    return dict(endpoint_rms_mm=float(np.sqrt(np.mean(distance ** 2))),
                endpoint_max_mm=float(distance.max()),
                tof_rms_us=float(np.sqrt(np.mean((a[:, 0] - b[:, 0]) ** 2))))


def refined_blank(source, output, factor):
    """Refine the actual all-zero PA at identical physical extent."""
    raw = source.read_bytes()
    mode, symmetry, voltage, nx, ny, nz, mirror = struct.unpack("<iidiiii", raw[:32])
    assert mode == -1, "this probe expects the shipped standard blank.pa"
    assert not np.frombuffer(raw, dtype="<f8", offset=32).any(), "external PA is not zero"
    dims = [(n - 1) * factor + 1 for n in (nx, ny, nz)]
    path = output / f"blank-x{factor}.pa"
    path.write_bytes(struct.pack("<iidiiii", mode, symmetry, voltage, *dims, mirror)
                     + bytes(8 * np.prod(dims)))
    return path


def native_run(executable, case, output, mode, amount, dt, factor):
    path = refined_blank(case / "blank.pa", output, factor)
    csv_path = output / f"native-{dt:.8g}-x{factor}.csv"
    command = [
        str(executable.resolve()), "fly", "--pa", str(path),
        "--particles", str(case / "replay.ion"), "--origin=-10,0,0",
        "--scale", str(0.1 / factor), "--bounds=-10,-4,-4,10,4,4",
        "--duration-us", "1", "--space-charge", "direct",
        "--charge-mode", mode, "--charge-amount", str(amount),
        "--max-step-us", str(dt), "--tolerance", "1e-8",
        "--nonrelativistic", "--output", str(csv_path)]
    run = subprocess.run(
        command, capture_output=True, text=True, check=True, timeout=300,
        env={**os.environ, "RAYON_NUM_THREADS": "4"})
    rows = list(csv.DictReader(csv_path.open()))
    births, _ = read_oracle(case / "oracle.txt")
    result = np.array([
        [float(row["tof_us"]) + births[int(row["ion"]) - 1, 0]]
        + [float(row[a]) for a in ["x_mm", "y_mm", "z_mm"]]
        for row in rows])
    return result, dict(
        max_step_us=dt, grid_factor=factor,
        compute_s=float(re.search(r"fly_s:\s*([\d.]+)", run.stderr)[1]),
        stderr=run.stderr, command=command)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--oracle", type=Path,
                        default=Path("/mnt/c/Users/lsu10/simrep-oracle/space-charge"))
    parser.add_argument("--output", type=Path, default=Path("/tmp/simrep-space-charge-reference"))
    parser.add_argument("--mode", choices=["factor", "coulombic"], default="coulombic")
    parser.add_argument("--native", type=Path)
    parser.add_argument("--reference-steps", type=float, nargs="+", default=[1e-3, 5e-4, 2.5e-4])
    parser.add_argument("--native-steps", type=float, nargs="+", default=[1e-4, 5e-5, 2.5e-5])
    args = parser.parse_args()
    case = args.oracle / args.mode
    output = args.output / args.mode
    output.mkdir(parents=True, exist_ok=True)
    initial, oracle = read_oracle(case / "oracle.txt")
    amount = 1. if args.mode == "factor" else 3.47e-13
    charge = initial[:, 2] * initial[:, 10]
    charge *= 1.602176634e-19 if args.mode == "factor" else amount / np.abs(charge).sum()
    digest = hashlib.sha256(initial.tobytes() + charge.tobytes()).hexdigest()
    references, report = [], dict(mode=args.mode, particles=len(initial), input_sha256=digest)
    for dt in args.reference_steps:
        cached = output / f"dop853-events-{digest[:12]}-{dt:.8g}.csv"
        if cached.exists():
            result = np.loadtxt(cached, delimiter=",")
            evaluations = None
        else:
            result, evaluations = reference(initial, charge, dt)
            np.savetxt(cached, result, delimiter=",")
        references.append(result)
        print(json.dumps(dict(reference_dt_us=dt, evaluations=evaluations,
                              simion_error=difference(oracle, result))), flush=True)
    report["reference_halving"] = [
        difference(a, b) for a, b in zip(references, references[1:])]
    report["simion_error"] = difference(oracle, references[-1])
    if args.native:
        report["native"] = []
        for dt in args.native_steps:
            result, timing = native_run(args.native, case, output, args.mode, amount, dt, 1)
            timing["error"] = difference(result, references[-1])
            report["native"].append(timing)
            print(json.dumps(timing), flush=True)
        report["grid_refinement"] = []
        for factor in [2, 4, 8]:
            result, timing = native_run(
                args.native, case, output, args.mode, amount, args.native_steps[0], factor)
            timing["error"] = difference(result, references[-1])
            report["grid_refinement"].append(timing)
            print(json.dumps(timing), flush=True)
    report_path = output / ("verdict.json" if args.native else "reference.json")
    report_path.write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(report, indent=2), flush=True)


if __name__ == "__main__":
    main()
