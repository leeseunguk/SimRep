#!/usr/bin/env python3
"""Three fresh SIMION compute controls, restoring the captured native STARTs."""
import argparse
import json
import math
import re
import shutil
import statistics
import subprocess
from pathlib import Path

SIMION = "/mnt/c/Program Files/SIMION-2024/simion.exe"
CASES = ("magnet1", "mag90", "sector_2dp", "sector_2dc", "sector_3dp",
         "helmholtz", "solenoid")
REPEATS = dict(magnet1=100, mag90=100, sector_2dp=10, sector_2dc=10,
               sector_3dp=10, helmholtz=10, solenoid=1)


def win(path):
    return subprocess.check_output(["wslpath", "-w", str(path)], text=True).strip()


def records(text, tag):
    return [line.split(",")[1:] for line in text.splitlines()
            if line.startswith(tag + ",")]


def run(root, output, selected):
    output.mkdir(parents=True, exist_ok=True)
    shutil.copytree(root / "contour", output / "contour", dirs_exist_ok=True)
    summary = output / "summary.json"
    report = json.loads(summary.read_text()) if selected and summary.exists() else {}
    for name in selected or CASES:
        source = root / name
        destination = output / name
        destination.mkdir(parents=True, exist_ok=True)
        for path in source.iterdir():
            if path.is_file() and (path.suffix.lower() in (".iob", ".fly", ".fly2", ".rec", ".txt")
                                  or ".pa" in path.name):
                shutil.copyfile(path, destination / path.name)
        captured = records((source / "flight.log").read_text(), "START")
        captured = [[str(index + 1)] + row[1:]
                    for index, row in enumerate(captured * REPEATS[name])]
        ions = []
        for row in captured:
            vx, vy, vz = map(float, row[7:10])
            speed = math.hypot(vx, vy, vz)
            azimuth = math.degrees(math.atan2(-vz, vx))
            elevation = math.degrees(math.asin(vy / speed)) if speed else 0.0
            ions.append(",".join(row[1:7] + [str(azimuth), str(elevation), row[10], "1", "1"]))
        (destination / "captured.ion").write_text("\n".join(ions) + "\n")
        original = source / (name + ".lua")
        # The retained wrapper imports the installed example and records STARTs.
        # Import it, then replace just initialize/terminate to silence records.
        program = ("simion.workbench_program()\nsim_segment_global=1\n"
                   f"simion.import [[{win(original)}]]\nadjustable cpu_record = 0\n"
                   "local starts = {\n")
        program += "\n".join("[" + row[0] + "]={" + ",".join(row[1:10]) + "},"
                             for row in captured)
        program += """
}
function segment.initialize()
  local p = assert(starts[ion_number])
  ion_time_of_birth, ion_mass, ion_charge = p[1], p[2], p[3]
  ion_px_mm, ion_py_mm, ion_pz_mm = p[4], p[5], p[6]
  ion_vx_mm, ion_vy_mm, ion_vz_mm = p[7], p[8], p[9]
  if cpu_record ~= 0 then
    print(string.format('START,%d,%.17g,%.17g,%.17g,%.17g,%.17g,%.17g,%.17g,%.17g,%.17g',
      ion_number,ion_time_of_birth,ion_mass,ion_charge,ion_px_mm,ion_py_mm,ion_pz_mm,
      ion_vx_mm,ion_vy_mm,ion_vz_mm))
  end
end
function segment.terminate()
  if cpu_record ~= 0 then
    print(string.format('FINAL,%d,%.17g,%.17g,%.17g,%.17g',
      ion_number,ion_time_of_flight,ion_px_mm,ion_py_mm,ion_pz_mm))
  end
end
"""
        if name == "helmholtz":
            # Keep the same exactly clipped 0.03 us acceptance interval, but
            # remove the retained wrapper's terminal print inside other_actions.
            program += """
function segment.other_actions()
  if ion_time_of_flight >= 0.03-1e-12 then ion_splat=1 end
end
"""
        (destination / (name + ".lua")).write_text(program)
        samples = []
        for index in range(4):
            audit = index == 0
            command = [SIMION, "--nogui", "--noprompt", "--num-threads", "1",
                       "fly", "--programs", "1", "--omit-relativistic-effects", "1",
                       "--retain-trajectories", "0", "--adjustable",
                       f"cpu_record={int(audit)}", "--particles", "captured.ion", name + ".iob"]
            result = subprocess.run(command, cwd=destination, stdin=subprocess.DEVNULL,
                                    capture_output=True, text=True, timeout=45)
            text = result.stdout + result.stderr
            (destination / f"run-{index}.log").write_text(text)
            result.check_returncode()
            matches = re.findall(r"Fly completed\.\s*(\d+) splats,\s*([0-9.]+) seconds", text)
            if len(matches) != 1 or int(matches[0][0]) != len(captured):
                raise RuntimeError(f"{name}: missing or mismatched compute record: {matches}")
            if audit:
                observed = records(text, "START")
                expected = {row[0]: list(map(float, row[1:10])) for row in captured}
                assert len(observed) == len(expected)
                start_error = max(abs(a - b) for row in observed
                                  for a, b in zip(map(float, row[1:10]), expected[row[0]]))
                assert start_error < 1e-12, (name, start_error)
            else:
                samples.append(float(matches[0][1]))
        report[name] = dict(ions=len(captured), compute_s=samples,
                            median_s=statistics.median(samples), start_max_error=start_error)
        print(name, json.dumps(report[name]), flush=True)
        (output / "summary.json").write_text(json.dumps(report, indent=2) + "\n")


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", type=Path, default=Path(
        "/mnt/c/Users/lsu10/simrep-oracle/physics-validation-20261023"))
    parser.add_argument("--output", type=Path, default=Path(
        "/mnt/c/Users/lsu10/simrep-oracle/physics-validation-20261023/cpu-magnetic-compute"))
    parser.add_argument("--case", nargs="+", choices=CASES)
    args = parser.parse_args()
    run(args.root, args.output, args.case)
