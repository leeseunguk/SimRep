#!/usr/bin/env python3
"""Uninstrumented T.Qual 209 compute calibration; preserve existing oracle logs."""
import argparse
import json
import re

from run_oracle import DEFAULT_SIMION, run_simion, win_path


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--particles", required=True)
    parser.add_argument("--ions", type=int, required=True)
    parser.add_argument("--output", required=True)
    parser.add_argument("--work", default="/mnt/c/Users/lsu10/simrep-oracle/_benchfly")
    args = parser.parse_args()
    run = run_simion(DEFAULT_SIMION, [
        "--nogui", "--noprompt", "--default-num-particles", str(args.ions + 1000),
        "fly", "--trajectory-quality", "209", "--recording-enable", "0",
        "--retain-trajectories", "0", "--particles", win_path(args.particles), "einzel.iob",
    ], args.work, 40)
    match = re.search(r"Fly completed\.\s+(\d+) splats,\s+([\d.]+) seconds", run["stdout"])
    if match:
        run["ions"] = int(match[1])
        run["compute_s"] = float(match[2])
    with open(args.output, "w", encoding="utf-8") as f:
        json.dump(run, f, indent=2)
    if run["exit_code"] != 0 or not match or run["ions"] != args.ions:
        print(run["stdout"], run["stderr"])
        raise RuntimeError("strict einzel compute calibration failed")
    print(f'SIMION T.Qual 209: ions={run["ions"]} compute_s={run["compute_s"]} '
          f'process_wall_s={run["seconds"]} evidence={args.output}')


if __name__ == "__main__":
    main()
