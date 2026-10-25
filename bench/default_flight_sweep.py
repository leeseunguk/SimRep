#!/usr/bin/env python3
"""Run the einzel 10k step-control candidates serially (five compute medians).

Builds a standalone API executable against the CLI's release dependencies before
measurement, uses 16 Rayon workers, and preserves its entire output. No SIMION
process is started and process wall time is not reported.
"""
import argparse
import json
import os
from pathlib import Path
import re
import statistics
import subprocess
import sys


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", type=Path)
    parser.add_argument("--oracle", type=Path,
                        default=Path("/mnt/c/Users/lsu10/simrep-oracle"))
    parser.add_argument("--quick", action="store_true",
                        help="Sweep only automatic tolerance (tolerance=0 in output)")
    parser.add_argument("--cli-field", action="store_true",
                        help="Use the CLI's combined electric/magnetic field wrapper")
    parser.add_argument("--defaults-only", action="store_true",
                        help="Measure only FlightOptions defaults, with no candidate overrides")
    parser.add_argument("--paired-cli", type=Path,
                        help="Alternate default API and CLI flights to control host-load drift")
    args = parser.parse_args()
    if args.paired_cli and not args.defaults_only:
        parser.error("--paired-cli requires --defaults-only")
    env = {**os.environ, "SIMREP_DEFAULT_SWEEP": str(args.oracle.resolve()),
           "RAYON_NUM_THREADS": "16"}
    if args.quick:
        env["SIMREP_SWEEP_QUICK"] = "1"
    if args.cli_field:
        env["SIMREP_SWEEP_CLI_FIELD"] = "1"
    args.output.parent.mkdir(parents=True, exist_ok=True)
    build = subprocess.run(
        ["cargo", "build", "--release", "-p", "simrep-cli", "--bin", "simrep",
         "--message-format=json"], capture_output=True, text=True, env=env,
        stdin=subprocess.DEVNULL, timeout=1800, check=True)
    libraries = {}
    for line in build.stdout.splitlines():
        artifact = json.loads(line)
        if artifact["reason"] == "compiler-artifact":
            for filename in artifact["filenames"]:
                if filename.endswith(".rlib"):
                    libraries[artifact["target"]["name"]] = Path(filename)
    executable = args.output.with_suffix(".api").resolve()
    source = Path(__file__).with_name("default_flight_api.rs").resolve()
    command = ["rustc", "--edition=2024", "-O", "-L",
               f"dependency={libraries['simrep_fly'].parent}", str(source),
               "-o", str(executable)]
    for name in ("simrep_fly", "simrep_io", "rayon"):
        command += ["--extern", f"{name}={libraries[name]}"]
    compile_result = subprocess.run(
        command, capture_output=True, text=True, env=env,
        stdin=subprocess.DEVNULL, timeout=1800, check=True)
    tolerances = ["0"] if args.quick or args.defaults_only else ["0", "1e-12", "1e-10", "1e-8", "1e-6", "1e-4"]
    cells = ["default"] if args.defaults_only else ["1", "2", "4", "8", str(sys.float_info.max)]
    capture = [json.dumps({"compile": command, "workers": 16, "flights_per_candidate": 5,
                           "field": "combined" if args.cli_field else "instance"}) + "\n",
               build.stderr, compile_result.stderr]
    for tolerance in tolerances:
        for cell in cells:
            times = []
            cli_times = []
            for repeat in range(5):
                run_env = {**env, "SIMREP_SWEEP_TOLERANCE": tolerance,
                           "SIMREP_SWEEP_CELL": cell, "SIMREP_SWEEP_REPEATS": "1"}
                order = (["cli", "api"] if repeat % 2 == 0 else ["api", "cli"]) if args.paired_cli else ["api"]
                for kind in order:
                    command = [str(executable)] if kind == "api" else [
                        str(args.paired_cli.resolve()), "fly", "--device", "cpu",
                        "--pa", str(args.oracle / "_benchfly/einzel.pa0"),
                        "--particles", str(args.oracle / "_benchfly/einzel_10000.ion"),
                        "--voltage", "1=0,2=110,3=0", "--bounds=-101,-19,-19,291,19,19",
                        "--output", str(args.output.with_suffix(".csv"))]
                    result = subprocess.run(
                        command, capture_output=True, text=True, env=run_env,
                        stdin=subprocess.DEVNULL, timeout=1800, check=True)
                    capture += [f"RUN,{kind},tolerance={tolerance},cells={cell},repeat={repeat + 1}\n",
                                result.stdout, result.stderr]
                    if kind == "api":
                        measurement = next(line for line in result.stdout.splitlines()
                                           if line.startswith("SWEEP,"))
                        times.append(float(re.search(r"median_s=([\d.]+)", measurement)[1]))
                    else:
                        cli_times.append(float(re.search(r"fly_s:\s*([\d.]+)", result.stderr)[1]))
            summary = re.sub(r"median_s=[\d.]+",
                             f"median_s={statistics.median(times):.9f}", measurement)
            capture.append(summary + "\n")
            args.output.write_text("".join(capture))
            print(summary, flush=True)
            if cli_times:
                summary = f"PAIRED_CLI,median_s={statistics.median(cli_times):.9f}"
                capture.append(summary + "\n")
                args.output.write_text("".join(capture))
                print(summary, flush=True)


if __name__ == "__main__":
    main()
