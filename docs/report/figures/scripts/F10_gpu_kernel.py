"""F10: GPU flight-kernel compute time before/after the cached-power Horner specialization.

Input: data/F10-gpu-kernel.csv from bench/fly-gpu.md ('Cached-power Horner specialization',
'Retained before/after controls'). Vulkan compute-pass timestamps on the RTX 3070, the same
solved templates and beam, results bitwise identical between kernels.
"""
import csv

import numpy as np

import _style as S

rows = list(csv.DictReader(open(S.DATA_DIR / "F10-gpu-kernel.csv", encoding="utf-8")))
names = {"einzel": "아인젤", "quadrupole": "4중극"}
fig, axes = S.figure(7.4, ncols=2)
for ax, prec, title in zip(axes, ("f32 Preview", "native f64"), ("(a) 실시간 미리보기 (f32)", "(b) 정밀 (GPU f64)")):
    r = [x for x in rows if x["precision"] == prec]
    labels = [f"{names[x['template']]}\n{int(x['ions']):,}개" for x in r]
    g = np.array([float(x["general_ms"]) for x in r])
    u = np.array([float(x["unrolled_ms"]) for x in r])
    x = np.arange(len(r))
    w = 0.38
    ax.bar(x - w / 2, g, w, color=S.INK_MUTED, label="개선 전 (일반 루프)")
    ax.bar(x + w / 2, u, w, color=S.OURS, label="개선 후 (Horner 전개)")
    for xi, gi, ui in zip(x, g, u):
        ax.text(xi + w / 2, ui * 1.08, f"{ui:.3g}", ha="center", va="bottom", fontsize=6.5, color=S.INK)
        ax.text(xi - w / 2, gi * 1.08, f"{gi:.3g}", ha="center", va="bottom", fontsize=6.5, color=S.INK_MUTED)
    ax.set_yscale("log")
    S.log_ticks(ax.yaxis)
    ax.set_ylim(1, max(g) * 3)
    ax.set_xticks(x)
    ax.set_xticklabels(labels)
    ax.tick_params(axis="x", length=0)
    ax.set_title(title, fontsize=S.FS_BASE)
    ax.yaxis.grid(True)
    ax.set_axisbelow(True)
axes[0].set_ylabel("GPU 계산 시간 (ms, 로그 축)")
axes[0].legend(loc="upper left", fontsize=S.FS_SMALL)
fig.tight_layout(w_pad=1.5)
S.save(fig, "F10")
for x in rows:
    print(x["precision"], x["template"], x["ions"], f"{float(x['general_ms'])/float(x['unrolled_ms']):.3f}x")
