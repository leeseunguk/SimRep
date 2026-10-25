"""F06: cut-cell grid convergence (potential and near-surface field) vs grid spacing.

Input: data/F06-cutcell-convergence.csv, copied from docs/cutcell.md
('Grid convergence' table, f64 CPU, residual 1e-12; staircase sphere rows from the
'Measured acceptance' table). Geometry is dimensionless: electrodes at 1 V / 0 V,
domain [-1,1]^d, radii 0.31 / 0.83, so errors are in volts and h is in domain units.
"""
import csv

import numpy as np

import _style as S

rows = list(csv.DictReader(open(S.DATA_DIR / "F06-cutcell-convergence.csv", encoding="utf-8")))
series = [
    ("sphere", "구 (3D)", S.ACCENTS[0], "o"),
    ("planar_cylinders_worst_phase", "평면 동축 원통 (2D, 최악 위상)", S.ACCENTS[1], "s"),
    ("axisymmetric_cylinders", "축대칭 동축 원통", S.ACCENTS[2], "^"),
]
fig, (a, b) = S.figure(9.6, ncols=2)
for key, label, color, m in series:
    r = [x for x in rows if x["geometry"] == key]
    h = np.array([float(x["h"]) for x in r])
    ep = np.array([float(x["max_potential_error_V"]) for x in r])
    ef = np.array([float(x["near_surface_field_error"]) for x in r])
    p_order = np.log(ep[0] / ep[-1]) / np.log(h[0] / h[-1])
    f_order = np.log(ef[0] / ef[-1]) / np.log(h[0] / h[-1])
    a.loglog(h, ep, marker=m, color=color, lw=1.4, ms=4.5, label=f"{label}, 차수 {p_order:.2f}")
    b.loglog(h, ef, marker=m, color=color, lw=1.4, ms=4.5, label=f"{label}, 차수 {f_order:.2f}")
st = [x for x in rows if x["geometry"] == "sphere_staircase"]
a.loglog([float(x["h"]) for x in st], [float(x["max_potential_error_V"]) for x in st], marker="x", ls=":",
         color=S.INK_MUTED, lw=1.2, ms=5, label="구, 계단형 경계 (비교)")
# slope guides
hh = np.array([0.0022, 0.03])
a.loglog(hh, 6.0 * hh**2, color=S.INK, lw=0.8, ls="--")
a.text(0.0030, 6.0 * 0.0030**2 * 2.2, "기울기 2", fontsize=S.FS_SMALL, color=S.INK, ha="right", va="bottom")
b.loglog(hh, 3.0 * hh, color=S.INK, lw=0.8, ls="--")
b.text(0.0030, 3.0 * 0.0030 * 1.45, "기울기 1", fontsize=S.FS_SMALL, color=S.INK, ha="right", va="bottom")
for ax in (a, b):
    ax.set_xlabel("격자 간격 h (무차원, 영역 [-1, 1] 기준)")
    ax.grid(True, which="major")
    ax.set_axisbelow(True)
    S.log_ticks(ax.yaxis)
    ax.set_xticks([0.002, 0.005, 0.01, 0.02, 0.03])
    ax.set_xticklabels(["0.002", "0.005", "0.01", "0.02", "0.03"])
    ax.minorticks_off()
a.set_ylabel("최대 전위 오차 (V, 전극 1 V 기준)")
b.set_ylabel("표면 근처 최대 전기장 오차 (V/단위 길이)")
a.set_title("(a) 전위", fontsize=S.FS_BASE)
b.set_title("(b) 표면 두 칸 안 전기장", fontsize=S.FS_BASE)
for ax in (a, b):
    ax.legend(fontsize=6.8, loc="upper left", bbox_to_anchor=(-0.02, -0.2), handlelength=1.8, frameon=False)
fig.tight_layout(w_pad=1.5)
S.save(fig, "F06")
