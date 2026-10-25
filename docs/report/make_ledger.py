import json
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent

EQUATIONS = {
    1: ("3장 식 (1)", "docs/cpu-solver.md의 Discrete operator 절", "adapted-standard", "docs/cpu-solver.md", "Discrete operator; crates/simrep-solve/src/mg.rs solve"),
    2: ("3.1절 식 (2)", "docs/cpu-solver.md의 Convergence and reporting 절", "convention", "docs/cpu-solver.md", "Convergence and reporting; mg.rs reached"),
    3: ("3.3절 식 (3)", "Gibou et al., 2002의 식 (21)", "adapted", "docs/cutcell.md", "Operator and symmetry invariant; Gibou et al. 2002 eq. (21)"),
    4: ("4장 식 (4)", "crates/simrep-fly/src/flight.rs의 derivative 함수", "adapted-standard", "crates/simrep-fly/src/flight.rs", "velocity, derivative; crates/simrep-fly/src/lib.rs constants"),
    5: ("4.4절 식 (5)", "이 문서의 판정 규약", "convention", "docs/PLAN.md", "1. 목표: 8배 격자 기준 판정"),
    6: ("5.1절 식 (6)", "docs/physics-magnetic-collision.md의 Units and magnetic fields 절", "adapted-standard", "docs/physics-magnetic-collision.md", "Units and magnetic fields; physics/magnetic.rs Coil::field_gauss"),
    7: ("5.2절 식 (7)", "docs/physics-magnetic-collision.md의 HS1 절", "adapted-standard", "docs/physics-magnetic-collision.md", "HS1; physics/collision.rs collision_rate_per_us"),
    8: ("5.3절 식 (8)", "docs/dielectric.md의 Discretization and interfaces 절", "adapted-standard", "docs/dielectric.md", "Discretization and interfaces; dielectric.rs harmonic"),
    9: ("5.4절 식 (9)", "docs/physics-space-charge.md의 Direct Coulomb 절", "adapted-standard", "docs/physics-space-charge.md", "Direct Coulomb; physics/space_charge.rs DirectCharge::field"),
    10: ("5.5절 식 (10)", "docs/magnetostatics.md의 Equations and flight contract 절", "adapted-standard", "docs/magnetostatics.md", "Equations and flight contract; magnetostatics.rs scalar"),
    11: ("5.5절 식 (11)", "docs/magnetostatics.md의 Equations and flight contract 절", "adapted-standard", "docs/magnetostatics.md", "Equations and flight contract; magnetostatics.rs currents"),
}

TABLES = {
    1: ("1.1절 표 1 환경", "docs/PLAN.md의 2절 표와 bench/simion-baseline.md", "measured", "docs/PLAN.md", "2. 환경과 제약; bench/simion-baseline.md preamble"),
    2: ("2장 표 2 크레이트", "각 크레이트 Cargo.toml(예: crates/simrep-core/Cargo.toml)의 description 항목과 근거 열의 구현 문서 절", "measured", "crates/simrep-core/Cargo.toml", "description of all 11 crates; row-level implementation sections in the 근거 column"),
    3: ("3.2절 표 3 refine 시간", "bench/refine-real.md End-to-end performance, bench/simion-baseline.md Refine", "measured", "bench/refine-real.md", "End-to-end performance (lines ~163-185)"),
    4: ("3.3절 표 4 cut-cell", "docs/cutcell.md Measured acceptance", "measured", "docs/cutcell.md", "Measured acceptance (2026-10-23)"),
    5: ("4.5절 표 5 einzel", "bench/fly-real.md, bench/default-flight.md, bench/simion-baseline.md", "measured", "bench/fly-real.md", "Same 1,000 ions, same independent 8x reference; default-flight Main integration; simion-baseline Fly"),
    6: ("4.6절 표 6 4중극", "bench/fly-real.md Compute time at achieved precision, docs/geometry-pipeline.md V3", "measured", "bench/fly-real.md", "Compute time at achieved precision; Source-geometry cuts v3"),
    7: ("4.8절 표 7 앱 갱신", "docs/dev/frontend.md 제출/수거 파이프라인 절", "measured", "docs/dev/frontend.md", "제출/수거 파이프라인과 작은 비행 패킷"),
    8: ("5.1절 표 8 자기장", "bench/magnetic-cpu.md Actual matched-population compute", "measured", "bench/magnetic-cpu.md", "Actual matched-population compute"),
    9: ("5.3절 표 9 유전체", "docs/dielectric.md Automatic selection at the default residual tolerance", "measured", "docs/dielectric.md", "Automatic selection at the default residual tolerance"),
    10: ("5.6절 표 10 외부 자기장 지도", "bench/bfield-import.md의 Cartesian full-flight errors 절", "measured", "bench/bfield-import.md", "Cartesian full-flight errors"),
    11: ("6장 표 11 호환", "docs/gem-validation.md, docs/fly2-validation.md, bench/refine-real.md, docs/dev/frontend.md", "measured", "docs/gem-validation.md", "summary counts; fly2-validation summary; refine-real PA0 loading; frontend IOB"),
    12: ("6장 표 12 STEP", "docs/step-import.md Measured accuracy and timing", "measured", "docs/step-import.md", "Measured accuracy and timing"),
    13: ("8장 표 13 종합", "표 아래 문단에 행별로 적은 원본 문서 절에서 가져와 이 문서에서 모은 것", "derived", "bench/refine-real.md", "rows cite bench/refine-real.md, docs/cutcell.md, bench/default-flight.md, bench/fly-real.md, docs/geometry-pipeline.md, bench/magnetic-cpu.md, docs/dielectric.md, bench/space-charge.md, docs/magnetostatics.md"),
    14: ("9장 표 14 한계", "근거 열에 적은 각 문서 절의 한계를 이 문서에서 모은 것", "derived", "docs/report/evidence-digest.md", "9. Limitations"),
}


def main() -> None:
    manuscript = json.loads((HERE / "manuscript.json").read_text(encoding="utf-8"))
    figures = [b for b in manuscript["blocks"] if b["type"] == "figure"]
    figure_ledger = {}
    ledger_path = HERE / "figure-ledger.json"
    if ledger_path.exists():
        figure_ledger = {row["id"]: row for row in json.loads(ledger_path.read_text(encoding="utf-8"))}
    status = sys.argv[1] if len(sys.argv) > 1 else "unverified"
    rows = []
    for number, (location, citation, kind, file, symbol) in EQUATIONS.items():
        rows.append(_row(f"eq:{number}", location, citation, kind, file, symbol, status))
    for number, (location, citation, kind, file, symbol) in TABLES.items():
        rows.append(_row(f"table:{number}", location, citation, kind, file, symbol, status))
    for block in figures:
        fid = block["asset"]
        info = figure_ledger.get(fid, {})
        rows.append(
            _row(
                f"fig:{block['id']}",
                block["mention"],
                block["source"],
                "measured",
                f"figures/{fid}.png",
                str(info.get("source", "figure-ledger.json")),
                status,
            )
        )
    (HERE / "source-ledger.json").write_text(json.dumps(rows, ensure_ascii=False, indent=1), encoding="utf-8")
    print(f"rows={len(rows)} status={status}")


def _row(item, location, citation, kind, file, symbol, status):
    kind = "adapted" if kind == "adapted-standard" else kind
    if kind == "adapted" and "Gibou" not in citation:
        kind = "derived"
    return {
        "item": item,
        "location": location,
        "citation": citation,
        "kind": kind,
        "checked_against": {"file": file, "symbol": symbol},
        "status": status,
    }


if __name__ == "__main__":
    main()
