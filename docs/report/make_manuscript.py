import json
import re
import runpy
from pathlib import Path

HERE = Path(__file__).resolve().parent
blocks = runpy.run_path(str(HERE / "build_manuscript.py"))["B"]
order = [b["id"] for b in blocks if b["type"] == "figure"]
number = {fid: i + 1 for i, fid in enumerate(order)}
missing = set()


def resolve(text: str) -> str:
    def sub(m: re.Match[str]) -> str:
        fid = m.group(1)
        if fid not in number:
            missing.add(fid)
            return m.group(0)
        return f"그림 {number[fid]}"

    text = re.sub(r"\[\[(F\d\d)\]\]", sub, text)
    return re.sub(r"((?:그림|표) (\d+))(은|는|이|가|을|를|과|와)(?![가-힣])", particle, text)


FINAL_CONSONANT = {"0": True, "1": True, "2": False, "3": True, "4": False, "5": False, "6": True, "7": True, "8": True, "9": False}
PAIRS = {"은": ("은", "는"), "는": ("은", "는"), "이": ("이", "가"), "가": ("이", "가"), "을": ("을", "를"), "를": ("을", "를"), "과": ("과", "와"), "와": ("과", "와")}


def particle(m: re.Match[str]) -> str:
    digits = m.group(2)
    last = digits[-1]
    has_final = True if last == "0" else FINAL_CONSONANT[last]
    with_final, without_final = PAIRS[m.group(3)]
    return m.group(1) + (with_final if has_final else without_final)


NOTATION = {"A_z": "Az", "A_φ": "Aφ", "rA_φ": "rAφ", "B_r": "Bᵣ", "B_z": "Bz", "E_i": "Eᵢ", "J_z": "Jz", "P_coll": "Pcoll", "Q_j": "Qⱼ", "V_j": "Vⱼ",
            "a_B": "aB", "a_E": "aE", "n_g": "ng", "r_ij": "rᵢⱼ", "v_g": "vg", "ρ_f": "ρf", "τ_abs": "τabs", "τ_rel": "τrel"}
NOTATION_RE = re.compile(r"(?<![A-Za-z0-9_])(" + "|".join(re.escape(k) for k in NOTATION) + r")(?![A-Za-z0-9_])")
adopted = {}
for sub in ("polish", "polish2"):
    adopted_path = HERE / sub / "adopted.json"
    if adopted_path.exists():
        adopted.update(json.loads(adopted_path.read_text(encoding="utf-8")))
original_text: dict[int, tuple[str, str]] = {}
for index, b in enumerate(blocks):
    for key in ("text", "caption"):
        if isinstance(b.get(key), str):
            flat = " ".join(resolve(b[key]).split())
            marked = " ".join(b[key].split())
            new = adopted.get(flat, adopted.get(marked, adopted.get(NOTATION_RE.sub(lambda x: NOTATION[x.group(1)], flat))))
            if new is not None:
                original_text[index] = (key, b[key])
                b[key] = new
reverted = []
for index, b in enumerate(blocks):
    if b["type"] not in ("eq", "table"):
        continue
    neighbours = [index + 1] if b["type"] == "eq" else [index - 1, index + 1]
    if any(b["source"] in resolve(blocks[n].get("text", "")) for n in neighbours if 0 <= n < len(blocks)):
        continue
    for n in neighbours:
        if n in original_text:
            key, text = original_text.pop(n)
            blocks[n][key] = text
            reverted.append(n)
if reverted:
    print(f"reverted polish to keep sources: {sorted(reverted)}")
FINAL_FIXES = {"einzel 렌즈 10만 개를 배정밀도 GPU로 날리면 계산에 0.637 s가 걸렸다. SIMION 기본 품질의 21.37 s보다 33.5배 짧다. 이 값은 bench/fly-gpu.md의 Native precision study 절과 bench/simion-baseline.md의 Fly 절에서 가져왔다.": "einzel 렌즈 10만 개를 배정밀도 GPU로 날리면 계산에 2.46 s가 걸려 SIMION 기본 품질의 21.37 s보다 8.7배 짧다. 예전 기록 0.637 s는 재현되지 않았다. 이 값은 bench/fly-gpu.md의 Native dispatch safety 절과 bench/simion-baseline.md의 Fly 절에서 가져왔다.", "최대 6.4e-5 V 이내로 같았다": "최대 6.42e-5 V 차이로 같았다", "6.4e-5 V 안에서 같다": "최대 6.42e-5 V 차이로 같다", "(/mnt/c/Users/<user>/simion_docs_raw/magnetic_potential.html)": "(SIMION 2024와 함께 설치되는 도움말 문서)"}
for b in blocks:
    for key in ("text", "caption"):
        if isinstance(b.get(key), str):
            for old, new in FINAL_FIXES.items():
                b[key] = b[key].replace(old, new)
            b[key] = NOTATION_RE.sub(lambda m: NOTATION[m.group(1)], b[key])
for b in blocks:
    for key in ("text", "caption", "mention", "source"):
        if isinstance(b.get(key), str):
            b[key] = resolve(b[key])
    if b["type"] == "table":
        b["rows"] = [[resolve(c) for c in row] for row in b["rows"]]
for b in blocks:
    if b["type"] == "figure":
        n = number[b["id"]]
        b["id"] = str(n)
        b["mention"] = f"그림 {n}"
if missing:
    raise SystemExit(f"unresolved figure tokens: {sorted(missing)}")

refs = runpy.run_path(str(HERE / "build_manuscript.py")).get("REFERENCES", [])
manuscript = {
    "citation_style": "author-year",
    "blocks": blocks,
    "reference_heading": {"level": 1, "text": "참고문헌"},
    "references": refs,
    "assets": {fid: {"path": f"figures/{fid}.png"} for fid in order},
}
(HERE / "manuscript.json").write_text(json.dumps(manuscript, ensure_ascii=False, indent=1), encoding="utf-8")
print(f"blocks={len(blocks)} figures={order}")
