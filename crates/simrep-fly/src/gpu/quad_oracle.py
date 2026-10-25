#!/usr/bin/env python3
"""Reproduce the app-template quadrupole SIMION compute/readout calibration."""
import argparse
import json
import pathlib
import re
import shutil
import sys

ROOT = pathlib.Path(__file__).resolve().parents[4]
sys.path.insert(0, str(ROOT / "tools/simion-oracle"))
from run_oracle import DEFAULT_SIMION, run_simion, win_path


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source", default="/mnt/c/Users/lsu10/Documents/SimRep/quadrupole-2")
    parser.add_argument("--beam", default="/mnt/c/Users/lsu10/simrep-gpu-bench/quadrupole-1000.ion")
    parser.add_argument("--work", default="/mnt/c/Users/lsu10/simrep-oracle/_quad_gpu")
    args = parser.parse_args()
    work = pathlib.Path(args.work)
    work.mkdir(parents=True, exist_ok=True)
    for source in pathlib.Path(args.source).glob("quadrupole.pa*"):
        shutil.copyfile(source, work / source.name)
        shutil.copyfile(source, work / source.name.replace("quadrupole", "quad"))
        shutil.copyfile(source, work / source.name.replace("quadrupole", "einzel"))
    shutil.copyfile(
        pathlib.Path(DEFAULT_SIMION).parent / "examples/einzel/einzel.iob",
        work / "quad.iob",
    )
    shutil.copyfile(pathlib.Path(__file__).with_name("quad_setup.lua"), work / "setup.lua")
    shutil.copyfile(pathlib.Path(__file__).with_name("quad_readout.lua"), work / "readout.lua")
    if pathlib.Path(args.beam).resolve() != (work / "particles.ion").resolve():
        shutil.copyfile(args.beam, work / "particles.ion")
    count = sum(1 for line in (work / "particles.ion").read_text().splitlines()
                if line.strip() and not line.lstrip().startswith(";"))
    setup = run_simion(DEFAULT_SIMION, ["--nogui", "--noprompt", "lua", "setup.lua"], str(work), 40)
    (work / "setup.json").write_text(json.dumps(setup, indent=2))
    if setup["exit_code"]:
        raise RuntimeError(setup["stdout"] + setup["stderr"])
    print(setup["stdout"])
    for quality in (3, 209):
        for recording in (False, True):
            argv = ["--nogui", "--noprompt", "--default-num-particles", str(count + 100),
                    "fly", "--trajectory-quality", str(quality), "--recording-enable", "0",
                    "--retain-trajectories", "0", "--particles", win_path(str(work / "particles.ion")),
                    "readout.iob" if recording else "compute.iob"]
            run = run_simion(DEFAULT_SIMION, argv, str(work), 120)
            match = re.search(r"Fly completed\.\s+(\d+) splats,\s+([\d.]+) seconds", run["stdout"])
            if match:
                run["ions"] = int(match[1])
                run["compute_s"] = float(match[2])
            name = f"quality-{quality}-{'readout' if recording else 'compute'}.json"
            (work / name).write_text(json.dumps(run, indent=2))
            if run["exit_code"] or not match or run["ions"] != count:
                raise RuntimeError(run["stdout"] + run["stderr"])
            print(f"SIMION quality={quality} readout={recording} ions={count} compute_s={run['compute_s']}",
                  flush=True)


if __name__ == "__main__":
    main()
