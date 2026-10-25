#!/usr/bin/env python3
"""Recheck PA collision changes against preserved RF/TOF converged references."""
import argparse
import json
import re
import subprocess
from pathlib import Path

from rf_tof_report import error, native_state, rows


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--native", type=Path, default=Path("target/release/examples/rf_tof"))
    parser.add_argument("--root", type=Path,
                        default=Path("/mnt/c/Users/lsu10/simrep-oracle/_rf_tof"))
    parser.add_argument("--output", type=Path, default=Path("target/collision-rf-tof.json"))
    args = parser.parse_args()
    old = json.loads((args.root / "cpu-measurements.json").read_text())
    tof_reference = next(r for r in old if r["case"] == "tof" and r["factor"] == 8
                         and r["step_us"] == 0.000625 and not r["validation"])
    result = {}
    for name in ("tof", "rf_mirror", "rf_square_wave"):
        before = next(r for r in old if r["case"] == name and r["factor"] == 1
                      and r["step_us"] == 0.02 and not r["validation"])
        if name == "tof":
            reference = native_state(tof_reference)
        else:
            validation = next(r for r in old if r["case"] == name and r["validation"])
            reference = [list(map(float, line.split(",")[2:6]))
                         for line in validation["stdout"].splitlines()
                         if line.startswith("REFERENCE,")]
        argv = [str(args.native), str(args.root), name, "cpu", "1", "0.02", "1", "1e-12"]
        run = subprocess.run(argv, capture_output=True, text=True, timeout=120, check=True)
        current = {"argv": argv, "exit_code": run.returncode,
                   "stdout": run.stdout, "stderr": run.stderr}
        states = native_state(current)
        case = json.loads((args.root / name / "case.json").read_text())
        data = {
            "before_vs_reference": error(native_state(before), reference),
            "after_vs_reference": error(states, reference),
            "after_vs_before": error(states, native_state(before)),
            "full_numeric_rows_equal": rows(current) == rows(before),
            "after_vs_simion": error(states, case["finals"]),
            "before_compute_s": float(re.search(r"compute_s=([\d.]+)", before["stderr"])[1]),
            "after_compute_s": float(re.search(r"compute_s=([\d.]+)", run.stderr)[1]),
        }
        result[name] = {"comparison": data, "run": current}
        print(name, json.dumps(data), flush=True)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, indent=2))


if __name__ == "__main__":
    main()
