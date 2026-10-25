#!/usr/bin/env python3
"""Serial native measurements; stdout/stderr and process walls stay in JSON."""
import argparse
import json
import os
import subprocess
import sys
import time
from run_oracle import win_path


# Shared SIMION oracle working directory; required via SIMREP_ORACLE_CACHE
# or --root.
def oracle_cache():
    root = os.environ.get("SIMREP_ORACLE_CACHE")
    if root:
        return root
    raise SystemExit(
        "SIMREP_ORACLE_CACHE is not set and --root was not given. "
        "Set SIMREP_ORACLE_CACHE to the directory holding the rf_tof oracle "
        "data (default layout: $SIMREP_ORACLE_CACHE/_rf_tof), or pass --root "
        "pointing at that directory.")


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--native", required=True)
    ap.add_argument("--device", choices=["cpu", "gpu"], required=True)
    ap.add_argument("--root", default=None,
                    help="rf_tof oracle data directory (default: $SIMREP_ORACLE_CACHE/_rf_tof)")
    ap.add_argument("--grid-study", action="store_true")
    ap.add_argument("--output", help="optional evidence JSON basename")
    args = ap.parse_args()
    if args.root is None:
        args.root = os.path.join(oracle_cache(), "_rf_tof")
    native = os.path.abspath(args.native)
    root = win_path(args.root) if native.endswith(".exe") else args.root
    jobs = [(case, 1, 0.02, 1e-12, False) for case in ["tof", "rf_mirror", "rf_square_wave"]]
    if args.grid_study:
        if args.device == "cpu":
            jobs += [("tof", factor, step, 1e-13, False) for factor, step in [(1, 0.00125), (2, 0.00125), (4, 0.00125), (8, 0.00125), (8, 0.000625)]]
        jobs += [(case, 1, 0.02, 1e-12, True) for case in ["tof", "rf_mirror", "rf_square_wave"]]
        if args.device == "cpu":
            jobs += [("tof", 8, 0.000625, 1e-13, True)]
    results = []
    for case, factor, step, tolerance, validate in jobs:
        command = [native, root, case, args.device, str(factor), str(step), "1", str(tolerance)]
        if validate:
            command.append("validate")
        start = time.perf_counter()
        p = subprocess.run(command, cwd="/mnt/c", stdin=subprocess.DEVNULL,
                           capture_output=True, text=True, timeout=120)
        results.append({"case": case, "factor": factor, "step_us": step, "validation": validate,
                        "tolerance": tolerance, "argv": command, "exit_code": p.returncode,
                        "process_wall_s": time.perf_counter() - start,
                        "stdout": p.stdout, "stderr": p.stderr})
        print(case, factor, step, "exit", p.returncode, "wall", results[-1]["process_wall_s"])
        print(p.stderr)
        if p.returncode:
            print(p.stdout)
            break
    output = os.path.join(args.root, args.output or args.device + "-measurements.json")
    with open(output, "w", encoding="utf-8") as f:
        json.dump(results, f, indent=2)
    return 0 if all(r["exit_code"] == 0 for r in results) else 1


if __name__ == "__main__":
    sys.exit(main())
