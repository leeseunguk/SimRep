#!/usr/bin/env python3
"""Report quadrupole errors; a 4x reference is NOT spatially converged."""
import argparse
import csv
import json
import math
from pathlib import Path


def error(actual, expected):
    if actual.keys() != expected.keys():
        raise ValueError("particle IDs differ")
    distances = [math.dist(actual[i][1:4], expected[i][1:4]) for i in actual]
    times = [abs(actual[i][0] - expected[i][0]) for i in actual]
    return {
        "ions": len(actual),
        "endpoint_rms_mm": math.sqrt(sum(d*d for d in distances)/len(actual)),
        "endpoint_max_mm": max(distances),
        "tof_rms_us": math.sqrt(sum(t*t for t in times)/len(actual)),
        "tof_max_us": max(times),
    }


def native(path):
    with path.open() as stream:
        return {int(r["ion"]): [float(r[k]) for k in ("tof_us", "x_mm", "y_mm", "z_mm")]
                for r in csv.DictReader(stream)}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("work", type=Path)
    args = parser.parse_args()
    reference = native(args.work / "reference-4-0.025.csv")
    result = {"reference_spatially_converged": False}
    for name in ("cpu-1000.csv", "gpu-1000.csv", "reference-2-0.05.csv",
                 "reference-4-0.05.csv"):
        result[name] = error(native(args.work / name), reference)
    for quality in (3, 209):
        run = json.loads((args.work / f"quality-{quality}-readout.json").read_text())
        rows = [line.split(",")[1:] for line in run["stdout"].splitlines()
                if line.startswith("FINAL,")]
        states = {int(row[0]): list(map(float, row[1:5])) for row in rows}
        result[f"simion-{quality}"] = error(states, reference)
        compute = json.loads((args.work / f"quality-{quality}-compute.json").read_text())
        result[f"simion-{quality}"]["compute_s"] = compute["compute_s"]
    print(json.dumps(result, indent=2))


if __name__ == "__main__":
    main()
