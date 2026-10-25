#!/usr/bin/env python3
"""Independent DOP853 reference for the two recorded, straight-leader beams.

Integrates transverse positions/velocities with leader-plane x as the independent
variable. No production force, push, or event code is used. NumPy/SciPy required.
The all-zero external PA has no field-interpolation error; grid probes keep its
physical extent fixed. Softening is a physical model parameter, not refinement.
"""
import argparse
from collections import Counter
import csv
import hashlib
import json
import math
import os
from pathlib import Path
import re
import shutil
import struct
import subprocess
import statistics

import numpy as np
from scipy.integrate import solve_ivp

from beam_compare import records


ACCELERATION = 1.602176634e-19 / 1.66053906892e-27 * 1e-6
COULOMB = 1 / (4 * math.pi * 8.8541878128e-15)
POSITION = ["x_mm", "y_mm", "z_mm"]
VELOCITY = ["vx_mm_per_us", "vy_mm_per_us", "vz_mm_per_us"]


def difference(a, b):
    distance = np.linalg.norm(a[:, 1:4] - b[:, 1:4], axis=1)
    time = np.abs(a[:, 0] - b[:, 0])
    velocity = np.linalg.norm(a[:, 4:7] - b[:, 4:7], axis=1)
    return dict(endpoint_rms_mm=float(np.sqrt(np.mean(distance ** 2))),
                endpoint_max_mm=float(distance.max()),
                tof_rms_us=float(np.sqrt(np.mean(time ** 2))),
                tof_max_us=float(time.max()),
                velocity_rms_mm_per_us=float(np.sqrt(np.mean(velocity ** 2))),
                worst_ion=int(distance.argmax()) + 1)


def exit_classes(states):
    """Side-face labels and the repeated terminal follower plane.

    SIMION's terminal follower plane can trail the leader by a few micrometres.
    Classifying that numerical offset as a side-face change would be spurious.
    """
    result = []
    counts = Counter(np.round(states[1:, 1], 8))
    plane_x, count = counts.most_common(1)[0]
    if count < 2:
        plane_x = states[0, 1]
    for i, s in enumerate(states):
        p = s[1:4]
        if i == 0 or abs(p[0] - plane_x) < 1e-7:
            result.append("leader_plane")
            continue
        faces = [(abs(p[1] - 4), "y+"), (abs(p[1] + 4), "y-"), (abs(p[2] - 4), "z+"),
                 (abs(p[2] + 4), "z-")]
        result.append(min(faces)[1])
    return result


def reference(initial, current, max_dx, tolerance):
    """Exact zero-field, straight-plane line model, restarting at every loss."""
    n = len(initial)
    assert np.all(initial[:, 3] == -10)
    assert initial[0, 2] == 0 and np.all(initial[0, 7:9] == 0)
    vx = initial[:, 6]
    assert np.all(vx > 0)
    weights = initial[:, 2] * initial[:, 10]
    currents = current * weights / np.abs(weights).sum()
    density = currents * 1e-6 / vx
    ratio = initial[:, 2] / initial[:, 1]
    state = initial[:, [4, 5, 7, 8]].copy()
    final = np.zeros((n, 7))
    live = np.arange(n)
    x, evaluations = -10., 0
    closest = (math.inf, 0, 0, x)
    losses = []
    while len(live):
        ids = live.copy()

        def rhs(plane, values):
            nonlocal closest
            s = values.reshape(-1, 4)
            delta = s[:, None, :2] - s[None, :, :2]
            r2 = np.einsum("ijk,ijk->ij", delta, delta)
            np.fill_diagonal(r2, np.inf)
            pair = np.unravel_index(r2.argmin(), r2.shape)
            if r2[pair] < closest[0] ** 2:
                closest = (float(np.sqrt(r2[pair])), int(ids[pair[0]]) + 1,
                           int(ids[pair[1]]) + 1, float(plane))
            electric = 2 * COULOMB * np.einsum(
                "ij,ijk->ik", density[ids][None, :] / r2, delta)
            acceleration = ACCELERATION * ratio[ids, None] * electric
            return np.column_stack((s[:, 2:] / vx[ids, None],
                                    acceleration / vx[ids, None])).ravel()

        events = []
        for local in range(len(ids)):
            def side(_, values, local=local):
                return 4 - np.abs(values[4 * local:4 * local + 2]).max()
            side.terminal = True
            side.direction = -1
            events.append(side)
        sol = solve_ivp(
            rhs, (x, 10.), state[ids].ravel(), method="DOP853",
            max_step=max_dx, rtol=tolerance,
            atol=np.tile([tolerance, tolerance, tolerance * 100, tolerance * 100], len(ids)),
            events=events)
        if not sol.success:
            raise RuntimeError(sol.message)
        evaluations += sol.nfev
        x = float(sol.t[-1])
        state[ids] = sol.y[:, -1].reshape(-1, 4)
        if sol.status == 1:
            outward = (np.abs(state[ids, :2]) >= 4 - 1e-10) & (
                state[ids, :2] * state[ids, 2:] > 0)
            lost = outward.any(axis=1)
            assert lost.any()
        else:
            lost = np.ones(len(ids), dtype=bool)
        for i in ids[lost]:
            final[i] = [(x + 10) / vx[i], x, *state[i, :2], vx[i], *state[i, 2:]]
            losses.append([int(i) + 1, x])
        live = ids[~lost]
    return final, dict(evaluations=evaluations, closest_pair_mm=list(closest),
                       loss_order=losses)


def blank(source, output, factor):
    raw = source.read_bytes()
    mode, symmetry, voltage, nx, ny, nz, mirror = struct.unpack("<iidiiii", raw[:32])
    assert mode == -1
    assert not np.frombuffer(raw, dtype="<f8", offset=32).any()
    dims = [(v - 1) * factor + 1 for v in (nx, ny, nz)]
    path = output / f"blank-x{factor}.pa"
    path.write_bytes(struct.pack("<iidiiii", mode, symmetry, voltage, *dims, mirror)
                     + bytes(8 * math.prod(dims)))
    return path


def native_run(args, output, dt, grid, fraction, trial):
    path = output / (f"native-{args.native_sha256[:12]}-{dt:.9g}"
                     f"-x{grid}-f{fraction:.9g}-t{args.threads}-trial{trial}.csv")
    log = path.with_suffix(".json")
    command = [
        str(args.native.resolve()), "fly", "--pa", str(blank(args.case / "blank.pa", output, grid)),
        "--particles", str(args.case / "replay.ion"), "--origin=-10,0,0",
        "--scale", str(.1 / grid), "--bounds=-10,-4,-4,10,4,4",
        "--duration-us", "1", "--device", "cpu", "--charge-mode", "beam",
        "--beam-current-a", str(args.current),
        "--nonrelativistic", "--output", str(path)]
    if dt > 0:
        command += ["--max-step-us", str(dt)]
    if fraction > 0:
        command += ["--space-charge-step-fraction", str(fraction)]
    if not log.exists() or args.repeat:
        proc = subprocess.run(command, capture_output=True, text=True, check=True,
                              stdin=subprocess.DEVNULL, timeout=args.native_timeout,
                              env={**os.environ, "RAYON_NUM_THREADS": str(args.threads)})
        metadata = dict(command=command, stderr=proc.stderr,
                        compute_s=float(re.search(r"fly_s:\s*([\d.]+)", proc.stderr)[1]),
                        global_steps=int(re.search(r"global_steps:\s*(\d+)", proc.stderr)[1]),
                        retries=int(re.search(r"global_steps:\s*\d+,\s*rejected:\s*(\d+)", proc.stderr)[1]))
        log.write_text(json.dumps(metadata, indent=2) + "\n")
    else:
        metadata = json.loads(log.read_text())
    with path.open() as f:
        rows = list(csv.DictReader(f))
    result = np.array([[float(row["tof_us"])] +
                       [float(row[k]) for k in POSITION + VELOCITY] for row in rows])
    return result, dict(max_step_us=dt, grid_factor=grid, step_fraction=fraction,
                        threads=args.threads,
                        terminations={reason: sum(r["reason"] == reason for r in rows)
                                      for reason in sorted({r["reason"] for r in rows})},
                        **metadata)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--case", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--reference-cache", type=Path,
                        help="reuse independent reference CSVs from a prior output directory")
    parser.add_argument("--native", type=Path)
    parser.add_argument("--threads", type=int, default=4)
    parser.add_argument("--trials", type=int, default=1)
    parser.add_argument("--native-timeout", type=float, default=300)
    parser.add_argument("--baseline-fraction", type=float, default=.1,
                        help="0 omits the flag and tests the actual beam default")
    parser.add_argument("--reference-dx", nargs="+", type=float, default=[.25, .125, .0625])
    parser.add_argument("--reference-tolerance", type=float, default=3e-12)
    parser.add_argument("--native-steps", nargs="+", type=float, default=[1e-4, 5e-5, 2.5e-5],
                        help="leader/follower maximum dt in us; 0 uses automatic CLI defaults")
    parser.add_argument("--grid-factors", nargs="+", type=int, default=[])
    parser.add_argument("--step-fractions", nargs="+", type=float, default=[])
    parser.add_argument("--repeat", action="store_true")
    args = parser.parse_args()
    args.current = 1e-5
    if args.native:
        args.native_sha256 = hashlib.sha256(args.native.read_bytes()).hexdigest()
    args.output.mkdir(parents=True, exist_ok=True)
    cache = args.reference_cache or args.output
    text = (args.case / "oracle.txt").read_text()
    starts, ends = records(text, "BEAM_START"), records(text, "BEAM_FINAL")
    assert sorted(starts) == sorted(ends) == list(range(1, len(starts) + 1))
    initial = np.array([starts[i] for i in sorted(starts)])
    oracle = np.array([ends[i][:7] for i in sorted(starts)])
    oracle[:, 0] -= initial[:, 0]
    digest = hashlib.sha256(initial.tobytes()).hexdigest()
    if cache.resolve() != args.output.resolve():
        for source in cache.glob(f"dop853-{digest[:12]}-*"):
            shutil.copyfile(source, args.output / source.name)
    report = dict(case=args.case.name, input_sha256=digest, particles=len(initial),
                  current_a=args.current,
                  native_sha256=args.native_sha256 if args.native else None,
                  reference=[], native=[],
                  simion_compute_s=float(re.search(
                      r"Fly completed\.\s+\d+ splats,\s+([\d.]+) seconds", text)[1]))
    previous = None
    for dx in args.reference_dx:
        key = f"dop853-{digest[:12]}-{dx:.9g}-{args.reference_tolerance:.3g}"
        path = cache / (key + ".csv")
        meta_path = cache / (key + ".json")
        if path.exists():
            ref = np.loadtxt(path, delimiter=",")
            meta = json.loads(meta_path.read_text())
        else:
            ref, meta = reference(initial, args.current, dx, args.reference_tolerance)
            np.savetxt(path, ref, delimiter=",", fmt="%.17g")
            meta_path.write_text(json.dumps(meta, indent=2) + "\n")
        entry = dict(max_plane_spacing_mm=dx, tolerance=args.reference_tolerance, **meta)
        if previous is not None:
            entry["self_change"] = difference(ref, previous)
            entry["class_changes"] = sum(a != b for a, b in zip(exit_classes(ref), exit_classes(previous)))
        report["reference"].append(entry)
        print(json.dumps(dict(reference={k: v for k, v in entry.items()
                                         if k != "loss_order"})), flush=True)
        previous = ref
    np.savetxt(args.output / "reference.csv", ref, delimiter=",", fmt="%.17g")
    report["simion_error"] = difference(oracle, ref)
    report["simion_class_changes"] = [
        i + 1 for i, (a, b) in enumerate(zip(exit_classes(oracle), exit_classes(ref))) if a != b]
    report["reference_exit_classes"] = {
        c: exit_classes(ref).count(c) for c in sorted(set(exit_classes(ref)))}
    variants = [(dt, 1, args.baseline_fraction) for dt in args.native_steps]
    if args.native_steps:
        variants += [(args.native_steps[-1], g, args.baseline_fraction) for g in args.grid_factors]
        variants += [(args.native_steps[-1], 1, f) for f in args.step_fractions]
    previous = None
    if args.native:
        for dt, grid, fraction in variants:
            times = []
            first = None
            try:
                for trial in range(args.trials):
                    result, meta = native_run(args, args.output, dt, grid, fraction, trial)
                    times.append(meta["compute_s"])
                    if first is not None:
                        assert np.array_equal(first, result), "native trials changed numerical results"
                    first = result
            except subprocess.CalledProcessError as error:
                entry = dict(max_step_us=dt, grid_factor=grid, step_fraction=fraction,
                             failure=error.returncode, stderr=error.stderr)
                report["native"].append(entry)
                print(json.dumps(dict(native=entry)), flush=True)
                continue
            meta["compute_trials_s"] = times
            meta["compute_s"] = statistics.median(times)
            entry = dict(**meta, error=difference(result, ref))
            if previous is not None:
                entry["self_change"] = difference(result, previous)
            errors = np.linalg.norm(result[:, 1:4] - ref[:, 1:4], axis=1)
            entry["ions_above_1e_4_mm"] = (np.flatnonzero(errors > 1e-4) + 1).tolist()
            entry["class_changes"] = [
                i + 1 for i, (a, b) in enumerate(zip(exit_classes(result), exit_classes(ref))) if a != b]
            report["native"].append(entry)
            print(json.dumps(dict(native={k: v for k, v in entry.items()
                                          if k not in ("command", "stderr", "ions_above_1e_4_mm")})), flush=True)
            previous = result
    (args.output / "report.json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps({k: v for k, v in report.items() if k not in ("reference", "native")}), flush=True)


if __name__ == "__main__":
    main()
