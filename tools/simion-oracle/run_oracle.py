#!/usr/bin/env python3
"""Run the installed SIMION in batch (--nogui) mode to produce oracle results.

The example folder is copied out of the read-only SIMION install into
<oracle-root>/<example>/ (a Windows-visible path so SIMION can write there),
then `refine` and/or `fly` are run and everything is logged to oracle.json.
"""
import argparse
import datetime
import json
import os
import shutil
import subprocess
import sys
import time

DEFAULT_SIMION = "/mnt/c/Program Files/SIMION-2024/simion.exe"
DEFAULT_ORACLE_ROOT = "/mnt/c/Users/lsu10/simrep-oracle"


def win_path(path):
    return subprocess.run(["wslpath", "-w", path], capture_output=True, text=True,
                          check=True).stdout.strip()


def copy_example(src, dst):
    """Fresh copy without metadata, so the read-only attribute is not carried over."""
    if os.path.exists(dst):
        shutil.rmtree(dst)
    for root, _dirs, files in os.walk(src):
        out = os.path.join(dst, os.path.relpath(root, src))
        os.makedirs(out, exist_ok=True)
        for name in files:
            shutil.copyfile(os.path.join(root, name), os.path.join(out, name))


def snapshot(directory):
    state = {}
    for root, _dirs, files in os.walk(directory):
        for name in files:
            path = os.path.join(root, name)
            st = os.stat(path)
            state[os.path.relpath(path, directory)] = (st.st_mtime_ns, st.st_size)
    return state


def kill_simion_started_after(start_utc):
    """subprocess timeout only kills the WSL-side wrapper; tree-kill the Windows process too."""
    since = (start_utc - datetime.timedelta(seconds=2)).strftime("%Y-%m-%dT%H:%M:%SZ")
    ps = ("Get-Process simion -ErrorAction SilentlyContinue | Where-Object "
          "{ $_.StartTime.ToUniversalTime() -ge [DateTimeOffset]::Parse('%s').UtcDateTime } | "
          "ForEach-Object { taskkill.exe /T /F /PID $_.Id }" % since)
    subprocess.run(["powershell.exe", "-NoProfile", "-Command", ps], timeout=60)


def run_simion(simion, args, cwd, timeout):
    argv = [simion] + args
    before = snapshot(cwd)
    start_utc = datetime.datetime.now(datetime.timezone.utc)
    t0 = time.perf_counter()
    try:
        proc = subprocess.run(argv, cwd=cwd, capture_output=True, timeout=timeout)
        exit_code, out, err, timed_out = proc.returncode, proc.stdout, proc.stderr, False
    except subprocess.TimeoutExpired as exc:
        kill_simion_started_after(start_utc)
        exit_code, out, err, timed_out = None, exc.stdout or b"", exc.stderr or b"", True
    seconds = time.perf_counter() - t0
    after = snapshot(cwd)
    changed = sorted(p for p, sig in after.items() if before.get(p) != sig)
    return {
        "argv": [win_path(simion)] + args,
        "command_line": subprocess.list2cmdline([win_path(simion)] + args),
        "cwd": win_path(cwd),
        "exit_code": exit_code,
        "timed_out": timed_out,
        "seconds": round(seconds, 3),
        "output_files": [win_path(os.path.join(cwd, p)) for p in changed],
        "stdout": out.decode("utf-8", errors="replace"),
        "stderr": err.decode("utf-8", errors="replace"),
    }


def main():
    ap = argparse.ArgumentParser(description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("example", help="example folder name under SIMION examples/ (e.g. einzel, "
                                    "performance_tests/5_planar_pa_tests)")
    ap.add_argument("--refine", action="store_true",
                    help="refine every *.pa# in the example (or every *.pa if there is no .pa#)")
    ap.add_argument("--fly", metavar="IOB", help="IOB file (relative to the example) to fly")
    ap.add_argument("--rec", metavar="REC",
                    help="data recording definition (.rec) for --fly (default: <iob stem>.rec if present)")
    ap.add_argument("--simion", default=DEFAULT_SIMION, help="path to simion.exe (WSL path)")
    ap.add_argument("--oracle-root", default=DEFAULT_ORACLE_ROOT,
                    help="working root, must be on a Windows drive (WSL path)")
    ap.add_argument("--gem2pa", action="store_true",
                    help="run gem2pa on every *.gem in the example before --refine")
    ap.add_argument("--num-threads", metavar="N",
                    help="max CPU threads for SIMION (global --num-threads option)")
    ap.add_argument("--globals", default="", metavar="ARGS",
                    help="extra global SIMION options before the command (e.g. "
                         "'--default-num-particles 100000')")
    ap.add_argument("--refine-args", default="", metavar="ARGS",
                    help="extra arguments passed to every refine command")
    ap.add_argument("--fly-args", default="", metavar="ARGS",
                    help="extra arguments passed to the fly command")
    ap.add_argument("--work-dir", metavar="DIR",
                    help="use this existing Windows-visible dir instead of copying the example")
    ap.add_argument("--timeout", type=float, default=1800, help="per-command timeout in seconds")
    args = ap.parse_args()
    if not (args.refine or args.fly or args.gem2pa):
        ap.error("nothing to do: pass --refine, --gem2pa and/or --fly IOB")

    src = os.path.join(os.path.dirname(args.simion), "examples", args.example)
    if args.work_dir:
        src = None
        work = args.work_dir
        if not os.path.isdir(work):
            ap.error("work dir not found: %s" % work)
    else:
        if not os.path.isdir(src):
            ap.error("example folder not found: %s" % src)
        work = os.path.join(args.oracle_root, args.example)
        copy_example(src, work)

    base = ["--nogui", "--noprompt"]
    if args.num_threads:
        base += ["--num-threads", args.num_threads]
    if args.globals:
        base += args.globals.split()
    version = subprocess.run([args.simion, "--nogui", "--version"], capture_output=True,
                             timeout=60).stdout.decode("utf-8", errors="replace")
    runs = []

    if args.gem2pa:
        gems = sorted(n for n in os.listdir(work) if n.lower().endswith(".gem"))
        if not gems:
            ap.error("no .gem files in %s" % work)
        for gem in gems:
            r = run_simion(args.simion, base + ["gem2pa", gem], work, args.timeout)
            runs.append(dict({"step": "gem2pa", "target": gem}, **r))

    if args.refine:
        names = sorted(os.listdir(work))
        targets = [n for n in names if n.lower().endswith(".pa#")] or \
                  [n for n in names if n.lower().endswith(".pa")]
        if not targets:
            ap.error("no .pa# or .pa files to refine in %s" % work)
        extra = args.refine_args.split() if args.refine_args else []
        for pa in targets:
            r = run_simion(args.simion, base + ["refine"] + extra + [pa], work, args.timeout)
            runs.append(dict({"step": "refine", "target": pa}, **r))

    if args.fly:
        if not os.path.isfile(os.path.join(work, args.fly)):
            ap.error("IOB not found in example: %s" % args.fly)
        stem = os.path.splitext(args.fly)[0]
        rec = args.rec or (stem + ".rec" if os.path.isfile(os.path.join(work, stem + ".rec")) else None)
        rec_out = stem + ".oracle_recording.txt"
        fly_args = ["fly", "--recording-enable", "1", "--recording-output", rec_out]
        if rec:
            fly_args += ["--recording", rec]
        fly_args += args.fly_args.split() if args.fly_args else []
        r = run_simion(args.simion, base + fly_args + [args.fly], work, args.timeout)
        rec_path = os.path.join(work, rec_out)
        runs.append(dict({"step": "fly", "target": args.fly, "recording": rec,
                          "recording_output": win_path(rec_path) if os.path.isfile(rec_path) else None},
                         **r))

    oracle = {
        "example": args.example,
        "source_dir": win_path(src) if src else None,
        "work_dir": win_path(work),
        "simion_exe": win_path(args.simion),
        "simion_version": version.strip(),
        "created_utc": datetime.datetime.now(datetime.timezone.utc).isoformat(timespec="seconds"),
        "runs": runs,
    }
    out_json = os.path.join(work, "oracle.json")
    with open(out_json, "w", encoding="utf-8") as f:
        json.dump(oracle, f, indent=2)

    for r in runs:
        print("%-6s %-24s exit=%s seconds=%.3f%s" % (r["step"], r["target"], r["exit_code"],
                                                    r["seconds"], " TIMEOUT" if r["timed_out"] else ""))
    print("wrote", out_json)
    return 0 if all(r["exit_code"] == 0 for r in runs) else 1


if __name__ == "__main__":
    sys.exit(main())
