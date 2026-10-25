#!/usr/bin/env python3
"""Deterministic, bounded SIMION time refinement of captured beam launches.

Runs serial foreground SIMION subprocesses from C-drive working copies, with
closed stdin and a per-run deadline. Does not modify original example/oracle
files. Uses the recorded SIMION KE (not native conversion constants).
"""
import argparse
import datetime
import json
import math
from pathlib import Path
import re
import shutil
import subprocess

import numpy as np

from beam_compare import records
from beam_reference import difference, exit_classes
from run_oracle import kill_simion_started_after


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--case", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--steps", type=float, nargs="+",
                        default=[1e-4, 5e-5, 2.5e-5, 1.25e-5],
                        help="maximum dt in us; 0 removes the step callback for baseline timing")
    parser.add_argument("--timeout", type=float, default=90)
    parser.add_argument("--reference", type=Path,
                        help="independent beam_reference.py endpoint CSV")
    args = parser.parse_args()
    reference = np.loadtxt(args.reference, delimiter=",") if args.reference else None
    assert str(args.output.resolve()).startswith("/mnt/c/")
    args.output.mkdir(parents=True, exist_ok=True)
    for name in ("repulsion1.iob", "blank.pa"):
        shutil.copyfile(args.case / name, args.output / name)
    readout = Path(__file__).with_name("beam_readout.lua").read_text()
    initial = records((args.case / "oracle.txt").read_text(), "BEAM_START")
    with (args.output / "replay.ion").open("w") as out:
        for i in sorted(initial):
            tob, mass, charge, x, y, z, vx, vy, vz, ke, cwf = initial[i]
            az = math.degrees(math.atan2(-vz, vx))
            el = math.degrees(math.atan2(vy, math.hypot(vx, vz)))
            out.write(",".join(format(v, ".17g") for v in
                               [tob, mass, charge, x, y, z, az, el, ke, cwf]) + "\n")
    report, previous = [], None
    for dt in args.steps:
        path = args.output / f"oracle-{dt:.9g}.txt"
        if not path.exists():
            program = readout
            if dt == 0:
                program, removed = re.subn(
                    r"\nfunction segment\.tstep_adjust\(\)\n.*?\nend\n",
                    "\n", program, count=1, flags=re.DOTALL)
                assert removed == 1, "baseline timing must omit the per-step callback"
            (args.output / "repulsion1.lua").write_text(program)
            command = [
                "/mnt/c/Program Files/SIMION-2024/simion.exe", "--nogui", "--noprompt",
                "fly", "--particles", "replay.ion", "--programs", "1", "--grouped", "1",
                "--omit-relativistic-effects", "1", "--repulsion", "beam",
                "--repulsion-amount", "1e-5", "--trajectory-quality", "-9"]
            if dt > 0:
                command += ["--adjustable", f"beam_max_dt={dt:.17g}"]
            command.append("repulsion1.iob")
            started = datetime.datetime.now(datetime.timezone.utc)
            try:
                proc = subprocess.run(
                    command, cwd=args.output, stdin=subprocess.DEVNULL,
                    capture_output=True, text=True, timeout=args.timeout)
            except subprocess.TimeoutExpired:
                kill_simion_started_after(started)
                raise
            path.write_text(proc.stdout + proc.stderr)
            if proc.returncode:
                raise RuntimeError(f"SIMION exit {proc.returncode}: {proc.stdout[-3000:]} {proc.stderr}")
        text = path.read_text()
        starts, ends = records(text, "BEAM_START"), records(text, "BEAM_FINAL")
        assert starts.keys() == initial.keys() == ends.keys()
        measured = np.array([starts[i] for i in sorted(starts)])
        captured = np.array([initial[i] for i in sorted(initial)])
        assert np.allclose(measured, captured, rtol=1e-12, atol=1e-12), np.max(np.abs(measured - captured))
        final = np.array([ends[i][:7] for i in sorted(ends)])
        final[:, 0] -= measured[:, 0]
        np.savetxt(path.with_suffix(".csv"), final, delimiter=",", fmt="%.17g")
        entry = dict(max_step_us=dt, compute_s=float(re.search(
            r"Fly completed\.\s+\d+ splats,\s+([\d.]+) seconds", text)[1]),
            leader_x_mm=float(final[0, 1]),
            exit_classes={c: exit_classes(final).count(c) for c in sorted(set(exit_classes(final)))})
        if previous is not None:
            entry["self_change"] = difference(final, previous)
            entry["class_changes"] = sum(a != b for a, b in zip(exit_classes(final), exit_classes(previous)))
        if reference is not None:
            entry["own_reference_error"] = difference(final, reference)
            entry["own_reference_class_changes"] = [
                i + 1 for i, (a, b) in enumerate(zip(exit_classes(final), exit_classes(reference)))
                if a != b]
        report.append(entry)
        print(json.dumps(entry), flush=True)
        previous = final
    (args.output / "report.json").write_text(json.dumps(report, indent=2) + "\n")


if __name__ == "__main__":
    main()
