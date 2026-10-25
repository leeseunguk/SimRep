"""F17: imported external B-field maps, full-flight endpoint/TOF error vs map spacing.

Input: data/F17-bfield-import-20261024.csv = bench/bfield-import-20261024.csv (be-bmap-import, merged
2026-10-25), produced by `cargo test -p simrep-fly --release --test bfield_import -- --nocapture`.
Errors are re-imported map vs the exact analytic/Biot-Savart field, same ion and RK45 settings
(1e-12, 0.05 us ceiling, detector x = 1.4 mm). Cartesian rows use the generic delimited export
(bench/bfield-import.md: CSV, Opera, COMSOL spreadsheet and COMSOL grid give identical errors).
Uniform fields reproduce exactly (0 error) and are not drawn on log axes. Timing columns are
marked contended in the CSV and are deliberately not plotted.
"""
import csv

import numpy as np

import _style as S

rows = list(csv.DictReader(open(S.DATA_DIR / "F17-bfield-import-20261024.csv")))
series = [
    ("loop", "Delimited", "원형 코일, 3D 격자", S.ACCENTS[0], "o", "-"),
    ("solenoid", "Delimited", "솔레노이드, 3D 격자", S.ACCENTS[1], "s", "-"),
    ("sector", "Delimited", "섹터 1/r 장, 3D 격자", S.ACCENTS[2], "^", "-"),
    ("loop", "Axisymmetric", "원형 코일, 축대칭 (z, r)", S.ACCENTS[0], "o", "--"),
    ("solenoid", "Axisymmetric", "솔레노이드, 축대칭 (z, r)", S.ACCENTS[1], "s", "--"),
]
fig, (a, b) = S.figure(9.4, ncols=2, gridspec_kw={"wspace": 0.34})
for field, fmt, label, color, m, ls in series:
    r = sorted((x for x in rows if x["field"] == field and x["format"] == fmt), key=lambda x: -float(x["h_mm"]))
    h = np.array([float(x["h_mm"]) for x in r])
    ep = np.array([float(x["endpoint_mm"]) for x in r])
    tf = np.array([float(x["tof_us"]) for x in r])
    mfc = color if ls == "-" else "white"
    a.loglog(h, ep, marker=m, ls=ls, color=color, mfc=mfc, ms=4.5, lw=1.3, label=label)
    b.loglog(h, tf, marker=m, ls=ls, color=color, mfc=mfc, ms=4.5, lw=1.3, label=label)
    print(field, fmt, list(zip(h, ep, tf)))
for ax, c4, fac, xt in ((a, 1.2e-8, 0.35, 0.42), (b, 1.0e-6, 3.0, 0.22)):
    hh = np.array([0.2, 0.8])
    ax.loglog(hh, c4 * (hh / 0.8) ** 4, color=S.INK, lw=0.8, ls=":")
    ax.text(xt, c4 * (xt / 0.8) ** 4 * fac, "기울기 4", fontsize=S.FS_SMALL, color=S.INK)
    ax.set_xticks([0.2, 0.4, 0.8])
    ax.set_xticklabels(["0.2", "0.4", "0.8"])
    ax.minorticks_off()
    S.log_ticks(ax.yaxis)
    ax.set_xlabel("지도 격자 간격 h (mm)")
    ax.grid(True)
    ax.set_axisbelow(True)
a.set_ylabel("끝점 위치 오차 (mm, 로그)")
b.set_ylabel("TOF 오차 (µs, 로그)")
a.set_title("(a) 정확한 장 대비 끝점 오차", fontsize=S.FS_BASE)
b.set_title("(b) 정확한 장 대비 TOF 오차", fontsize=S.FS_BASE)
a.legend(loc="upper center", bbox_to_anchor=(1.12, -0.2), ncol=3, fontsize=S.FS_SMALL)
S.save(fig, "F17")
