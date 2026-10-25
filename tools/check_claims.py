#!/usr/bin/env python3
"""Check published quantitative claims against their recorded sources."""

import argparse
import ast
import csv
import json
import re
import sys
from decimal import Decimal, ROUND_HALF_UP
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
NUMBER = r"(?:\d{1,3}(?:,\d{3})+|\d+)(?:\.\d+)?(?:[eE][+-]?\d+)?"
TOKEN = re.compile(rf"(?<![A-Za-z0-9_]){NUMBER}(?![A-Za-z0-9_])")
UNITS = (
    r"(?:배|%|V\b|m?s\b|[µμu]s\b|mm\b|MiB\b|쪽|점|만|천|개|초|단계|차|건|"
    r"x\b|×|코어|스레드)"
)
QUANTITY = re.compile(
    rf"(?<![A-Za-z0-9_])({NUMBER})"
    rf"(?=\s*(?:[~–-]\s*{NUMBER}\s*)?{UNITS})"
)


def decimal(text):
    return Decimal(str(text).replace(",", "").strip())


def markdown_section(text, heading):
    headings = re.sub(r"```[\s\S]*?```", lambda match: " " * len(match[0]), text)
    headers = list(re.finditer(r"^(#{1,6}) (.+)$", headings, re.MULTILINE))
    matches = [i for i, match in enumerate(headers) if match[2] == heading]
    if len(matches) != 1:
        raise ValueError(f"expected one Markdown heading {heading!r}, got {len(matches)}")
    index = matches[0]
    start = headers[index]
    end = next(
        (match.start() for match in headers[index + 1:] if len(match[1]) <= len(start[1])),
        len(text),
    )
    return text[start.end():end]


def json_selection(data, selector):
    blocks = data["blocks"]
    chapter = selector["chapter"]
    starts = [
        i for i, block in enumerate(blocks)
        if block.get("type") == "h" and block.get("level") == 1
        and block.get("text") == chapter
    ]
    if len(starts) != 1:
        raise ValueError(f"expected one JSON chapter {chapter!r}, got {len(starts)}")
    start = starts[0] + 1
    end = next(
        (i for i in range(start, len(blocks))
         if blocks[i].get("type") == "h" and blocks[i].get("level") == 1),
        len(blocks),
    )
    candidates = [block for block in blocks[start:end] if block["type"] == selector["type"]]
    if "contains" in selector:
        candidates = [
            block for block in candidates if selector["contains"] in block.get("text", "")
        ]
    if len(candidates) != 1:
        raise ValueError(f"expected one JSON block for {selector!r}, got {len(candidates)}")
    value = candidates[0][selector["field"]]
    for key in selector.get("path", []):
        value = value[key]
    return value


def extract(source, root):
    path = root / source["file"]
    if source["kind"] == "csv":
        with path.open(encoding="utf-8", newline="") as handle:
            rows = [
                row for row in csv.DictReader(handle)
                if all(row[key] == value for key, value in source["where"].items())
            ]
        values = [row[source["column"]] for row in rows]
    else:
        if source["kind"] == "json":
            value = json_selection(json.loads(path.read_text(encoding="utf-8")), source["select"])
            if source.get("count"):
                return Decimal(len(value))
            text = str(value)
        else:
            text = path.read_text(encoding=source.get("encoding", "utf-8"))
            if source.get("section") and path.suffix == ".md":
                text = markdown_section(text, source["section"])
        matches = list(re.finditer(source["pattern"], text, re.MULTILINE))
        if source.get("count"):
            return Decimal(len(matches))
        values = [match[source.get("group", 1)] for match in matches]
    if source.get("count"):
        return Decimal(len(values))
    if not values:
        raise ValueError("source selector matched no values")
    numbers = [decimal(value) for value in values]
    if source.get("many"):
        return numbers
    if len(numbers) != 1:
        raise ValueError(f"expected one source value, got {len(numbers)}")
    return numbers[0]


def calculate(expression, resolve):
    """Evaluate only named source values, arithmetic, and min/max/len."""
    def binary(left, right, operation):
        if isinstance(left, list) or isinstance(right, list):
            if not isinstance(left, list):
                left = [left] * len(right)
            if not isinstance(right, list):
                right = [right] * len(left)
            if len(left) != len(right):
                raise ValueError("source arrays have different lengths")
            return [binary(a, b, operation) for a, b in zip(left, right)]
        if isinstance(operation, ast.Add):
            return left + right
        if isinstance(operation, ast.Sub):
            return left - right
        if isinstance(operation, ast.Mult):
            return left * right
        if isinstance(operation, ast.Div):
            return left / right
        raise ValueError("unsupported arithmetic operator")

    def visit(node):
        if isinstance(node, ast.Name):
            return resolve(node.id)
        if isinstance(node, ast.Constant) and type(node.value) in (int, float):
            return decimal(node.value)
        if isinstance(node, ast.BinOp):
            return binary(visit(node.left), visit(node.right), node.op)
        if isinstance(node, ast.Call) and isinstance(node.func, ast.Name) and not node.keywords:
            if node.func.id in ("min", "max") and node.args:
                values = [visit(arg) for arg in node.args]
                return {"min": min, "max": max}[node.func.id](
                    values[0] if len(values) == 1 else values
                )
            if node.func.id == "len" and len(node.args) == 1:
                return Decimal(len(visit(node.args[0])))
        raise ValueError("derivation must use sources, arithmetic, min, max, or len")

    result = visit(ast.parse(expression, mode="eval").body)
    if not isinstance(result, Decimal) or not result.is_finite():
        raise ValueError("derivation did not produce one finite number")
    return result


def source_names(expression):
    return {
        node.id for node in ast.walk(ast.parse(expression, mode="eval"))
        if isinstance(node, ast.Name) and node.id not in ("min", "max", "len")
    }


def number_spans(text, claim):
    """Find the exact claimed token, not every number in its prose fragment."""
    matches = list(re.finditer(re.escape(claim["fragment"]), text))
    if not matches:
        raise ValueError(f"fragment missing verbatim: {claim['fragment']!r}")
    tokens = [match for match in TOKEN.finditer(claim["fragment"]) if match[0] == claim["number"]]
    if len(tokens) != 1:
        raise ValueError("fragment must contain exactly one occurrence of its claimed number")
    token = tokens[0]
    return [(match.start() + token.start(), match.start() + token.end()) for match in matches]


def prose_only(text):
    # Preserve offsets so coverage can be checked against exact original spans.
    patterns = [
        r"```[\s\S]*?```",
        r"`[^`\n]+`",
        r"!?\[[^\]\n]*\]\([^)]+\)",
    ]
    for pattern in patterns:
        text = re.sub(pattern, lambda match: " " * len(match[0]), text)
    return text


def table_numbers(text):
    # Dimension/device names are identifiers, not measured numbers.
    text = re.sub(r"\b3D\b|4중극|HS1", lambda match: " " * len(match[0]), text)
    return TOKEN.finditer(text)


def check(registry, root):
    sources = registry["sources"]
    cache = {}
    source_errors = {}
    readme = (root / registry["readme_file"]).read_text(encoding="utf-8")
    report = json_selection(
        json.loads((root / registry["report_file"]).read_text(encoding="utf-8")),
        registry["report_table"],
    )
    coverage = {"readme": set(), "report": {}}
    passed = failed = 0
    ids = set()

    def resolve(name):
        if name in source_errors:
            raise ValueError(f"{name}: {source_errors[name]}")
        if name not in cache:
            try:
                cache[name] = extract(sources[name], root)
            except (KeyError, ValueError, OSError, ArithmeticError) as error:
                source_errors[name] = str(error)
                raise ValueError(f"{name}: {error}") from error
        return cache[name]

    for section in ("readme", "report_table13"):
        print(f"== {section} ==")
        for claim in registry[section]:
            names = sorted(source_names(claim["derive"]))
            source = "; ".join(
                sorted({
                    f"{sources[name]['file']} [{sources[name]['section']}]" for name in names
                })
            )
            try:
                if claim["id"] in ids:
                    raise ValueError("duplicate claim id")
                ids.add(claim["id"])
                if section == "readme":
                    spans = number_spans(readme, claim)
                    coverage["readme"].update(spans)
                else:
                    cell = tuple(claim["cell"])
                    spans = number_spans(report[cell[0]][cell[1]], claim)
                    coverage["report"].setdefault(cell, set()).update(spans)
                actual = calculate(claim["derive"], resolve)
                published = decimal(claim["number"])
                quantum = decimal(claim["rounding"])
                rounded = actual.quantize(quantum, rounding=ROUND_HALF_UP)
                if rounded != published:
                    raise ValueError(
                        f"derived={actual}, rounded={rounded} (quantum {quantum}), "
                        f"published={published}"
                    )
                if claim.get("relation") == "upper_bound" and actual > published:
                    raise ValueError(
                        f"derived={actual} exceeds literal upper bound {published}; "
                        f"rounding agrees but '이내' does not"
                    )
                print(
                    f"PASS {claim['id']}: {claim['number']} <- {actual} "
                    f"(round {quantum}, half up) | {source}"
                )
                passed += 1
            except (KeyError, IndexError, ValueError, ArithmeticError, OSError) as error:
                print(f"FAIL {claim['id']}: {error} | {source}")
                failed += 1

    uncovered = []
    prose = prose_only(readme)
    for token in QUANTITY.finditer(prose):
        if token.span(1) not in coverage["readme"]:
            line = readme.count("\n", 0, token.start(1)) + 1
            uncovered.append(f"{registry['readme_file']}:{line}: {token[1]}")
    for row, cells in enumerate(report):
        for column, text in enumerate(cells):
            for token in table_numbers(text):
                if token.span() not in coverage["report"].get((row, column), set()):
                    uncovered.append(f"{registry['report_file']} table 13 [{row},{column}]: {token[0]}")
    print("== coverage ==")
    if uncovered:
        for location in uncovered:
            print(f"FAIL unregistered number: {location}")
        failed += len(uncovered)
    else:
        print("PASS all README quantity tokens and table 13 numbers have registry entries")
    print(f"Summary: {passed} claims passed, {failed} failures")
    return 1 if failed else 0


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--registry", type=Path, default=ROOT / "docs/claims.json",
        help="registry JSON (default: docs/claims.json)",
    )
    parser.add_argument("--root", type=Path, default=ROOT, help="repository root")
    args = parser.parse_args()
    try:
        return check(json.loads(args.registry.read_text(encoding="utf-8")), args.root)
    except (KeyError, IndexError, ValueError, ArithmeticError, OSError) as error:
        print(f"FAIL registry: {error}")
        return 1


if __name__ == "__main__":
    sys.exit(main())
