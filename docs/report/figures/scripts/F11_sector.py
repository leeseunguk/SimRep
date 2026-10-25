"""F11: 90-degree magnetic sector (SIMION example magnetic_sector/sector_2dp), 15 ions, CLI flight.

Data (all from the current release CLI unless noted):
  data/F11-sector2dp-samples.csv  positions at fixed times t = 0.2, 0.4, ... us; each time is one run of
      simrep fly --device cpu --run-file <physics-validation-20261023/sector_2dp/run.json>
        --particles <.../sector_2dp/ions.ion> --bounds=-20,-120,-120,20,120,120 --nonrelativistic
        --duration-us t --allow-time-limit
      (ions that already left the workbench report their exit point)
  data/F11-sector2dp-simrep-end.csv   the same command without a duration limit (final endpoints)
  data/F11-sector2dp-simion-final.csv SIMION 2024 FINAL records from .../sector_2dp/flight.log
      (identical captured START states, exported PAs and placements)
"""
import csv

import numpy as np

import _style as S

samples = list(csv.DictReader(open(S.DATA_DIR / "F11-sector2dp-samples.csv")))
end = {int(r["ion"]): r for r in csv.DictReader(open(S.DATA_DIR / "F11-sector2dp-simrep-end.csv"))}
sim = {int(r["ion"]): r for r in csv.DictReader(open(S.DATA_DIR / "F11-sector2dp-simion-final.csv"))}
ions = sorted(end)
d = np.array([np.linalg.norm([float(end[i][k]) - float(sim[i][k]) for k in ("x_mm", "y_mm", "z_mm")]) for i in ions])
dt = np.array([abs(float(end[i]["tof_us"]) - float(sim[i]["tof_us"])) for i in ions])
print("max endpoint |SimRep - SIMION| mm:", d.max(), "max TOF diff us:", dt.max())

fig, (a, b) = S.figure(8.0, ncols=2, gridspec_kw={"width_ratios": [1.35, 1.0], "wspace": 0.3})
for i in ions:
    pts = [(0.0, -100.0, 50.0)] + [(float(r["t_request_us"]), float(r["y_mm"]), float(r["z_mm"]))
                                    for r in samples if int(r["ion"]) == i]
    pts.append((float(end[i]["tof_us"]), float(end[i]["y_mm"]), float(end[i]["z_mm"])))
    pts = np.array(pts)
    a.plot(pts[:, 1], pts[:, 2], color=S.OURS, lw=0.7, alpha=0.8)
a.plot([-100], [50], "o", color=S.INK, ms=4)
a.text(-100, 56, "출발 (y = -100, z = 50 mm)", fontsize=S.FS_SMALL, ha="left")
ye = [float(sim[i]["y_mm"]) for i in ions]
ze = [float(sim[i]["z_mm"]) for i in ions]
a.plot(ye, ze, "s", mfc="none", mec=S.SIMION, ms=5, label="SIMION 끝점")
a.plot([float(end[i]["y_mm"]) for i in ions], [float(end[i]["z_mm"]) for i in ions], "x", color=S.OURS, ms=4,
       label="SimRep 끝점")
a.set_aspect("equal")
a.set_xlabel("y (mm)")
a.set_ylabel("z (mm)")
a.set_title("(a) 궤적 (SimRep CLI, 0.2 µs 간격 위치)", fontsize=S.FS_BASE)
a.legend(loc="lower left", fontsize=S.FS_SMALL)
a.grid(True)
a.set_axisbelow(True)

b.semilogy(ions, np.maximum(d, 1e-9), "o", color=S.OURS, ms=4, label="끝점 위치 차이 (mm)")
b.semilogy(ions, np.maximum(dt, 1e-9), "s", mfc="none", color=S.SIMION, ms=4, label="TOF 차이 (µs)")
S.log_ticks(b.yaxis)
b.set_xlabel("이온 번호 (–)")
b.set_ylabel("|SimRep - SIMION| (mm 또는 µs, 로그 축)")
b.set_title("(b) 이온별 SIMION과의 차이", fontsize=S.FS_BASE)
b.set_xticks(ions[::2])
b.legend(loc="center", fontsize=S.FS_SMALL)
b.grid(True)
b.set_axisbelow(True)
S.save(fig, "F11")
