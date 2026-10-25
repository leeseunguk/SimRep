#!/usr/bin/env python3
"""Controlled repulsion1 endpoint oracle and exact initial-state replay.

Run each SIMION invocation foreground, closed stdin, C-drive cwd, 25 s timeout.
The output directory is a new working copy; installed/original examples are not changed.
The native comparison can run separately with --native-only after the oracle.
"""
import argparse
import csv
import json
import math
import os
from pathlib import Path
import re
import shutil
import statistics
import subprocess


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source", type=Path, default=Path("/mnt/c/Users/lsu10/simrep-oracle/repulsion"))
    parser.add_argument("--work", type=Path, default=Path("/mnt/c/Users/lsu10/simrep-oracle/space-charge"))
    parser.add_argument("--mode", choices=["factor", "coulombic"], default="coulombic")
    parser.add_argument("--amount", type=float, default=3.47e-13)
    parser.add_argument("--quality", type=int, default=-9)
    parser.add_argument("--native-only", action="store_true")
    parser.add_argument("--native", type=Path)
    parser.add_argument("--max-step", type=float, default=1e-4)
    args = parser.parse_args()
    work = args.work / args.mode
    work.mkdir(parents=True, exist_ok=True)
    log = work / "oracle.txt"
    if not args.native_only:
        for name in ["blank.pa", "repulsion1.iob", "repulsion1.fly2"]:
            shutil.copyfile(args.source / name, work / name)
        shutil.copyfile(Path(__file__).with_name("space_charge_readout.lua"), work / "repulsion1.lua")
        (work / "space_charge_case.txt").write_text(
            f"{'coulomb' if args.mode == 'coulombic' else 'factor'} {args.amount:.17g} {args.quality}\n")
        proc = subprocess.run(
            ["/mnt/c/Program Files/SIMION-2024/simion.exe", "--nogui", "--noprompt",
             "fly", "--programs", "1", "repulsion1.iob"],
            cwd=work, stdin=subprocess.DEVNULL, capture_output=True, text=True, timeout=25)
        log.write_text(proc.stdout + proc.stderr)
        if proc.returncode:
            raise RuntimeError(f"SIMION exit {proc.returncode}: {proc.stdout[-3000:]} {proc.stderr}")
    text = log.read_text()
    starts, finals = {}, {}
    for line in text.splitlines():
        if line.startswith("START,"):
            v = list(map(float, line.split(",")[1:]))
            starts[int(v[0])] = v[1:]
        if line.startswith("FINAL,"):
            v = list(map(float, line.split(",")[1:]))
            finals[int(v[0])] = v[1:]
    if not starts or starts.keys() != finals.keys():
        raise RuntimeError(f"incomplete endpoints: {len(starts)} starts, {len(finals)} finals; see {log}")
    # SIMION's "TOF" variable is absolute time, unlike native lifetime TOF.
    for i, final in finals.items():
        final[0] -= starts[i][0]
    ion = work / "replay.ion"
    with ion.open("w") as f:
        for i in sorted(starts):
            tob, mass, charge, x, y, z, vx, vy, vz, ke, cwf = starts[i]
            az = math.degrees(math.atan2(-vz, vx))
            el = math.degrees(math.atan2(vy, math.hypot(vx, vz)))
            # Replay the measured velocity, not SIMION's mass/charge constant
            # through its reported energy. Native uses the CODATA constants.
            electric_acceleration = 1.602176634e-19 / 1.66053906892e-27 * 1e-6
            ke = mass * (vx*vx + vy*vy + vz*vz) / (2*electric_acceleration)
            f.write(",".join(format(v, ".17g") for v in [tob, mass, charge, x, y, z, az, el, ke, cwf]) + "\n")
    match = re.search(r"Fly completed\.\s+\d+ splats,\s+([\d.]+) seconds", text)
    result = {"mode": args.mode, "amount": args.amount, "quality": args.quality, "particles": len(starts),
              "simion_compute_s": float(match[1]) if match else None}
    if args.native:
        native_csv = work / f"native-{args.max_step:.0e}.csv"
        command = [str(args.native.resolve()), "fly", "--pa", str(work / "blank.pa"),
                   "--particles", str(ion), "--origin=-10,0,0", "--scale", "0.1",
                   "--bounds=-10,-4,-4,10,4,4", "--duration-us", "1",
                   "--space-charge", "direct", "--charge-mode", args.mode, "--charge-amount", str(args.amount),
                   "--max-step-us", str(args.max_step), "--tolerance", "1e-8", "--nonrelativistic",
                   "--output", str(native_csv)]
        proc = subprocess.run(command, stdin=subprocess.DEVNULL, capture_output=True, text=True,
                              env={**os.environ, "RAYON_NUM_THREADS": "4"}, timeout=300)
        if proc.returncode:
            raise RuntimeError(f"native exit {proc.returncode}: {proc.stderr}")
        native = list(csv.DictReader(native_csv.open()))
        errors = []
        tof_errors = []
        for row in native:
            ref = finals[int(row["ion"])]
            errors.append(math.sqrt(sum((float(row[a])-v)**2 for a,v in zip(["x_mm","y_mm","z_mm"],ref[1:4]))))
            tof_errors.append(float(row["tof_us"])-ref[0])
        timing = re.search(r"fly_s:\s*([\d.]+)", proc.stderr)
        result.update(native_compute_s=float(timing[1]) if timing else None,
                      endpoint_rms_mm=math.sqrt(statistics.mean(e*e for e in errors)),
                      endpoint_max_mm=max(errors),
                      tof_rms_us=math.sqrt(statistics.mean(t*t for t in tof_errors)),
                      simion_mean_tof_us=statistics.mean(r[0] for r in finals.values()),
                      native_mean_tof_us=statistics.mean(float(r["tof_us"]) for r in native),
                      native_stderr=proc.stderr,
                      native_command=command)
    (work / f"comparison-{args.max_step:.0e}.json").write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps(result, indent=2))


if __name__ == "__main__":
    main()
