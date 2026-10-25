#!/usr/bin/env python3
"""Stage readout-only oracle runs and compare their per-ion CSV records."""

import argparse
import csv
import json
import math
import pathlib
import re
import shutil


def stdout_text(path):
    text = path.read_text()
    if path.suffix == ".json":
        return "\n".join(r["stdout"] for r in json.loads(text)["runs"] if r["step"] == "fly")
    return text


def prepare(args):
    args.destination.mkdir(parents=True, exist_ok=True)
    for pattern in ("*.pa0", "*.pa[1-9]*", "*.pa_", "*.iob", "*.fly", "*.rec"):
        for source in args.source.glob(pattern):
            shutil.copyfile(source, args.destination / source.name)
    shutil.copyfile(pathlib.Path(__file__).with_name("fly_reference.lua"),
                    args.destination / "einzel.lua")
    if (args.source / "testplanelib.lua").exists():
        shutil.copyfile(args.source / "testplanelib.lua",
                        args.destination / "testplanelib.lua")
    print(args.destination)


def extract(args):
    text = stdout_text(args.oracle)
    starts = [line.split(",")[1:] for line in text.splitlines() if line.startswith("START,")]
    with args.output.open("w", newline="") as file:
        writer = csv.writer(file)
        writer.writerow(["; TOB", "MASS", "CHARGE", "X", "Y", "Z", "AZ", "EL", "KE", "CWF", "COLOR"])
        for _, birth, mass, charge, x, y, z, vx, vy, vz, ke in starts:
            vx, vy, vz = map(float, (vx, vy, vz))
            # SIMION azimuth is toward -z; elevation is toward +y.
            azimuth = math.degrees(math.atan2(-vz, vx))
            elevation = math.degrees(math.atan2(vy, math.hypot(vx, vz)))
            writer.writerow([birth, mass, charge, x, y, z, azimuth, elevation, ke, 1, 1])
    print(f"extracted {len(starts)} ions to {args.output}")


def compare(args):
    text = stdout_text(args.oracle)
    reference = {}
    if args.event == "stdout":
        pattern = r"status2,\s*(\d+) Splat: xyz\(\s*([^,]+),\s*([^,]+),\s*([^)]+)\)"
        reference = {int(n): [float(x), float(y), float(z)]
                     for n, x, y, z in re.findall(pattern, text)}
        keys = ("x_mm", "y_mm", "z_mm")
    else:
        for line in text.splitlines():
            if line.startswith(args.event + ","):
                values = line.split(",")
                reference[int(values[1])] = list(map(float, values[2:]))
        keys = ("tof_us", "x_mm", "y_mm", "z_mm", "ke_ev")
    with args.native.open(newline="") as file:
        native = {int(row["ion"]): row for row in csv.DictReader(file)}
    if reference.keys() != native.keys():
        raise ValueError(f"ion IDs differ: oracle={len(reference)}, native={len(native)}")
    differences = {key: [] for key in keys}
    position_errors = []
    for n, expected in reference.items():
        delta = [float(native[n][key]) - value for key, value in zip(keys, expected)]
        for key, value in zip(keys, delta):
            differences[key].append(value)
        position_errors.append(math.sqrt(sum(
            (float(native[n][key]) - expected[keys.index(key)]) ** 2
            for key in ("x_mm", "y_mm", "z_mm"))))
    def summary(values):
        return {"max_abs": max(map(abs, values)), "rms": math.sqrt(sum(v*v for v in values)/len(values))}
    result = {"ions": len(reference), "event": args.event,
              "position_mm": summary(position_errors),
              "differences": {key: summary(values) for key, values in differences.items()}}
    print(json.dumps(result, indent=2))


def subset(args):
    text = stdout_text(args.oracle)
    expected = {int(v[1]): list(map(float, v[3:6]))
                for line in text.splitlines() if line.startswith("FINAL,")
                for v in [line.split(",")]}
    with args.native.open(newline="") as file:
        native = {int(row["ion"]): row for row in csv.DictReader(file)}
    errors = sorted(expected, key=lambda n: math.dist(expected[n], [
        float(native[n][key]) for key in ("x_mm", "y_mm", "z_mm")]), reverse=True)
    rows = [line for line in args.particles.read_text().splitlines()
            if line.strip() and not line.lstrip().startswith(";")]
    selected = set(errors[:100])
    selected.update(round(i * (len(rows) - 1) / 899) + 1 for i in range(900))
    for n in errors:
        if len(selected) == 1000:
            break
        selected.add(n)
    selected = sorted(selected)
    args.output.write_text("; TOB,MASS,CHARGE,X,Y,Z,AZ,EL,KE,CWF,COLOR\n"
                           + "\n".join(rows[n-1] for n in selected) + "\n")
    args.output.with_suffix(".ids").write_text("\n".join(map(str, selected)) + "\n")
    print(f"selected {len(selected)} ions, including worst 100; {args.output}")


def classify(args):
    expected = {int(v[1]): list(map(float, v[2:]))
                for line in stdout_text(args.oracle).splitlines() if line.startswith("FINAL,")
                for v in [line.split(",")]}
    with args.native.open(newline="") as file:
        native = {int(row["ion"]): row for row in csv.DictReader(file)}
    with args.clearance.open(newline="") as file:
        clearance = {int(row["ion"]): row for row in csv.DictReader(file)}
    if expected.keys() != native.keys() or expected.keys() != clearance.keys():
        raise ValueError("classification requires matching per-ion IDs")
    def face(p):
        if abs(p[1] - 291) < 1e-4:
            return "exit_x"
        if abs(abs(p[2]) - 19) < 1e-4:
            return "side_y"
        return "electrode_or_other"
    records = []
    outcomes = {}
    for n, ref in expected.items():
        row = native[n]
        position = [float(row[key]) for key in ("x_mm", "y_mm", "z_mm")]
        native_face = face([n, *position]) if row["reason"] == "outside" else row["reason"]
        oracle_face = face([n, *ref[1:4]])
        kind = f"{oracle_face}/{native_face}"
        outcomes[kind] = outcomes.get(kind, 0) + 1
        records.append((float(clearance[n]["initial_radius_mm"]),
                        float(clearance[n]["min_electrode_distance_mm"]),
                        math.dist(ref[1:4], position), abs(ref[0] - float(row["tof_us"]))))
    def histogram(column, edges):
        result = []
        for low, high in zip(edges, edges[1:]):
            group = [r for r in records if low <= r[column] < high]
            if group:
                result.append({"range_mm": [low, high], "ions": len(group),
                               "position_max_mm": max(r[2] for r in group),
                               "position_rms_mm": math.sqrt(sum(r[2]**2 for r in group)/len(group)),
                               "tof_max_us": max(r[3] for r in group),
                               "tof_rms_us": math.sqrt(sum(r[3]**2 for r in group)/len(group))})
        return result
    print(json.dumps({"outcomes_oracle_native": outcomes,
                      "minimum_clearance_mm": min(r[1] for r in records),
                      "by_initial_radius": histogram(0, [0, 3, 6, 9, 12, 14, 16]),
                      "by_electrode_clearance": histogram(1, [0, .1, .5, 1, 2, 4, 8, 20])}, indent=2))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(required=True)
    command = commands.add_parser("prepare")
    command.add_argument("source", type=pathlib.Path)
    command.add_argument("destination", type=pathlib.Path)
    command.set_defaults(run=prepare)
    command = commands.add_parser("extract")
    command.add_argument("oracle", type=pathlib.Path)
    command.add_argument("output", type=pathlib.Path)
    command.set_defaults(run=extract)
    command = commands.add_parser("compare")
    command.add_argument("oracle", type=pathlib.Path)
    command.add_argument("native", type=pathlib.Path)
    command.add_argument("--event", choices=("FINAL", "PLANE3", "stdout"), default="FINAL")
    command.set_defaults(run=compare)
    command = commands.add_parser("subset")
    command.add_argument("oracle", type=pathlib.Path)
    command.add_argument("native", type=pathlib.Path)
    command.add_argument("particles", type=pathlib.Path)
    command.add_argument("output", type=pathlib.Path)
    command.set_defaults(run=subset)
    command = commands.add_parser("classify")
    command.add_argument("oracle", type=pathlib.Path)
    command.add_argument("native", type=pathlib.Path)
    command.add_argument("clearance", type=pathlib.Path)
    command.set_defaults(run=classify)
    args = parser.parse_args()
    args.run(args)


if __name__ == "__main__":
    main()
