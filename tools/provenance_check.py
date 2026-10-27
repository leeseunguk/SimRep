#!/usr/bin/env python3
"""Check docs/validation/PROVENANCE.md against the committed files it lists.

Every row of the ledger's table names one reference/fixture/answer-key file,
its first-commit date and the sha256 of its bytes. This script recomputes
each digest and fails when a listed file is missing or its bytes changed.

Use --add PATH to append a ledger row for a new file (origin and licence are
left for the author to fill in - do not guess them).
"""

import argparse
import hashlib
import re
import sys
from datetime import date
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
LEDGER = ROOT / "docs" / "validation" / "PROVENANCE.md"

ROW = re.compile(r"^\| `([^`]+)` \| (.*?) \| (\d{4}-\d{2}-\d{2}|unknown) \| `([0-9a-f]{64})` \| (.*?) \|\s*$")


def sha256(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def rows(ledger):
    for line in ledger.read_text(encoding="utf-8").splitlines():
        match = ROW.match(line)
        if match:
            yield match


def check(ledger, root):
    bad = []
    count = 0
    for match in rows(ledger):
        count += 1
        path, origin, day, digest, licence = match.groups()
        target = root / path
        if not target.is_file():
            bad.append(f"missing file: {path}")
            continue
        actual = sha256(target)
        if actual != digest:
            bad.append(f"changed file: {path} (ledger {digest[:12]}..., actual {actual[:12]}...)")
    if count == 0:
        bad.append(f"no ledger rows parsed from {ledger}")
    for problem in bad:
        print(f"FAIL {problem}")
    print(f"Summary: {count - len(bad)}/{count} ledger rows verified")
    return 1 if bad else 0


def add(ledger, root, path):
    path = Path(path)
    target = path if path.is_absolute() else root / path
    if not target.is_file():
        print(f"FAIL --add: no such file {target}")
        return 1
    try:
        rel = target.resolve().relative_to(root.resolve()).as_posix()
    except ValueError:
        print(f"FAIL --add: {target} is outside the repository root {root}")
        return 1
    if any(match.group(1) == rel for match in rows(ledger)):
        print(f"FAIL --add: {rel} already has a ledger row")
        return 1
    row = f"| `{rel}` | unknown | {date.today().isoformat()} | `{sha256(target)}` | unknown |\n"
    with ledger.open("a", encoding="utf-8") as handle:
        handle.write(row)
    print(f"appended {rel}; fill in its origin and licence")
    return 0


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--ledger", type=Path, default=LEDGER, help="ledger file (default: docs/validation/PROVENANCE.md)")
    parser.add_argument("--root", type=Path, default=ROOT, help="repository root")
    parser.add_argument("--add", metavar="PATH", help="append a ledger row for PATH")
    args = parser.parse_args()
    if args.add:
        return add(args.ledger, args.root, args.add)
    return check(args.ledger, args.root)


if __name__ == "__main__":
    sys.exit(main())
