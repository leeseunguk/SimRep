#!/usr/bin/env python3
"""Measure CLI defaults against retained, independent benchmark references.

No step/tolerance flags are supplied unless --legacy reconstructs the old CLI's
0.1 us ceiling and 1e-8 tolerances (1e-10 with --api-baseline).
"""
import argparse
import csv
import json
import math
import os
from pathlib import Path
import re
import statistics
import struct
import subprocess


ORACLE = Path("/mnt/c/Users/lsu10/simrep-oracle")
SOURCE = Path("/home/lsu10/orca/workspaces/SIMION/SIMION_Rep")


def rows(path):
    with path.open() as stream:
        return list(csv.DictReader(stream))


def errors(actual, reference):
    assert len(actual) == len(reference), (len(actual), len(reference))
    position = [
        math.dist([float(a[k]) for k in ("x_mm", "y_mm", "z_mm")],
                  [float(b[k]) for k in ("x_mm", "y_mm", "z_mm")])
        for a, b in zip(actual, reference)
    ]
    tof = [abs(float(a["tof_us"]) - float(b["tof_us"])) for a, b in zip(actual, reference)]
    return dict(position_max_mm=max(position),
                position_rms_mm=math.sqrt(statistics.mean(p*p for p in position)),
                tof_max_us=max(tof), tof_rms_us=math.sqrt(statistics.mean(t*t for t in tof)))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("native", type=Path)
    parser.add_argument("output", type=Path)
    parser.add_argument("--rf-example", type=Path)
    parser.add_argument("--legacy", action="store_true")
    parser.add_argument("--api-baseline", action="store_true",
                        help="Reconstruct the old API's 1e-10 tolerance on the old CLI")
    parser.add_argument("--reuse", action="store_true", help="Analyze existing captures in output")
    parser.add_argument("--repeats", type=int, default=1,
                        help="Serial repetitions; report median compute time")
    parser.add_argument("--before", type=Path, help="Compare with a prior capture directory")
    parser.add_argument("--only", choices=["einzel", "magnetic", "rf", "collision", "charge"])
    parser.add_argument("--einzel-10k", action="store_true",
                        help="Time the preserved 10k beam instead of the reference subset")
    args = parser.parse_args()
    if args.repeats < 1:
        parser.error("--repeats must be positive")
    if args.api_baseline and not args.legacy:
        parser.error("--api-baseline requires --legacy")
    args.output.mkdir(parents=True, exist_ok=True)
    native = str(args.native.resolve())
    saved = args.output / "results.json"
    results = json.loads(saved.read_text()) if saved.exists() else {}
    env = {**os.environ, "RAYON_NUM_THREADS": "16"}

    def run(name, command):
        if args.legacy:
            command += ["--max-step-us", "0.1", "--tolerance",
                        "1e-10" if args.api_baseline else "1e-8"]
        times = []
        for repeat in range(args.repeats):
            suffix = "" if repeat == 0 else f"-{repeat + 1}"
            log = args.output / f"{name}{suffix}.log"
            if args.reuse and log.exists():
                text = log.read_text()
            else:
                process = subprocess.run(command, capture_output=True, text=True, stdin=subprocess.DEVNULL,
                                         env=env, timeout=600)
                text = process.stdout + process.stderr
                log.write_text(text)
                if process.returncode:
                    raise RuntimeError(f"{name}: {process.returncode}\n{process.stderr}")
            match = re.search(r"(?:^|, )fly_s:\s*([\d.]+)", text, re.M)
            times.append(float(match[1]))
        return {"compute_s": statistics.median(times), "compute_samples_s": times,
                "command": command}

    if args.only in (None, "einzel"):
        name = "einzel-10k" if args.einzel_10k else "einzel"
        output = args.output / f"{name}.csv"
        results[name] = run(name, [
            native, "fly", "--device", "cpu", "--pa", str(ORACLE / "_benchfly/einzel.pa0"),
            "--particles", str(ORACLE / ("_benchfly/einzel_10000.ion" if args.einzel_10k
                                        else "_real_einzel/subset.ion")),
            "--voltage", "1=0,2=110,3=0", "--bounds=-101,-19,-19,291,19,19",
            "--output", str(output)])
        if not args.einzel_10k:
            results[name].update(errors(rows(output), rows(SOURCE / "target/fly-real/grid-8.csv")))
        print(name, json.dumps(results[name]), flush=True)

    if args.only in (None, "magnetic"):
        root = ORACLE / "physics-validation-20261023"
        for name in ("magnet1", "mag90", "sector_2dp", "sector_2dc", "sector_3dp", "helmholtz", "solenoid"):
            folder = root / name
            bounds = next(line.split(",")[1:] for line in (folder / "metadata.log").read_text().splitlines()
                          if line.startswith("BOUNDS,"))
            output = args.output / f"{name}.csv"
            command = [native, "fly", "--device", "cpu", "--particles", str(folder / "ions.ion"),
                       "--run-file", str(folder / "run.json"), "--bounds=" + ",".join(bounds),
                       "--nonrelativistic", "--duration-us", "0.03" if name == "helmholtz" else "10000",
                       "--output", str(output)]
            for line in (folder / "metadata.log").read_text().splitlines():
                if not line.startswith("INSTANCE,"):
                    continue
                fields = line.split(",")
                pa = folder / f"instance{fields[1]}.pa"
                with pa.open("rb") as stream:
                    magnetic = struct.unpack("<i", stream.read(32)[28:32])[0] & 8
                if not magnetic:
                    command += ["--pa", str(pa), "--origin=" + ",".join(fields[3:6]),
                                "--axes=" + ",".join(fields[6:15]), "--scale", fields[15]]
            if name == "helmholtz":
                command += ["--allow-time-limit"]
            results[name] = run(name, command)
            results[name].update(errors(rows(output), rows(folder / "native-reference.csv")))
            log = (folder / "flight.log").read_text()
            results[name]["simion_compute_s"] = float(re.search(r" splats, ([\d.]+) seconds", log)[1])
            finals = sorted((line.split(",") for line in log.splitlines() if line.startswith("FINAL,")),
                            key=lambda row: int(row[1]))
            oracle = [dict(zip(("tof_us", "x_mm", "y_mm", "z_mm"), map(float, row[2:6])))
                      for row in finals]
            starts = sorted((line.split(",") for line in log.splitlines() if line.startswith("START,")),
                            key=lambda row: int(row[1]))
            with (folder / "ions.ion").open() as stream:
                inputs = [list(map(float, row)) for row in csv.reader(stream)]
            matched = len(starts) == len(inputs)
            for start, ion in zip(starts, inputs):
                velocity = list(map(float, start[8:11]))
                azimuth, elevation = map(math.radians, ion[6:8])
                direction = [math.cos(elevation) * math.cos(azimuth), math.sin(elevation),
                             -math.cos(elevation) * math.sin(azimuth)]
                speed = math.hypot(*velocity)
                matched &= math.dist(velocity, [speed*d for d in direction]) <= max(speed, 1)*1e-6
                matched &= math.dist(list(map(float, start[5:8])), ion[3:6]) < 1e-9
            results[name]["simion_inputs_match"] = matched
            if matched:
                results[name]["simion_error"] = errors(oracle, rows(folder / "native-reference.csv"))
            print(name, json.dumps(results[name]), flush=True)

    if args.only in (None, "rf") and args.rf_example:
        for name in ("tof", "rf_mirror", "rf_square_wave"):
            command = [str(args.rf_example.resolve()), str(ORACLE / "_rf_tof"), name, "cpu", "1",
                       "0.1" if args.legacy else "default", "1"]
            if args.legacy:
                command += ["1e-10" if args.api_baseline else "1e-8"]
            times = []
            for repeat in range(args.repeats):
                suffix = "" if repeat == 0 else f"-{repeat + 1}"
                log = args.output / f"{name}{suffix}.log"
                if args.reuse and log.exists():
                    text = log.read_text()
                    actual = rows(args.output / f"{name}.csv")
                else:
                    process = subprocess.run(command, capture_output=True, text=True, stdin=subprocess.DEVNULL,
                                             env=env, timeout=120)
                    text = process.stdout + process.stderr
                    log.write_text(text)
                    if process.returncode:
                        raise RuntimeError(process.stderr)
                    csv_text = process.stdout[process.stdout.index("ion,tof_us,"):].splitlines()
                    actual = list(csv.DictReader(csv_text))
                    with (args.output / f"{name}.csv").open("w") as stream:
                        stream.write("\n".join(csv_text) + "\n")
                times.append(float(re.search(r" compute_s=([\d.]+)", text)[1]))
            results[name] = {"compute_s": statistics.median(times), "compute_samples_s": times,
                             "command": command}
            measurements = json.loads((ORACLE / "_rf_tof/cpu-measurements.json").read_text())
            if name == "tof":
                reference_run = next(r for r in measurements if r["case"] == name and r["factor"] == 8
                           and r["step_us"] == 0.000625 and not r["validation"])
                states = [list(map(float, line.split(",")[1:5])) for line in reference_run["stdout"].splitlines()
                          if re.match(r"^\d+,", line)]
            else:
                reference_run = next(r for r in measurements if r["case"] == name and r["validation"])
                states = [list(map(float, line.split(",")[2:6])) for line in reference_run["stdout"].splitlines()
                          if line.startswith("REFERENCE,")]
            reference = [dict(zip(("tof_us", "x_mm", "y_mm", "z_mm"), state)) for state in states]
            results[name].update(errors(actual, reference))
            case = json.loads((ORACLE / "_rf_tof" / name / "case.json").read_text())
            oracle = [dict(zip(("tof_us", "x_mm", "y_mm", "z_mm"), state[:4])) for state in case["finals"]]
            results[name]["simion_error"] = errors(oracle, reference)
            print(name, json.dumps(results[name]), flush=True)

    if args.only in (None, "collision"):
        from scipy.stats import ks_2samp
        root = ORACLE / "physics-validation-20261023"
        for name in ("hs1", "sds"):
            folder = root / ("collision_" + name)
            output = args.output / f"{name}.csv"
            command = [native, "fly", "--particles", str(folder / "ions.ion"),
                       "--collision", name, "--seed", "813", "--duration-us", "50",
                       "--allow-time-limit", "--output", str(output)]
            if name == "sds":
                command += ["--mbmr", str(ORACLE / "collision_sds/mbmr.dat"), "--temperature-k", "273.15"]
            results[name] = run(name, command)
            actual = rows(output)
            oracle = [list(map(float, line.split(",")[2:])) for line in (folder / "flight.log").read_text().splitlines()
                      if line.startswith("FINAL,")]
            results[name]["ks"] = {}
            if args.before:
                previous = rows(args.before / f"{name}.csv")
                results[name]["before_ks"] = {}
            for column, key in enumerate(("tof_us", "x_mm", "y_mm", "z_mm", "ke_ev")):
                a = [float(row[key]) for row in actual]
                b = [row[column] for row in oracle]
                if column == 0:
                    a, b = [round(v, 7) for v in a], [round(v, 7) for v in b]
                test = ks_2samp(a, b, method="exact")
                results[name]["ks"][key] = [float(test.statistic), float(test.pvalue)]
                if args.before:
                    c = [float(row[key]) for row in previous]
                    if column == 0:
                        c = [round(v, 7) for v in c]
                    test = ks_2samp(a, c, method="exact")
                    results[name]["before_ks"][key] = [float(test.statistic), float(test.pvalue)]
            print(name, json.dumps(results[name]), flush=True)

    if args.only in (None, "charge"):
        for mode, amount in (("factor", "1"), ("coulombic", "3.47e-13")):
            folder = ORACLE / "space-charge" / mode
            output = args.output / f"charge-{mode}.csv"
            results["charge-" + mode] = run("charge-" + mode, [
                native, "fly", "--pa", str(folder / "blank.pa"), "--particles", str(folder / "replay.ion"),
                "--origin=-10,0,0", "--scale", "0.1", "--bounds=-10,-4,-4,10,4,4",
                "--duration-us", "1", "--space-charge", "direct", "--charge-mode", mode,
                "--charge-amount", amount, "--nonrelativistic", "--output", str(output)])
            log = (folder / "oracle.txt").read_text()
            births = {int(line.split(",")[1]): float(line.split(",")[2])
                      for line in log.splitlines() if line.startswith("START,")}
            digest = "7e5c5347992d" if mode == "factor" else "9dc1c7e637bd"
            path = Path("/tmp/simrep-space-charge-reference") / mode / f"dop853-events-{digest}-0.00025.csv"
            with path.open() as stream:
                states = [list(map(float, row)) for row in csv.reader(stream)]
            reference = [
                dict(zip(("tof_us", "x_mm", "y_mm", "z_mm"), [state[0] - births[i+1], *state[1:4]]))
                for i, state in enumerate(states)]
            results["charge-" + mode].update(errors(rows(output), reference))
            results["charge-" + mode]["simion_compute_s"] = float(
                re.search(r" splats,\s*([\d.]+) seconds", log)[1])
            print("charge-" + mode, json.dumps(results["charge-" + mode]), flush=True)
    saved.write_text(json.dumps(results, indent=2) + "\n")
    if args.before:
        before = json.loads((args.before / "results.json").read_text())
        for name, current in results.items():
            if "position_rms_mm" in current:
                current["before_delta"] = errors(
                    rows(args.output / f"{name}.csv"), rows(args.before / f"{name}.csv"))
        saved.write_text(json.dumps(results, indent=2) + "\n")
        print("| Case | Before compute s | After compute s | Before endpoint RMS mm | After endpoint RMS mm | After TOF RMS us |")
        print("|---|---:|---:|---:|---:|---:|")
        for name, current in results.items():
            old = before[name]
            print(f"| {name} | {old['compute_s']:.6g} | {current['compute_s']:.6g} | "
                  f"{old.get('position_rms_mm', 0):.6g} | {current.get('position_rms_mm', 0):.6g} | "
                  f"{current.get('tof_rms_us', 0):.6g} |")


if __name__ == "__main__":
    main()
