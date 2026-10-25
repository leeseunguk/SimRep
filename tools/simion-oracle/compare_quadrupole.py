#!/usr/bin/env python3
"""Compare terminal states; keep numerical references and physical geometry explicit."""
import argparse
import csv
import json
import math
from pathlib import Path


def load(path):
    if path.suffix == ".json":
        data = json.loads(path.read_text())
        return {
            int(row[1]): list(map(float, row[2:6]))
            for line in data["stdout"].splitlines()
            if line.startswith("FINAL,")
            for row in [line.split(",")]
        }
    with path.open() as stream:
        return {
            int(row["ion"]): [float(row[key]) for key in ("tof_us", "x_mm", "y_mm", "z_mm")]
            for row in csv.DictReader(stream)
        }


def compare(actual, reference, indices=None):
    if actual.keys() != reference.keys():
        raise ValueError("Particle IDs differ")
    indices = list(actual if indices is None else indices)
    distances = [math.dist(actual[i][1:], reference[i][1:]) for i in indices]
    times = [abs(actual[i][0] - reference[i][0]) for i in indices]
    return {
        "ions": len(indices),
        "endpoint_rms_mm": math.sqrt(sum(d * d for d in distances) / len(distances)),
        "endpoint_max_mm": max(distances),
        "tof_rms_us": math.sqrt(sum(t * t for t in times) / len(times)),
        "tof_max_us": max(times),
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("reference", type=Path)
    parser.add_argument("candidates", type=Path, nargs="+")
    parser.add_argument("--clearance", type=Path)
    parser.add_argument("--clearance-column", default="min_cell_segment_clearance_mm")
    args = parser.parse_args()
    reference = load(args.reference)
    clearances = None
    if args.clearance:
        with args.clearance.open() as stream:
            clearances = {
                int(row["ion"]): float(row[args.clearance_column])
                for row in csv.DictReader(stream)
            }
    result = {"reference": str(args.reference)}
    for path in args.candidates:
        actual = load(path)
        item = compare(actual, reference)
        same_wall = [
            i for i in actual
            if (actual[i][1] > 59.999) == (reference[i][1] > 59.999)
        ]
        item["exit_changed"] = len(actual) - len(same_wall)
        if same_wall:
            item["same_exit_class"] = compare(actual, reference, same_wall)
        if clearances:
            edges = [0.0, 1e-8, 0.25, 0.5, 1.0, 2.0, 4.0, math.inf]
            item[f"by_{args.clearance_column}"] = {
                f"{lo:g}..{hi:g}": compare(actual, reference, indices)
                for lo, hi in zip(edges, edges[1:])
                for indices in [[i for i, d in clearances.items() if lo <= d < hi]]
                if indices
            }
        result[str(path)] = item
    print(json.dumps(result, indent=2))


if __name__ == "__main__":
    main()
