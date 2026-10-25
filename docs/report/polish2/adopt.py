import json
import re
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
MARK = re.compile(r"^\[([PC]\d{3})\] ?(.*)$", re.S)
PROTECT = [
    re.compile(r"[-−+]?\d[\d,]*(?:\.\d+)?(?:e[-−+]?\d+)?"),
    re.compile(r"[A-Za-z][A-Za-z0-9_.\-/*]*[A-Za-z0-9_]"),
    re.compile(r"(?:그림|표) \d+|식 \(\d+\)|\d+\.\d+절"),
    re.compile(r"[ᵢⱼᵣₐₕᵀʳᵉᶠ₀²³θρεμΓΔπν]"),
]
NEGATION = re.compile(r"않|없|아니|못|금지|제외|불가|범위 밖")


def parse(path: Path) -> dict[str, str]:
    text = path.read_text(encoding="utf-8")
    text = text.split("<!-- HUMANIZE-SUMMARY")[0]
    if "[원문 시작]" in text:
        text = text.split("[원문 시작]", 1)[1]
    out: dict[str, str] = {}
    for chunk in re.split(r"\n\s*\n", text.strip()):
        m = MARK.match(chunk.strip())
        if m:
            out[m.group(1)] = " ".join(m.group(2).split())
    return out


def tokens(text: str) -> list[str]:
    found: list[str] = []
    for pattern in PROTECT:
        found.extend(sorted(pattern.findall(text)))
    return found


def main() -> None:
    adopted: dict[str, str] = {}
    report = []
    for run in sorted(p for p in HERE.iterdir() if p.is_dir()):
        final = run / "final.md"
        before = parse(run / "01_input.txt")
        if not final.exists():
            report.append({"run": run.name, "status": "missing final.md"})
            continue
        after = parse(final)
        summary = final.read_text(encoding="utf-8").split("<!-- HUMANIZE-SUMMARY", 1)
        meta = summary[1][:600] if len(summary) > 1 else ""
        if list(before) != list(after):
            report.append({"run": run.name, "status": "marker mismatch", "missing": sorted(set(before) - set(after))})
        for key, original in before.items():
            new = after.get(key)
            if new is None or new == original:
                continue
            reasons = []
            if tokens(original) != tokens(new):
                reasons.append("protected tokens changed")
            if len(NEGATION.findall(original)) > len(NEGATION.findall(new)):
                reasons.append("negation/limit weakened")
            if reasons:
                report.append({"run": run.name, "id": key, "rejected": reasons, "before": original, "after": new})
            else:
                adopted[original] = new
        report.append({"run": run.name, "status": "done", "summary": meta})
    (HERE / "adopted.json").write_text(json.dumps(adopted, ensure_ascii=False, indent=1), encoding="utf-8")
    (HERE / "adopt-report.json").write_text(json.dumps(report, ensure_ascii=False, indent=1), encoding="utf-8")
    rejected = sum(1 for r in report if "rejected" in r)
    print(f"adopted={len(adopted)} rejected={rejected}")
    if "--show" in sys.argv:
        for r in report:
            if "rejected" in r or r.get("status") not in (None, "done"):
                print(json.dumps(r, ensure_ascii=False)[:600])


if __name__ == "__main__":
    main()
