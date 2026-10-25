"""F12: calibrated solenoid coil map vs exact Biot-Savart (SIMION example solenoid, 100 ions).

Data (current release CLI, see data/F12-solenoid-cli-log.txt for the exact commands and stdout):
  F12-solenoid-map.csv    simrep fly --device cpu --run-file .../solenoid/run.json --particles .../ions.ion
                          --bounds=-150,-50,-50,150,50,50 --nonrelativistic           (default: coil map)
  F12-solenoid-exact.csv  the same with --exact-coils (exact Biot-Savart at every RK stage)
  F12-solenoid-native-reference.csv  retained tighter native reference (0.0025 us cap, 1e-12)
  (F12-solenoid-simion-final.csv is kept but NOT plotted: docs/physics-magnetic-collision.md and
   bench/default-flight.md note that the saved solenoid ions/native-reference predate later random
   SIMION launches in flight.log, so SIMION FINAL rows do not correspond to these ions.)
Calibration maxima come from the cpu_coil_map lines of the map run (512 off-node probes per region).
"""
import csv
import re

import numpy as np

import _style as S


def load(name):
    rows = sorted(csv.DictReader(open(S.DATA_DIR / name)), key=lambda r: int(r["ion"]))
    return np.array([[float(r[k]) for k in ("x_mm", "y_mm", "z_mm")] for r in rows])


m, e, ref, sim = (load(f"F12-solenoid-{n}.csv") for n in ("map", "exact", "native-reference", "simion-final"))
d_me = np.linalg.norm(m - e, axis=1)
d_mr = np.linalg.norm(m - ref, axis=1)
d_sr = np.linalg.norm(sim - ref, axis=1)
log = (S.DATA_DIR / "F12-solenoid-cli-log.txt").read_text()
cal = [(int(g[0]), float(g[1]), float(g[2])) for g in
       re.findall(r"cpu_coil_map: region=(\d+), nodes=\d+, max_abs_gauss=(\S+), max_relative=(\S+),", log)][:3]
fly = re.findall(r"fly_s: (\S+),", log)
print("max |map-exact| mm", d_me.max(), "max |map-ref|", d_mr.max(), "fly_s map/exact", fly)

fig, (a, b) = S.figure(7.6, ncols=2, gridspec_kw={"width_ratios": [1.5, 1.0], "wspace": 0.42})
ion = np.arange(1, len(m) + 1)
d_er = np.linalg.norm(e - ref, axis=1)
a.semilogy(ion, np.maximum(d_me, 1e-12), "o", ms=3, color=S.OURS, label="코일 지도 - 정확한 Biot–Savart (같은 설정)")
a.semilogy(ion, np.maximum(d_er, 1e-12), "s", ms=3, mfc="none", color=S.INK_MUTED, label="정확한 Biot–Savart - 엄격 기준해 (참고)")
S.log_ticks(a.yaxis)
a.set_xlabel("이온 번호 (–)")
a.set_ylabel("끝점 위치 차이 (mm, 로그; 1e-12 미만은 바닥에)")
a.set_title(f"(a) 끝점 차이, 비행 {float(fly[0]):.3f} s (지도) / {float(fly[1]):.3f} s (정확)", fontsize=S.FS_BASE)
a.legend(loc="upper center", bbox_to_anchor=(0.5, -0.2), fontsize=6.8)
a.grid(True)
a.set_axisbelow(True)

names = ["안쪽 관", "입구\n가장자리", "출구\n가장자리"]
x = np.arange(len(cal))
b.bar(x - 0.2, [c[1] for c in cal], 0.4, color=S.OURS, label="최대 절대 오차 (G)")
b.bar(x + 0.2, [c[2] for c in cal], 0.4, color=S.OURS_LIGHT, label="최대 상대 오차 (–)")
b.axhline(2e-9, color=S.OURS, lw=0.8, ls="--")
b.axhline(5e-10, color=S.INK_MUTED, lw=0.8, ls=":")
b.text(2.45, 2.4e-9, "허용 2e-9 G", fontsize=6.8, ha="right", color=S.OURS)
b.text(2.45, 3.2e-10, "허용 5e-10", fontsize=6.8, ha="right", color=S.INK_MUTED)
b.set_yscale("log")
S.log_ticks(b.yaxis)
b.set_ylim(1e-12, 2e-8)
b.set_xticks(x)
b.set_xticklabels(names, fontsize=S.FS_SMALL)
b.set_ylabel("보정 표본 오차 (G 또는 –, 로그)")
b.set_title("(b) 지도 영역별 보정 (표본 512개)", fontsize=S.FS_BASE)
b.legend(loc="upper right", fontsize=6.8)
S.save(fig, "F12")
