#!/usr/bin/env python3
"""Measure the five largest converted GEMs; optional synchronous SIMION runs."""
import argparse
import json
import re
import statistics
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(REPO / "tools/simion-oracle"))
from check_gem import invoke
from run_oracle import win_path

ap = argparse.ArgumentParser(description=__doc__)
ap.add_argument("--oracle", action="store_true")
ap.add_argument("--runs", type=int, default=3)
args = ap.parse_args()
root = Path("/mnt/c/Users/lsu10/simrep-oracle/_gem")
cache = root / "results.json"
results = json.loads(cache.read_text())
largest = sorted((r for r in results if r["status"] == "pass"),
                 key=lambda r: r.get("oracle_points", 0), reverse=True)[:5]
native = REPO / "target/release/examples/convert"
for r in largest:
    gem = root / "examples" / r["file"]
    samples = []
    for _ in range(args.runs):
        result = invoke([str(native), str(gem), str(gem.with_suffix(".bench.pa#")),
                         "--bench"], gem.parent, 30)
        assert result["exit"] == 0, result
        samples.append(float(re.search(r"compute_ms: ([0-9.]+)", result["stdout"])[1]))
    r["native_compute_ms"] = statistics.median(samples)
    r["native_compute_samples_ms"] = samples
    if args.oracle:
        oracle = gem.with_suffix(".oracle.pa#")
        result = invoke(["/mnt/c/Program Files/SIMION-2024/simion.exe",
                         "--nogui", "--noprompt", "gem2pa",
                         win_path(str(gem)), win_path(str(oracle))], gem.parent, 30)
        assert result["exit"] == 0, result
        r["simion"] = result
    print(r["file"], r["oracle_points"], "compute ms:", samples,
          "SIMION process wall ms:", r["simion"]["wall_ms"], flush=True)
global_largest = sorted((r for r in results if r.get("oracle_points")),
                        key=lambda r: r["oracle_points"], reverse=True)[:5]
if args.oracle:
    converted_names = {r["file"] for r in largest}
    for r in global_largest:
        if r["file"] in converted_names:
            continue
        gem = root / "examples" / r["file"]
        result = invoke(["/mnt/c/Program Files/SIMION-2024/simion.exe",
                         "--nogui", "--noprompt", "gem2pa",
                         win_path(str(gem)), win_path(str(gem.with_suffix(".oracle.pa#")))],
                        gem.parent, 30)
        assert result["exit"] == 0, result
        r["simion"] = result
        print(r["file"], "Lua-only native input; SIMION process wall ms:",
              result["wall_ms"], flush=True)
cache.write_text(json.dumps(results, indent=2))
report = REPO / "docs/gem-validation.md"
text = report.read_text().split("\n## Five largest converted arrays\n")[0]
lines = text.rstrip().splitlines()
by_name = {r["file"]: r for r in [*largest, *global_largest]}
for i, line in enumerate(lines):
    cells = line.split("|")
    if len(cells) == 6 and cells[1].strip().strip("`") in by_name:
        r = by_name[cells[1].strip().strip("`")]
        if cells[2].strip() != str(r["oracle_points"]):
            continue
        cells[3] = (f" {r['native_compute_ms']:.6f} "
                    if "native_compute_ms" in r else " unsupported ")
        cells[4] = f" {r['simion']['wall_ms']:.6f} "
        lines[i] = "|".join(cells)
lines += ["", "## Five largest converted arrays", "",
          f"Native values are medians of {args.runs} runs, measured inside "
          "`Gem::rasterize`; parsing, allocation and writing are excluded. "
          "SIMION wall time includes its complete foreground gem2pa process. "
          "Hardware: Ryzen 9 5950X; at most 16 native slab workers. "
          "Lua-only files cannot have a Lua-free native compute measurement.", "",
          "| GEM | Points | Native compute ms (median) | Samples ms | SIMION process wall ms |",
          "| --- | ---: | ---: | --- | ---: |"]
for r in largest:
    lines.append(f"| `{r['file']}` | {r['oracle_points']} | "
                 f"{r['native_compute_ms']:.6f} | "
                 f"`{r['native_compute_samples_ms']}` | "
                 f"{r['simion']['wall_ms']:.6f} |")
report.write_text("\n".join(lines) + "\n")
