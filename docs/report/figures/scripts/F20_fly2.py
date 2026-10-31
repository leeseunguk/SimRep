"""F20: FLY2 particle definitions, SimRep vs SIMION create_ionfile (no flights).

Native rows: `target/release/examples/particles <SIMION-2024/examples/...fly2> "" <seed>` (the generator
used by tools/simion-oracle/check_fly2.py), seeds 1..8 for random sources.
SIMION rows: the retained oracle exports %USERPROFILE%\\simrep-oracle\\_fly2\\<name>.oracle.txt
(simion.iob2.load_fly2_file(...):create_ionfile, seeds 1..8; docs/fly2-validation.md).
  deterministic: bradbury_nielsen_grid/bngrid.fly2 (11 TOB x 21 positions = 231 particles)
  random:        plot/tof_histogram.fly2 (5000 per seed: Gaussian TOB mean 2 us FWHM 1 us,
                 filled circle of radius 5 mm at x = 5 mm, normal +x) -> 40,000 per side
Streams are not expected to match sample-for-sample; the random case is compared as distributions.
"""
import csv

import numpy as np
from scipy.stats import ks_2samp

import _style as S


def load(tag, side):
    rows = list(csv.DictReader(open(S.DATA_DIR / f"F20-{tag}-{side}.csv")))
    return {k: np.array([float(r[k]) for r in rows]) for k in rows[0]}


bn, bs = load("bngrid", "simrep"), load("bngrid", "simion")
diff = max(float(np.abs(bn[k] - bs[k]).max()) for k in bn)
tn, ts = load("tofhist", "simrep"), load("tofhist", "simion")
r2n, r2s = tn["y_mm"] ** 2 + tn["z_mm"] ** 2, ts["y_mm"] ** 2 + ts["z_mm"] ** 2
p_tob = ks_2samp(tn["tob_us"], ts["tob_us"]).pvalue
p_r2 = ks_2samp(r2n, r2s).pvalue
print("bngrid max abs diff", diff, "n", len(bn["tob_us"]), "| tofhist KS p tob", p_tob, "r2", p_r2,
      "means", tn["tob_us"].mean(), ts["tob_us"].mean())

fig = S.figure(10.6)[0]
fig.clf()
gs = fig.add_gridspec(2, 2, height_ratios=[1.0, 1.05], hspace=0.62, wspace=0.3)
a = fig.add_subplot(gs[0, :])
b = fig.add_subplot(gs[1, 0])
c = fig.add_subplot(gs[1, 1])

a.plot(bs["y_mm"], bs["tob_us"], "o", ms=6, mfc="none", mec=S.SIMION, mew=1.0, label="SIMION create_ionfile")
a.plot(bn["y_mm"], bn["tob_us"], ".", ms=3.5, color=S.OURS, label="SimRep")
a.set_xlabel("출발 위치 y (mm)")
a.set_ylabel("출발 시각 TOB (µs)")
a.set_title(f"(a) 결정적 정의 bngrid.fly2: 입자 {len(bn['tob_us'])}개, 모든 열 최대 차이 {diff:.1e}", fontsize=S.FS_BASE)
a.legend(loc="upper left", bbox_to_anchor=(1.0, 1.0), fontsize=S.FS_SMALL)
a.grid(True)
a.set_axisbelow(True)

bins = np.linspace(-1, 5, 61)
b.hist(ts["tob_us"], bins=bins, histtype="stepfilled", color=S.SIMION_LIGHT, label="SIMION")
b.hist(tn["tob_us"], bins=bins, histtype="step", color=S.OURS, lw=1.2, label="SimRep")
b.set_xlabel("출발 시각 TOB (µs)")
b.set_ylabel("입자 수 (개/0.1 µs)")
b.set_title(f"(b) 무작위 tof_histogram.fly2: TOB, KS p = {p_tob:.2f}", fontsize=S.FS_BASE)
b.legend(loc="upper right", fontsize=S.FS_SMALL)

for data, label, color, ls, lw in ((r2s, "SIMION", S.SIMION, "-", 2.2), (r2n, "SimRep", S.OURS, "--", 1.2)):
    xs = np.sort(data)
    c.step(xs, np.arange(1, len(xs) + 1) / len(xs), where="post", color=color, ls=ls, lw=lw, label=label)
c.set_xlabel("반경 제곱 y² + z² (mm²)")
c.set_ylabel("누적 비율 (–)")
c.set_title(f"(c) 원판 분포의 r², KS p = {p_r2:.2f}", fontsize=S.FS_BASE)
c.legend(loc="lower right", fontsize=S.FS_SMALL)
c.grid(True)
c.set_axisbelow(True)
S.save(fig, "F20")
