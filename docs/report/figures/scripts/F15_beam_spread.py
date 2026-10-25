"""F15: space-charge spreading of a cold uniform 1 mm beam (SIMION-documented beam semantics, CLI).

Beam: data/F15-beam.ion, 500 ions (1 amu, +1 e, 0.5182134834 eV -> v = 10 mm/us along +x), golden-angle
uniform disk of radius 1 mm at x = 0, ion 1 on axis (the beam leader). Total current 1e-8 A.
Each detector plane d = 0.4 ... 4.8 mm is one run of
  simrep fly --device cpu --particles F15-beam.ion --charge-mode beam --beam-current-a 1e-8
    --bounds=-1,-20,-20,10,20,20 --plane 1,0,0,d --nonrelativistic
(data/F15-beam-planes.csv). Analytic envelope: uniform cylinder R'' = C/R,
C = (q/m) * 96.48533202185007 * 2k * I * 1e-6 / v, k = 8.9875517923e12 V mm / C
(docs/physics-space-charge.md; R(0.48 us) = 1.193692751585 mm for R0 = 1 mm).
"""
import csv

import numpy as np
from scipy.integrate import solve_ivp

import _style as S

ions = [l.split(",") for l in open(S.DATA_DIR / "F15-beam.ion").read().split("\n") if l.strip()]
r0 = np.array([np.hypot(float(a[4]), float(a[5])) for a in ions])
rows = list(csv.DictReader(open(S.DATA_DIR / "F15-beam-planes.csv")))
planes = sorted({float(r["plane_x_mm"]) for r in rows})
v = 10.0
C = 96.48533202185007 * 2 * 8.9875517923e12 * 1e-8 * 1e-6 / v
t_end = max(planes) / v
sol = solve_ivp(lambda t, y: [y[1], C / y[0]], [0, t_end], [1.0, 0.0], rtol=1e-12, atol=1e-14, dense_output=True)
xs = np.linspace(0, max(planes), 200)
R = sol.sol(xs / v)[0]

med, lo, hi, rms = [], [], [], []
for p in planes:
    sel = sorted((r for r in rows if float(r["plane_x_mm"]) == p), key=lambda r: int(r["ion"]))
    r1 = np.array([np.hypot(float(r["y_mm"]), float(r["z_mm"])) for r in sel])
    m = r0 > 0.05
    ratio = r1[m] / r0[m]
    med.append(np.median(ratio)); lo.append(np.percentile(ratio, 5)); hi.append(np.percentile(ratio, 95))
    rms.append(np.sqrt(np.mean(r1**2)) / np.sqrt(np.mean(r0**2)))
med, lo, hi, rms = map(np.array, (med, lo, hi, rms))
Ra = sol.sol(np.array(planes) / v)[0]
print("x, analytic, rms-scale, median-scale:")
for p, a, b, c in zip(planes, Ra, rms, med):
    print(f"{p:4.1f} {a:.6f} {b:.6f} {c:.6f}  rms rel diff {b/a-1:+.2e}")

fig, (a, b) = S.figure(7.2, ncols=2, gridspec_kw={"wspace": 0.32})
a.plot(xs, R, color=S.INK, lw=1.0, label="해석해: 균일 원통 R'' = C/R")
a.fill_between(planes, lo, hi, color=S.OURS_LIGHT, alpha=0.6, lw=0, label="SimRep 이온별 r/r₀ (5–95%)")
a.plot(planes, rms, "o", color=S.OURS, ms=4, label="SimRep RMS 반경 비")
a.set_xlabel("검출면 위치 x (mm)")
a.set_ylabel("반경 배율 R/R₀ (–)")
a.set_title("(a) 빔 반경의 증가 (I = 10 nA, 500개)", fontsize=S.FS_BASE)
a.legend(loc="upper left", fontsize=6.8)
a.grid(True)
a.set_axisbelow(True)

sel = sorted((r for r in rows if float(r["plane_x_mm"]) == max(planes)), key=lambda r: int(r["ion"]))
y1 = np.array([float(r["y_mm"]) for r in sel])
z1 = np.array([float(r["z_mm"]) for r in sel])
y0 = np.array([float(a_[4]) for a_ in ions])
z0 = np.array([float(a_[5]) for a_ in ions])
b.plot(y0, z0, ".", ms=2, color=S.INK_MUTED, label="출발 (x = 0)")
b.plot(y1, z1, ".", ms=2, color=S.OURS, label=f"x = {max(planes):g} mm")
th = np.linspace(0, 2 * np.pi, 200)
b.plot(Ra[-1] * np.cos(th), Ra[-1] * np.sin(th), color=S.INK, lw=0.8, ls="--", label=f"해석해 반경 {Ra[-1]:.4f} mm")
b.set_aspect("equal")
b.set_xlabel("y (mm)")
b.set_ylabel("z (mm)")
b.set_title("(b) 단면 분포", fontsize=S.FS_BASE)
b.legend(loc="upper center", bbox_to_anchor=(0.5, -0.2), ncol=2, fontsize=6.5, markerscale=3)
b.set_xlim(-1.45, 1.45)
b.set_ylim(-1.45, 1.45)
S.save(fig, "F15")
