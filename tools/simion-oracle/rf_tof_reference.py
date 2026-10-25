#!/usr/bin/env python3
"""Copy examples and attach readout-only workbench callbacks, retaining PA PRGs."""
import argparse
import json
import os
import shutil
import sys

sys.path.insert(0, os.path.dirname(__file__))
from run_oracle import copy_example, run_simion, win_path, DEFAULT_SIMION


# Shared SIMION oracle working directory; required via SIMREP_ORACLE_CACHE
# or --output.
def oracle_cache():
    root = os.environ.get("SIMREP_ORACLE_CACHE")
    if root:
        return root
    raise SystemExit(
        "SIMREP_ORACLE_CACHE is not set and --output was not given. "
        "Set SIMREP_ORACLE_CACHE to the directory that holds (or should hold) "
        "the oracle working data (default layout: $SIMREP_ORACLE_CACHE/_rf_tof "
        "for this reference), or pass --output pointing at that directory.")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", default=None,
                        help="output directory (default: $SIMREP_ORACLE_CACHE/_rf_tof)")
    args = parser.parse_args()
    if args.output is None:
        args.output = os.path.join(oracle_cache(), "_rf_tof")
    logs = {}
    for name, example, stem in [
        ("tof", "tof", "tof"),
        ("rf_mirror", "performance_tests/8_rf_tests", "rf_mirror"),
        ("rf_square_wave", "performance_tests/8_rf_tests", "rf_square_wave"),
    ]:
        work = os.path.join(args.output, name)
        copy_example(os.path.join(os.path.dirname(DEFAULT_SIMION), "examples", example), work)
        # Separate IOB basename prevents the readout Lua from replacing the
        # original per-PA legacy fast_adjust program.
        for extension in [".iob", ".fly", ".rec"]:
            shutil.copyfile(os.path.join(work, stem + extension), os.path.join(work, "readout" + extension))
        shutil.copyfile(os.path.join(os.path.dirname(__file__), "rf_tof_readout.lua"), os.path.join(work, "readout.lua"))
        runs = []
        for pa in sorted(n for n in os.listdir(work) if n.endswith(".pa#")):
            runs.append(run_simion(DEFAULT_SIMION, ["--nogui", "--noprompt", "refine", "--convergence", "1e-7", pa], work, 300))
        metadata = run_simion(DEFAULT_SIMION, ["--nogui", "--noprompt", "lua",
            win_path(os.path.join(os.path.dirname(__file__), "workbench_metadata.lua")),
            stem + ".iob", win_path(os.path.join(work, "loaded-"))], work, 30)
        runs.append(metadata)
        runs.append(run_simion(DEFAULT_SIMION, [
            "--nogui", "--noprompt", "fly", "--programs", "1",
            "--trajectory-quality", "209", "--recording-enable", "0", "readout.iob",
        ], work, 120))
        high_precision_readout = runs[-1]
        readout3 = run_simion(DEFAULT_SIMION, [
            "--nogui", "--noprompt", "fly", "--programs", "1",
            "--trajectory-quality", "3", "--recording-enable", "0", "readout.iob",
        ], work, 120)
        runs.append(readout3)
        logs[name] = {"work": win_path(work), "runs": runs}
        config = {"instances": [], "particles": [], "finals": [], "traces": []}
        for line in metadata["stdout"].splitlines():
            fields = line.split(",")
            if fields[0] == "BOUNDS":
                config["bounds"] = list(map(float, fields[1:]))
            elif fields[0] == "INSTANCE":
                n = int(fields[1])
                config["instances"].append({"pa": "loaded-%d.pa" % n, "origin": list(map(float, fields[3:6])),
                                            "scale": float(fields[6]), "axes": []})
            elif fields[0] == "AXIS":
                config["instances"][int(fields[1]) - 1]["axes"].append(list(map(float, fields[3:6])))
        for line in high_precision_readout["stdout"].splitlines():
            fields = line.split(",")
            if fields[0] == "START":
                config["particles"].append(list(map(float, fields[2:])))
            elif fields[0] == "FINAL":
                config["finals"].append(list(map(float, fields[2:])))
            elif fields[0] == "TRACE":
                config["traces"].append(list(map(float, fields[1:])))
        config["finals_quality3"] = [list(map(float, line.split(",")[2:]))
                                     for line in readout3["stdout"].splitlines() if line.startswith("FINAL,")]
        with open(os.path.join(work, "case.json"), "w", encoding="utf-8") as out:
            json.dump(config, out, indent=2)
        for quality in [3, 209]:
            timing = run_simion(DEFAULT_SIMION, [
                "--nogui", "--noprompt", "fly", "--programs", "1",
                "--trajectory-quality", str(quality), "--recording-enable", "0", stem + ".iob",
            ], work, 120)
            timing["trajectory_quality"] = quality
            runs.append(timing)
        print(name, runs[-1]["stdout"], runs[-1]["stderr"])
    os.makedirs(args.output, exist_ok=True)
    with open(os.path.join(args.output, "reference.json"), "w", encoding="utf-8") as out:
        json.dump(logs, out, indent=2)
    return 0 if all(r["exit_code"] == 0 for c in logs.values() for r in c["runs"]) else 1


if __name__ == "__main__":
    raise SystemExit(main())
