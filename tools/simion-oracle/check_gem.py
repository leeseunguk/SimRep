#!/usr/bin/env python3
"""Compare every installed example GEM with native simrep-gem and SIMION gem2pa.

SIMION runs synchronously with stdin closed, a timeout and a Windows-drive cwd.
Only working copies are touched. Run from WSL, after building the convert example.
"""
import argparse
from datetime import date
import array
import datetime
import json
import re
import os
import selectors
import shutil
import struct
import subprocess
import time
from pathlib import Path

from run_oracle import (DEFAULT_EXAMPLES, DEFAULT_SIMION,
                        kill_simion_started_after, win_path)


# Shared SIMION oracle working directory; required via SIMREP_ORACLE_CACHE or --root.
def oracle_cache():
    root = os.environ.get("SIMREP_ORACLE_CACHE")
    if root:
        return Path(root)
    raise SystemExit(
        "SIMREP_ORACLE_CACHE is not set and --root was not given. "
        "Set SIMREP_ORACLE_CACHE to the directory that holds (or should hold) "
        "the oracle working data (default layout: $SIMREP_ORACLE_CACHE/_gem "
        "for this comparison), or pass --root pointing at that directory.")


def invoke(argv, cwd, timeout):
    start = datetime.datetime.now(datetime.timezone.utc)
    try:
        begin = time.perf_counter()
        with subprocess.Popen(argv, cwd=cwd, stdin=subprocess.DEVNULL,
                              stdout=subprocess.PIPE, stderr=subprocess.STDOUT) as p:
            stream = bytearray()
            line = bytearray()
            marks = []
            with selectors.DefaultSelector() as events:
                events.register(p.stdout, selectors.EVENT_READ)
                while events.get_map():
                    remaining = timeout - (time.perf_counter() - begin)
                    if remaining <= 0 or not events.select(remaining):
                        p.kill()
                        p.wait()
                        raise subprocess.TimeoutExpired(argv, timeout)
                    chunk = os.read(p.stdout.fileno(), 1)
                    if not chunk:
                        events.unregister(p.stdout)
                        break
                    stream.extend(chunk)
                    line.extend(chunk)
                    if chunk == b"#" and line.startswith(b"status2,"):
                        marks.append(time.perf_counter())
                    if chunk == b"\n":
                        line.clear()
            exit_code = p.wait(timeout=max(0.01, timeout - (time.perf_counter() - begin)))
        result = {"exit": exit_code, "stdout": stream.decode(errors="replace"),
                  "wall_ms": (time.perf_counter() - begin) * 1000}
        if len(marks) >= 2:
            result["progress_ms"] = (marks[-1] - marks[0]) * 1000
            result["progress_marks"] = len(marks)
        return result
    except subprocess.TimeoutExpired:
        if str(argv[0]).endswith("simion.exe"):
            kill_simion_started_after(start)
        return {"exit": None, "stdout": "timeout"}


def decode(path):
    with path.open("rb") as f:
        mode, symmetry, maximum, nx, ny, nz, mirror = struct.unpack("<iidiiii", f.read(32))
        spacing = struct.unpack("<ddd", f.read(24)) if mode == -2 else (1., 1., 1.)
        values = array.array("d")
        values.fromfile(f, nx * ny * nz)
        trailer = f.read()
    return (nx, ny, nz, symmetry, mirror, spacing), maximum, values, trailer


def compare(ours, oracle):
    ha, ma, a, ta = decode(ours)
    hb, mb, b, tb = decode(oracle)
    if ha != hb:
        return {"status": "header mismatch", "ours_header": ha, "oracle_header": hb}
    if a == b and ma == mb and ta == tb:
        return {"status": "pass", "mismatches": 0, "points": len(a),
                "byte_equal": ours.read_bytes() == oracle.read_bytes()}
    count = 0
    samples = []
    nx, ny = ha[:2]
    for i, (va, vb) in enumerate(zip(a, b)):
        ea, eb = va > ma, vb > mb
        va = va - 2 * ma if ea else va
        vb = vb - 2 * mb if eb else vb
        if va != vb or ea != eb:
            count += 1
            if len(samples) < 12:
                samples.append({"xyz": [i % nx, i // nx % ny, i // (nx * ny)],
                                "ours": [ea, va], "simion": [eb, vb],
                                "reason": "boundary membership" if ea != eb else "fill precedence or potential"})
    return {"status": "pass" if count == 0 and ta == tb else "mismatch",
            "mismatches": count, "points": len(a), "samples": samples,
            "trailer_equal": ta == tb,
            "byte_equal": ours.read_bytes() == oracle.read_bytes()}


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--examples", type=Path, default=DEFAULT_EXAMPLES)
    ap.add_argument("--root", type=Path, default=None,
                    help="Oracle working directory (default: $SIMREP_ORACLE_CACHE/_gem)")
    ap.add_argument("--native", type=Path, default=Path("target/release/examples/convert"))
    ap.add_argument("--simion", default=DEFAULT_SIMION)
    ap.add_argument("--timeout", type=float, default=120)
    ap.add_argument("--filter", default="")
    ap.add_argument("--skip-oracle", action="store_true", help="reuse existing per-file SIMION results")
    ap.add_argument("--report", type=Path, default=Path("docs/gem-validation.md"))
    args = ap.parse_args()
    if args.root is None:
        args.root = oracle_cache() / "_gem"
    args.native = args.native.resolve()
    args.report = args.report.resolve()
    args.root.mkdir(parents=True, exist_ok=True)
    # Copy dependencies too, without installed PA/IOB datasets or read-only metadata.
    for source in args.examples.rglob("*"):
        if source.is_file() and source.suffix.lower() in (
                ".gem", ".lua", ".stl", ".txt", ".csv", ".dat"):
            target = args.root / "examples" / source.relative_to(args.examples)
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(source, target)
    results = []
    cache = args.root / "results.json"
    old = json.loads(cache.read_text()) if cache.exists() else []
    previous = {r["file"]: r for r in old}
    for source in sorted(args.examples.rglob("*.gem")):
        relative = source.relative_to(args.examples)
        if args.filter and not re.search(args.filter, str(relative)):
            if str(relative) in previous:
                results.append(previous[str(relative)])
            continue
        gem = args.root / "examples" / relative
        oracle = gem.with_suffix(".oracle.pa#")
        ours = gem.with_suffix(".native.pa#")
        record = {"file": str(relative)}
        if args.skip_oracle and str(relative) in previous:
            record["simion"] = previous[str(relative)]["simion"]
        else:
            record["simion"] = invoke(
                [args.simion, "--nogui", "--noprompt", "gem2pa",
                 win_path(str(gem)), win_path(str(oracle))], gem.parent, args.timeout)
        native = invoke([str(args.native), str(gem), str(ours)], gem.parent, args.timeout)
        record["native"] = native
        if record["simion"]["exit"] == 0 and oracle.exists():
            with oracle.open("rb") as f:
                header = struct.unpack("<iidiiii", f.read(32))
                record["oracle_points"] = header[3] * header[4] * header[5]
        if native["exit"] == 0 and native["stdout"].startswith("include fragment:"):
            record.update(status="include fragment", reason=native["stdout"].strip())
        elif native["exit"] != 0:
            record.update(status="unsupported", reason=native["stdout"].strip())
        elif record["simion"]["exit"] != 0 or not oracle.exists():
            record.update(status="oracle error", reason=record["simion"]["stdout"].strip())
        else:
            try:
                record.update(compare(ours, oracle))
            except (OSError, ValueError, EOFError, struct.error) as error:
                record.update(status="oracle format unsupported", reason=str(error))
            match = re.search(r"compute_ms: ([0-9.]+)", native["stdout"])
            if match:
                record["native_compute_ms"] = float(match[1])
        results.append(record)
        print(record["file"], record["status"], record.get("mismatches", ""),
              flush=True)
        cache.write_text(json.dumps(results + [r for r in old if r["file"] not in
                                             {s["file"] for s in results}], indent=2))
    results.sort(key=lambda r: r["file"])
    cache.write_text(json.dumps(results, indent=2))
    counts = {status: sum(r["status"] == status for r in results)
              for status in sorted({r["status"] for r in results})}
    largest = sorted((r for r in results if r.get("oracle_points")),
                     key=lambda r: r["oracle_points"], reverse=True)[:5]
    lines = ["# Installed GEM oracle comparison", "",
             f"Generated by `tools/simion-oracle/check_gem.py` on {date.today()}.",
             "All 261 installed `.gem` files are run headlessly from working copies.",
             "Full command output, headers and mismatch coordinates: "
             f"`{win_path(str(args.root.resolve()))}\\results.json`.", "",
             f"Summary: {counts}.", "",
             "## Largest arrays", "",
             "Native time is the complete point classification loop, excluding",
             "parsing, allocation and output. SIMION exposes only process wall",
             "time through this CLI; its progress output is buffered, so timing",
             "progress marks is not a valid compute measurement. A preparation-free",
             "SIMION compute comparison is unavailable from this interface.",
             "The wall times below are reference data, not speedup ratios.", "",
             "| GEM | Points | Native compute ms | SIMION process wall ms |",
             "| --- | ---: | ---: | ---: |"]
    for r in largest:
        lines.append(f"| `{r['file']}` | {r['oracle_points']} | "
                     f"{r.get('native_compute_ms', 'unsupported')} | "
                     f"{r['simion'].get('wall_ms', 'not measured')} |")
    lines.extend(["",
             "## All installed GEM files", "",
             "| GEM | Result | Mismatching grid points / unsupported feature |",
             "| --- | --- | --- |"])
    for r in results:
        reason = str(r.get("mismatches", r.get("reason", ""))).replace("\n", " ").replace("|", "/")
        lines.append(f"| `{r['file']}` | {r['status']} | {reason} |")
    lines.extend(["", "## Mismatch samples", ""])
    for r in results:
        if r.get("samples"):
            lines.append(f"- `{r['file']}`: `{json.dumps(r['samples'])}`")
    args.report.write_text("\n".join(lines) + "\n")
    print("Summary:", counts, flush=True)


if __name__ == "__main__":
    main()
