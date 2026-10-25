#!/usr/bin/env python3
"""Measure SIMION's 3D PA collision surface with all voltages set to zero."""
import json
import shutil
from pathlib import Path

from run_oracle import DEFAULT_SIMION, run_simion, win_path


def main():
    source = Path("/mnt/c/Users/lsu10/simrep-oracle/_quad_gpu")
    work = source.parent / "_quad_collision_probe"
    work.mkdir(exist_ok=True)
    for path in source.glob("quad.pa*"):
        shutil.copyfile(path, work / path.name)
    shutil.copyfile(source / "compute.iob", work / "input.iob")
    (work / "setup.lua").write_text(
        "simion.wb:load('input.iob')\n"
        "local pa=simion.wb.instances[1].pa\n"
        "pa:fast_adjust{[1]=0,[2]=0,[3]=0}\n"
        "pa:save('quad.pa0')\n"
        "simion.wb:save('probe.iob')\n"
    )
    (work / "probe.lua").write_text(
        "simion.workbench_program()\nsim_segment_global=1\n"
        "function segment.terminate()\n"
        "print(string.format('FINAL,%d,%.17g,%.17g,%.17g,%.17g',"
        "ion_number,ion_time_of_flight,ion_px_mm,ion_py_mm,ion_pz_mm))\n"
        "end\n"
    )
    labels = []
    ions = []
    for y in (0.0, 0.001, 0.01, 0.1, 0.24, 0.25, 0.49, 0.5, 1.0, 2.0):
        labels.append(f"rod y={y}")
        ions.append(f"0,100,1,30,{y:.17g},0,90,0,100,1,1")
    for z in (3.99, 4.0, 4.001, 4.24, 4.25, 4.26, 4.49, 4.5, 4.51):
        labels.append(f"plate z={z}")
        ions.append(f"0,100,1,1,0,{z:.17g},0,0,100,1,1")
    (work / "particles.ion").write_text("\n".join(ions) + "\n")
    setup = run_simion(DEFAULT_SIMION, ["--nogui", "--noprompt", "lua", "setup.lua"], str(work), 40)
    (work / "setup.json").write_text(json.dumps(setup, indent=2))
    if setup["exit_code"] != 0:
        raise RuntimeError(setup["stdout"] + setup["stderr"])
    result = run_simion(
        DEFAULT_SIMION,
        ["--nogui", "--noprompt", "--default-num-particles", "100", "fly",
         "--trajectory-quality", "209", "--recording-enable", "0",
         "--retain-trajectories", "0", "--particles", win_path(str(work / "particles.ion")),
         "probe.iob"],
        str(work),
        60,
    )
    (work / "result.json").write_text(json.dumps(result, indent=2))
    if result["exit_code"] != 0:
        raise RuntimeError(result["stdout"] + result["stderr"])
    count = 0
    for line in result["stdout"].splitlines():
        if line.startswith("FINAL,"):
            row = line.split(",")
            ion = int(row[1])
            print(labels[ion - 1], "->", ",".join(row[2:]))
            count += 1
    if count != len(ions):
        raise RuntimeError(f"expected {len(ions)} terminal states, got {count}")


if __name__ == "__main__":
    main()
