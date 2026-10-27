#!/usr/bin/env python3
"""Render/check the formula inventory, or execute its linked CPU Rust tests."""
import argparse
import json
from pathlib import Path
import re
import subprocess

ROOT = Path(__file__).resolve().parents[1]
CATALOG = ROOT / "docs/validation/exact_solutions.json"
DOCUMENT = ROOT / "docs/validation/EXACT_SOLUTIONS.md"


def load_catalog():
    catalog = json.loads(CATALOG.read_text())
    if catalog["schema_version"] != 1:
        raise ValueError("unsupported exact-solution schema")
    seen = set()
    for case in catalog["cases"]:
        if case["id"] in seen:
            raise ValueError(f"duplicate case: {case['id']}")
        seen.add(case["id"])
        for field in ("physics", "formulas_latex", "symbols", "validity",
                      "citation", "tests", "gates", "tolerance"):
            if not case[field]:
                raise ValueError(f"missing {field} for {case['id']}")
        if case["citation"]["kind"] == "derived here" and not case["citation"]["derivation"]:
            raise ValueError(f"missing derivation for {case['id']}")
        for test in case["tests"]:
            source = ROOT / "crates" / test["package"] / "tests" / (test["target"] + ".rs")
            if not re.search(r"\bfn\s+" + re.escape(test["name"]) + r"\s*\(", source.read_text()):
                raise ValueError(f"missing linked test for {case['id']}: {test}")
    return catalog


def render(catalog):
    lines = [
        "# Exact-solution verification catalog", "",
        "Generated from `exact_solutions.json` by `python3 tools/exact_solutions.py --print`.",
        "Check the rendering with `--check`; execute every linked CPU public-API test with `--run`.",
        "Computed values, numerical uncertainty, and MISS/OPEN findings are in [GATES.md](GATES.md).",
        "", catalog["units"], "", catalog["method"], "",
        "The inventory follows the formula-to-executable-case approach of "
        "[Iteration-CFD's V&V plan](" + catalog["model"]["url"] + "). "
        "SimRep's implemented equations are specified in [docs/spec/equations.md](../spec/equations.md).",
        "", "## Case index", "",
        "| Case | Physics | Gates |", "| --- | --- | --- |",
    ]
    for case in catalog["cases"]:
        lines.append(f"| [{case['id']}](#{case['id']}) | {case['physics']} | "
                     + ", ".join(f"`{gate}`" for gate in case["gates"]) + " |")
    for case in catalog["cases"]:
        lines.extend(["", f"## {case['id']}", "", case["physics"], ""])
        for formula in case["formulas_latex"]:
            lines.extend(["$$", formula, "$$", ""])
        lines.extend(["| Symbols | Units | Meaning |", "| --- | --- | --- |"])
        for symbol, info in case["symbols"].items():
            lines.append(f"| `{symbol}` | {info['unit']} | {info['meaning']} |")
        lines.extend(["", "**Validity:** " + case["validity"], "",
                      "**Provenance:** " + case["citation"]["kind"] + ".", ""])
        for number, step in enumerate(case["citation"].get("derivation", []), 1):
            lines.append(f"{number}. {step}")
        lines.extend(["", "**Executable checks:**", ""])
        for test in case["tests"]:
            source = (f"../../crates/{test['package']}/tests/{test['target']}.rs")
            lines.append(f"- [`{test['target']}.rs::{test['name']}`]({source}): "
                         f"`cargo test --release -p {test['package']} --test {test['target']} "
                         f"{test['name']} -- --exact --nocapture`")
        lines.extend(["", "**Band and uncertainty:** " + case["tolerance"]])
    lines.extend(["", "## Deliberate omissions", ""])
    for case in catalog["omitted"]:
        lines.append(f"- `{case['id']}`: {case['reason']}")
    return "\n".join(lines) + "\n"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument("--print", action="store_true")
    mode.add_argument("--check", action="store_true")
    mode.add_argument("--run", action="store_true")
    args = parser.parse_args()
    catalog = load_catalog()
    if args.run:
        tests = {tuple(test[key] for key in ("package", "target", "name"))
                 for case in catalog["cases"] for test in case["tests"]}
        for package, target, name in sorted(tests):
            subprocess.run(["cargo", "test", "--release", "-p", package, "--test",
                            target, name, "--", "--exact", "--nocapture"],
                           cwd=ROOT, check=True)
        print(f"{len(catalog['cases'])} cases checked by {len(tests)} linked CPU tests")
    elif args.check:
        if DOCUMENT.read_text() != render(catalog):
            raise SystemExit("EXACT_SOLUTIONS.md is stale; render tools/exact_solutions.py --print")
        print(f"EXACT_SOLUTIONS.md current: {len(catalog['cases'])} cases")
    else:
        print(render(catalog), end="")


if __name__ == "__main__":
    main()
