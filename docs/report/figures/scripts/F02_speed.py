"""F02: SIMION compute time / SimRep compute time per measured example.

Input: data/F02-speed.csv (values copied from the bench/docs tables named in
its `source` column). Only measured SIMION times are used; extrapolated SIMION
baselines (large TOF / einzel beams) are deliberately left out.
"""
import csv

import numpy as np
from matplotlib.patches import Patch
from matplotlib.ticker import FuncFormatter

import _style as S

all_rows = list(csv.DictReader(open(S.DATA_DIR / "F02-speed.csv", encoding="utf-8")))
rows = [r for r in all_rows if r["shown"] == "1"]
cats = ["전기장 비행", "공간전하", "자기장 비행", "중성 기체 충돌", "전위 refine", "유전체 전위", "자기정역학"]
colors = dict(zip(cats, S.ACCENTS + ["#5b6773"]))

items = []
for c in cats:
    sub = [r for r in rows if r["category"] == c]
    sub.sort(key=lambda r: float(r["simion_compute_s"]) / float(r["ours_compute_s"]), reverse=True)
    items.extend(sub)

ratios = np.array([float(r["simion_compute_s"]) / float(r["ours_compute_s"]) for r in items])
labels = [f"{r['label']} · {r['size']}" for r in items]

fig, ax = S.figure(15.5)
y = np.arange(len(items))[::-1]
for yi, r, q in zip(y, items, ratios):
    ax.barh(yi, q, height=0.68, color=colors[r["category"]])
    ax.text(q * 1.08, yi, f"{q:,.1f}×" if q < 10 else f"{q:,.0f}×", va="center", ha="left",
            fontsize=S.FS_SMALL, color=S.INK)
ax.set_xscale("log")
ax.set_xlim(1, 700)
ax.set_yticks(y)
ax.set_yticklabels(labels)
ax.tick_params(axis="y", length=0)
ax.axvline(1, color=S.INK_MUTED, lw=0.8)
ax.xaxis.grid(True, which="major")
ax.set_axisbelow(True)
ax.set_xlabel("속도 배수 = SIMION 계산 시간 ÷ SimRep 계산 시간 (배, 로그 축)")
handles = [Patch(color=colors[c], label=c) for c in cats]
ax.legend(handles=handles, loc="lower left", bbox_to_anchor=(-0.02, 1.0), ncol=4, handlelength=1.0,
          columnspacing=1.2, borderaxespad=0.3)
ax.xaxis.set_major_formatter(FuncFormatter(lambda v, _: f"{v:g}"))
ax.set_ylim(-0.6, len(items) - 0.4)
S.save(fig, "F02")

for r in all_rows:
    q = float(r["simion_compute_s"]) / float(r["ours_compute_s"])
    print(f"{r['shown']} {r['label']:<30} {r['size']:<14} {float(r['simion_compute_s']):>9} / {float(r['ours_compute_s']):>11} = {q:8.2f}x")
