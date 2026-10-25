#!/usr/bin/env python3
"""Compare terminal native CSVs without modifying the preserved 8x reference."""

import argparse
import csv
import hashlib
import json
import math
from pathlib import Path


def records(path):
    with path.open(newline="") as stream:
        rows = list(csv.DictReader(stream))
    if not rows:
        raise ValueError(f"empty flight CSV: {path}")
    return rows


def position(row):
    return tuple(float(row[f"{a}_mm"]) for a in "xyz")


def distance(a, b):
    return math.dist(position(a), position(b))


def summary(values):
    return {"max": max(values), "rms": math.sqrt(math.fsum(x*x for x in values)/len(values))}


def surface(row, bounds):
    if row["reason"] != "outside":
        return row["reason"]
    xyz = position(row)
    face = min(range(6), key=lambda k: abs(xyz[k % 3]-bounds[k]))
    return f"{'xyz'[face % 3]}{'-' if face < 3 else '+'}"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("cpu", "gpu", "reference", "output"):
        parser.add_argument(f"--{name}", type=Path, required=True)
    parser.add_argument("--bounds", default="-101,-19,-19,291,19,19")
    args = parser.parse_args()
    cpu, gpu, reference = (records(path) for path in (args.cpu, args.gpu, args.reference))
    if not (len(cpu) == len(gpu) == len(reference)):
        raise ValueError("CSV row counts differ")
    bounds = [float(x) for x in args.bounds.split(",")]
    if len(bounds) != 6:
        raise ValueError("bounds requires six coordinates")
    rows = []
    for c, g, r in zip(cpu, gpu, reference):
        if not c["ion"] == g["ion"] == r["ion"]:
            raise ValueError("particle order differs")
        if g["reason"] not in ("outside", "electrode", "plane"):
            raise ValueError(f"ion {g['ion']} terminated numerically: {g['reason']}")
        rows.append({
            "ion": g["ion"],
            "gpu_cpu_mm": distance(g, c),
            "gpu_reference_mm": distance(g, r),
            "cpu_reference_mm": distance(c, r),
            "gpu_cpu_tof_us": abs(float(g["tof_us"])-float(c["tof_us"])),
            "gpu_surface": surface(g, bounds),
            "cpu_surface": surface(c, bounds),
        })
    with args.output.open("w", newline="") as stream:
        writer = csv.DictWriter(stream, fieldnames=rows[0].keys())
        writer.writeheader()
        writer.writerows(rows)
    metrics = {key: summary([row[key] for row in rows]) for key in
               ("gpu_cpu_mm", "gpu_reference_mm", "cpu_reference_mm", "gpu_cpu_tof_us")}
    agreement = sum(row["gpu_surface"] == row["cpu_surface"] for row in rows)
    metrics.update(
        ions=len(rows), surface_agreement=agreement,
        reference_sha256=hashlib.sha256(args.reference.read_bytes()).hexdigest(),
        acceptance_rms_mm=1.1*metrics["cpu_reference_mm"]["rms"],
    )
    print(json.dumps(metrics, indent=2))
    if metrics["gpu_reference_mm"]["rms"] > metrics["acceptance_rms_mm"]:
        raise SystemExit("FAIL: GPU reference RMS exceeds CPU coarse RMS by more than 10%")
    if agreement != len(rows):
        raise SystemExit("FAIL: GPU/CPU splat surfaces differ")


if __name__ == "__main__":
    main()
