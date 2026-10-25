"""F08: per-ion endpoint error of the 3D quadrupole against the physical 8x reference.

Inputs (copied into data/ by this script's companion commands, see README):
  data/F08-physical-8-global.csv   independent physical round-rod reference (h = 0.0625 mm)
  data/F08-simrep-cuts.csv         `simrep fly --cuts` on `simrep refine-geometry` output (user route)
  data/F08-simrep-nocuts.csv       same field, `--no-cuts` (grid-node staircase collisions)
  data/F08-simion-tq209.csv        SIMION 2024 T.Qual 209 on the template staircase PA (FINAL lines
                                   of _quad_gpu/quality-209-readout.json)
Plot: empirical CDF of the endpoint distance, log x axis.
"""
import csv

import numpy as np

import _style as S


def load(path, cols=("x_mm", "y_mm", "z_mm")):
    rows = list(csv.DictReader(open(path, encoding="utf-8")))
    ion = np.array([int(r["ion"]) for r in rows])
    xyz = np.array([[float(r[c]) for c in cols] for r in rows])
    order = np.argsort(ion)
    return ion[order], xyz[order]


ref_ion, ref = load(S.DATA_DIR / "F08-physical-8-global.csv")
series = [
    ("SIMION 2024, T.Qual 209 (계단형 PA)", "F08-simion-tq209.csv", S.SIMION),
    ("SimRep --no-cuts (같은 장, 격자점 충돌)", "F08-simrep-nocuts.csv", S.INK_MUTED),
    ("SimRep 사용자 경로 (refine-geometry → fly --cuts)", "F08-simrep-cuts.csv", S.OURS),
]
FLOOR = 1e-6

fig, ax = S.figure(9.5)
stats = []
for label, fname, color in series:
    ion, xyz = load(S.DATA_DIR / fname)
    assert np.array_equal(ion, ref_ion), fname
    d = np.linalg.norm(xyz - ref, axis=1)
    rms, mx, med = float(np.sqrt(np.mean(d**2))), float(d.max()), float(np.median(d))
    stats.append((label, rms, mx, med, int((d < FLOOR).sum())))
    xs = np.sort(np.maximum(d, FLOOR))
    ys = np.arange(1, len(xs) + 1) / len(xs)
    ax.step(xs, ys, where="post", color=color, lw=1.6, label=f"{label}: RMS {rms:.4g} mm, 최대 {mx:.3g} mm")
ax.set_xscale("log")
S.log_ticks(ax.xaxis)
ax.set_xlim(FLOOR * 0.8, 20)
ax.set_ylim(0, 1.01)
ax.set_xlabel("이온별 도착 위치 오차 |r - r_ref| (mm, 로그 축; 1e-6 mm 미만은 1e-6에 표시)")
ax.set_ylabel("누적 이온 비율 (–)")
ax.xaxis.grid(True, which="major")
ax.yaxis.grid(True)
ax.set_axisbelow(True)
ax.legend(loc="lower left", bbox_to_anchor=(0.0, 1.02), fontsize=S.FS_SMALL, handlelength=1.6, labelspacing=0.5, ncol=1)
S.save(fig, "F08")
for s in stats:
    print("%-55s rms=%.9f max=%.9f median=%.3g below_floor=%d" % s)
