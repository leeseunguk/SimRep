#!/usr/bin/env python3
"""One-source-at-a-time GPU rounding ablations against preserved references."""
import argparse
import json
import os
import re
import subprocess
import time

from rf_tof_report import error, native_state
from run_oracle import win_path


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--native", required=True)
    parser.add_argument("--root", default="/mnt/c/Users/lsu10/simrep-oracle/_rf_tof")
    parser.add_argument("--mode", action="append", required=True)
    parser.add_argument("--case", action="append", choices=["tof", "rf_mirror", "rf_square_wave"])
    parser.add_argument("--max-step", type=float, default=0.02)
    parser.add_argument("--tolerance", type=float, default=1e-12)
    parser.add_argument("--tag", default="")
    parser.add_argument("--copies", type=int, default=1)
    parser.add_argument("--device", choices=["cpu", "gpu"], default="gpu")
    args = parser.parse_args()
    with open(os.path.join(args.root, "cpu-measurements.json"), encoding="utf-8") as f:
        cpu = json.load(f)
    reference = {}
    for case in args.case or ["tof", "rf_mirror", "rf_square_wave"]:
        if case == "tof":
            reference[case] = native_state(next(r for r in cpu if r["case"] == case
                                               and r["factor"] == 8
                                               and r["step_us"] == 0.000625
                                               and not r["validation"]))
        else:
            validation = next(r for r in cpu if r["case"] == case and r["validation"])
            reference[case] = [list(map(float, line.split(",")[2:6]))
                               for line in validation["stdout"].splitlines()
                               if line.startswith("REFERENCE,")]
    env = os.environ.copy()
    env["WSLENV"] = env.get("WSLENV", "") + ":SIMREP_FLY_ROUND"
    for mode in args.mode:
        env["SIMREP_FLY_ROUND"] = mode
        results = []
        for case, ref in reference.items():
            argv = [os.path.abspath(args.native), win_path(args.root), case, args.device,
                    "1", str(args.max_step), str(args.copies), str(args.tolerance)]
            start = time.perf_counter()
            p = subprocess.run(argv, env=env, cwd="/mnt/c", stdin=subprocess.DEVNULL,
                               capture_output=True, text=True, timeout=40)
            run = {"case": case, "mode": mode, "device": args.device, "copies": args.copies,
                   "argv": argv, "exit_code": p.returncode,
                   "process_wall_s": time.perf_counter() - start,
                   "stdout": p.stdout, "stderr": p.stderr}
            if p.returncode:
                print(p.stdout, p.stderr, flush=True)
                raise RuntimeError(f"{mode}/{case} exited {p.returncode}")
            run["vs_reference"] = error(native_state(run), ref)
            run["compute_s"] = float(re.search(r"compute_s=([\d.]+)", p.stderr)[1])
            results.append(run)
            print(mode, case, "compute_s", run["compute_s"], run["vs_reference"], flush=True)
            if mode == "profile":
                print(p.stderr, flush=True)
        with open(os.path.join(args.root, args.device + "-precision-" + mode + args.tag + ".json"), "w",
                  encoding="utf-8") as f:
            json.dump(results, f, indent=2)


if __name__ == "__main__":
    main()
