#!/usr/bin/env python3
"""Compare preserved native/oracle records against our converged references."""
import argparse
import json
import math
import os
import re


def load(path):
    with open(path, encoding="utf-8") as f:
        return json.load(f)


def rows(run):
    return [list(map(float, line.split(",")[:9])) for line in run["stdout"].splitlines()
            if re.match(r"^\d+,", line)]


def traces(run, prefix="TRACE_NATIVE,"):
    return {(int(p[1]), round(float(p[2]), 12)): list(map(float, p[3:6]))
            for line in run["stdout"].splitlines() if line.startswith(prefix)
            for p in [line.split(",")]}


def error(actual, reference):
    ds = [sum((a[i] - b[i]) ** 2 for i in range(1, 4)) for a, b in zip(actual, reference)]
    ts = [(a[0] - b[0]) ** 2 for a, b in zip(actual, reference)]
    return {"position_rms_mm": math.sqrt(sum(ds) / len(ds)), "position_max_mm": math.sqrt(max(ds)),
            "tof_rms_us": math.sqrt(sum(ts) / len(ts)), "tof_max_us": math.sqrt(max(ts))}


def native_state(run):
    return [[r[1], r[2], r[3], r[4]] for r in rows(run)]

def kinetic_energy_errors(run, case):
    acceleration = 1.602176634e-19 / 1.66053906892e-27 * 1e-6
    errors = []
    for r, initial, final in zip(rows(run), case["particles"], case["finals"]):
        v2 = sum(v*v for v in r[5:8])
        inverse_gamma = math.sqrt(1.0 - v2 / 299792.458**2)
        energy = initial[1] * v2 / (acceleration * inverse_gamma * (1.0 + inverse_gamma))
        errors.append(energy - final[4])
    return {"rms_ev": math.sqrt(sum(e*e for e in errors)/len(errors)), "max_ev": max(map(abs, errors))}


def trajectory_error(actual, reference):
    differences = [sum((p[i] - reference[key][i]) ** 2 for i in range(3)) for key, p in actual.items()]
    return {"samples": len(differences), "rms_mm": math.sqrt(sum(differences) / len(differences)),
            "max_mm": math.sqrt(max(differences))}


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--root", default="/mnt/c/Users/lsu10/simrep-oracle/_rf_tof")
    args = ap.parse_args()
    cpu = load(os.path.join(args.root, "cpu-measurements.json"))
    gpu = load(os.path.join(args.root, "gpu-measurements.json"))
    oracle = load(os.path.join(args.root, "reference.json"))
    report = {}
    tof_reference = next(r for r in cpu if r["case"] == "tof" and r["factor"] == 8
                         and r["step_us"] == 0.000625 and not r["validation"])
    for name in ["tof", "rf_mirror", "rf_square_wave"]:
        c = next(r for r in cpu if r["case"] == name and not r["validation"])
        g = next(r for r in gpu if r["case"] == name and not r["validation"])
        case = load(os.path.join(args.root, name, "case.json"))
        validation = next(r for r in cpu if r["case"] == name and r["validation"])
        reference = native_state(tof_reference) if name == "tof" else [
            list(map(float, line.split(",")[2:6])) for line in validation["stdout"].splitlines()
            if line.startswith("REFERENCE,")]
        data = {
            "cpu_vs_reference": error(native_state(c), reference),
            "gpu_vs_reference": error(native_state(g), reference),
            "simion_209_vs_reference": error(case["finals"], reference),
            "simion_3_vs_reference": error(case["finals_quality3"], reference),
            "cpu_process_wall_s": c["process_wall_s"],
            "gpu_process_wall_s": g["process_wall_s"],
            "cpu_ke_vs_simion_209": kinetic_energy_errors(c, case),
            "gpu_ke_vs_simion_209": kinetic_energy_errors(g, case),
        }
        for device, r in [("cpu", c), ("gpu", g)]:
            data[device + "_compute_s"] = float(re.search(r"compute_s=([\d.]+)", r["stderr"])[1])
            v = next(r for r in (cpu if device == "cpu" else gpu) if r["case"] == name and r["validation"])
            if name == "tof":
                ref = traces(next(r for r in cpu if r["case"] == name and r["factor"] == 8 and r["validation"]))
            else:
                ref = traces(validation, "TRACE_REFERENCE,")
            data[device + "_trajectory_vs_reference"] = trajectory_error(traces(v), ref)
        oracle_traces = {(int(r[0]), round(r[1], 12)): r[2:5] for r in case["traces"]}
        data["simion_209_trajectory_vs_reference"] = trajectory_error(oracle_traces, ref)
        for r in oracle[name]["runs"]:
            if "trajectory_quality" in r:
                data["simion_%d_process_wall_s" % r["trajectory_quality"]] = r["seconds"]
                data["simion_%d_compute_s" % r["trajectory_quality"]] = float(re.search(r"(\d+(?:\.\d+)?) seconds", r["stdout"])[1])
        if name == "tof":
            data["grid_study"] = [{"factor": r["factor"], "step_us": r["step_us"],
                                   **error(native_state(r), reference)} for r in cpu
                                  if r["case"] == name and not r["validation"]]
        report[name] = data
    print(json.dumps(report, indent=2))
    with open(os.path.join(args.root, "comparison.json"), "w", encoding="utf-8") as f:
        json.dump(report, f, indent=2)


if __name__ == "__main__":
    main()
