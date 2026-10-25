#!/usr/bin/env python3
"""Record SIMION beam endpoints, replay identical launches, and report statistics.

Original examples are never changed. Each Windows invocation is foreground,
uses a C-drive working directory and closed stdin, and has a bounded timeout.
"""
import argparse
import csv
import datetime
import json
import math
import os
from pathlib import Path
import re
import shutil
import statistics
import subprocess

from run_oracle import DEFAULT_SIMION, kill_simion_started_after


# Shared SIMION oracle working directory; required via SIMREP_ORACLE_CACHE
# or explicit --source/--work.
def oracle_cache():
    root = os.environ.get("SIMREP_ORACLE_CACHE")
    if root:
        return Path(root)
    raise SystemExit(
        "SIMREP_ORACLE_CACHE is not set and --source/--work were not given. "
        "Set SIMREP_ORACLE_CACHE to the directory that holds the oracle "
        "working data (default layout: $SIMREP_ORACLE_CACHE/repulsion for "
        "--source and $SIMREP_ORACLE_CACHE/beam-endpoints for --work), or "
        "pass both --source and --work explicitly.")


def rms(values):
    return math.sqrt(statistics.mean(v*v for v in values))


def records(text, prefix):
    result = {}
    for line in text.splitlines():
        if line.startswith(prefix + ","):
            values = list(map(float, line.split(",")[1:]))
            result[int(values[0])] = values[1:]
    return result


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--source", type=Path, default=None,
                    help="repulsion1 inputs (default: $SIMREP_ORACLE_CACHE/repulsion)")
    ap.add_argument("--work", type=Path, default=None,
                    help="output directory (default: $SIMREP_ORACLE_CACHE/beam-endpoints)")
    ap.add_argument("--case", choices=["repulsion1", "repulsion_mixed_beam"], default="repulsion1")
    ap.add_argument("--current", type=float, default=1e-5)
    ap.add_argument("--native", type=Path)
    ap.add_argument("--native-only", action="store_true")
    ap.add_argument("--max-step", type=float, nargs="+", default=[1e-4, 5e-5, 2.5e-5])
    args = ap.parse_args()
    if args.source is None or args.work is None:
        cache = oracle_cache()
        if args.source is None:
            args.source = cache / "repulsion"
        if args.work is None:
            args.work = cache / "beam-endpoints"
    work = args.work / args.case
    work.mkdir(parents=True, exist_ok=True)
    log = work / "oracle.txt"
    if not args.native_only:
        for name in ["blank.pa", "repulsion1.iob"]:
            shutil.copyfile(args.source / name, work / name)
        shutil.copyfile(args.source / (args.case + ".fly2"), work / "repulsion1.fly2")
        shutil.copyfile(Path(__file__).with_name("beam_readout.lua"), work / "repulsion1.lua")
        started = datetime.datetime.now(datetime.timezone.utc)
        try:
            proc = subprocess.run(
                [DEFAULT_SIMION, "--nogui", "--noprompt",
                 "fly", "--programs", "1", "--grouped", "1", "--omit-relativistic-effects", "1",
                 "--repulsion", "beam", "--repulsion-amount",
                 str(args.current), "--trajectory-quality", "-9", "repulsion1.iob"],
                cwd=work, stdin=subprocess.DEVNULL, capture_output=True, text=True, timeout=30)
        except subprocess.TimeoutExpired:
            kill_simion_started_after(started)
            raise
        log.write_text(proc.stdout + proc.stderr)
        if proc.returncode:
            raise RuntimeError(f"SIMION exit {proc.returncode}: {proc.stdout[-3000:]} {proc.stderr}")
    text = log.read_text()
    starts, finals = records(text, "BEAM_START"), records(text, "BEAM_FINAL")
    if not starts or starts.keys() != finals.keys():
        raise RuntimeError(f"incomplete beam endpoints: {len(starts)} starts, {len(finals)} finals")
    for i, final in finals.items():
        final[0] -= starts[i][0]  # SIMION records absolute time, native TOF is lifetime.
    ion = work / "replay.ion"
    with ion.open("w") as f:
        for i in sorted(starts):
            tob, mass, charge, x, y, z, vx, vy, vz, _ke, cwf = starts[i]
            az = math.degrees(math.atan2(-vz, vx))
            el = math.degrees(math.atan2(vy, math.hypot(vx, vz)))
            acceleration = 1.602176634e-19 / 1.66053906892e-27 * 1e-6
            ke = mass * (vx*vx + vy*vy + vz*vz) / (2*acceleration)
            f.write(",".join(format(v, ".17g") for v in [tob, mass, charge, x, y, z, az, el, ke, cwf]) + "\n")
    match = re.search(r"Fly completed\.\s+\d+ splats,\s+([\d.]+) seconds", text)
    result = {"case": args.case, "particles": len(starts), "current_a": args.current,
              "semantics": "SIMION-documented beam semantics",
              "simion_compute_s": float(match[1]) if match else None,
              "simion_rms_radius_mm": rms(math.hypot(r[2], r[3]) for r in finals.values()),
              "native": []}
    previous = None
    if args.native:
        for dt in args.max_step:
            native_csv = work / f"native-{dt:.0e}.csv"
            command = [str(args.native.resolve()), "fly", "--pa", str(work / "blank.pa"),
                       "--particles", str(ion), "--origin=-10,0,0", "--scale", "0.1",
                       "--bounds=-10,-4,-4,10,4,4", "--duration-us", "1", "--device", "cpu",
                       "--charge-mode", "beam", "--beam-current-a", str(args.current),
                       "--max-step-us", str(dt), "--tolerance", "1e-8", "--nonrelativistic",
                       "--output", str(native_csv)]
            proc = subprocess.run(command, stdin=subprocess.DEVNULL, capture_output=True, text=True,
                                  env={**os.environ, "RAYON_NUM_THREADS": "4"}, timeout=180)
            if proc.returncode:
                raise RuntimeError(f"native exit {proc.returncode}: {proc.stderr}")
            native = list(csv.DictReader(native_csv.open()))
            if len(native) != len(finals):
                raise RuntimeError("native endpoint population differs")
            pos, vel, tof = [], [], []
            for row in native:
                ref = finals[int(row["ion"])]
                pos.append(math.dist([float(row[a]) for a in ["x_mm", "y_mm", "z_mm"]], ref[1:4]))
                vel.append(math.dist([float(row[a]) for a in ["vx_mm_per_us", "vy_mm_per_us", "vz_mm_per_us"]], ref[4:7]))
                tof.append(float(row["tof_us"]) - ref[0])
            timing = re.search(r"fly_s:\s*([\d.]+)", proc.stderr)
            record = {"max_step_us": dt, "endpoint_rms_mm": rms(pos), "endpoint_max_mm": max(pos),
                      "velocity_rms_mm_per_us": rms(vel), "tof_rms_us": rms(tof),
                      "rms_radius_mm": rms(math.hypot(float(r["y_mm"]), float(r["z_mm"])) for r in native),
                      "compute_s": float(timing[1]) if timing else None,
                      "terminations": {r: sum(row["reason"] == r for row in native) for r in sorted({row["reason"] for row in native})},
                      "command": command, "stderr": proc.stderr}
            if previous:
                record["self_endpoint_rms_change_mm"] = rms(
                    math.dist([float(a[k]) for k in ["x_mm", "y_mm", "z_mm"]],
                              [float(b[k]) for k in ["x_mm", "y_mm", "z_mm"]])
                    for a, b in zip(previous, native))
            previous = native
            result["native"].append(record)
    (work / "comparison.json").write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps(result, indent=2))


if __name__ == "__main__":
    main()
