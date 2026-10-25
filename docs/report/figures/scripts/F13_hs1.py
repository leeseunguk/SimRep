"""F13: HS1 hard-sphere collisions, SimRep CPU vs GPU vs SIMION end-state distributions (stored data).

Inputs (copied into data/ from C:\\Users\\lsu10\\simrep-oracle\\physics-validation-20261023):
  F13-hs1-cpu.csv, F13-hs1-gpu.csv  gpu-physics/hs1-{cpu,gpu}.csv: 4096 ions each, seed 813, CPU f64 and
      RTX 3070 Vulkan native f64 (docs/physics-magnetic-collision.md 'GPU distribution replay, 2026-10-24')
  F13-hs1-simion.csv                collision_hs1/flight.log FINAL rows: SIMION 2024, 4096 ions, seed 237
Case: 200 amu, +1 e, 0.1 eV along +x from the origin, He 0.53 Pa, 273 K, sigma 2.27e-18 m^2, 50 us.
KS p-values are recomputed here with scipy (exact two-sample test).
"""
import csv

import numpy as np
from scipy.stats import ks_2samp

import _style as S


def load(name):
    rows = list(csv.DictReader(open(S.DATA_DIR / name)))
    return {k: np.array([float(r[k]) for r in rows]) for k in ("x_mm", "ke_ev", "tof_us")}


cpu, gpu, sim = load("F13-hs1-cpu.csv"), load("F13-hs1-gpu.csv"), load("F13-hs1-simion.csv")
fig, axes = S.figure(7.0, ncols=2, gridspec_kw={"wspace": 0.3})
for ax, key, xlabel, title in zip(axes, ("x_mm", "ke_ev"), ("50 µs 뒤 x 위치 (mm)", "50 µs 뒤 운동 에너지 (eV)"),
                                 ("(a) x 위치 분포 (이온 4,096개씩)", "(b) 운동 에너지 분포")):
    p_gpu = ks_2samp(cpu[key], gpu[key], method="exact").pvalue
    p_sim = ks_2samp(cpu[key], sim[key], method="exact").pvalue
    for data, label, color, ls, lw in ((sim[key], "SIMION 2024", S.SIMION, "-", 2.2),
                                       (cpu[key], "SimRep CPU f64", S.INK, "-", 1.0),
                                       (gpu[key], "SimRep GPU f64 (RTX 3070)", S.OURS, "--", 1.2)):
        xs = np.sort(data)
        ax.step(xs, np.arange(1, len(xs) + 1) / len(xs), where="post", color=color, ls=ls, lw=lw, label=label)
    ax.set_xlabel(xlabel)
    ax.set_title(f"{title}\nKS p: CPU–GPU {p_gpu:.3f}, CPU–SIMION {p_sim:.3f}", fontsize=S.FS_BASE)
    ax.set_ylim(0, 1.01)
    ax.grid(True)
    ax.set_axisbelow(True)
    print(key, "KS p cpu-gpu", p_gpu, "cpu-simion", p_sim, "medians", np.median(cpu[key]), np.median(gpu[key]), np.median(sim[key]))
axes[1].set_xscale("log")
S.log_ticks(axes[1].xaxis)
axes[0].set_ylabel("누적 이온 비율 (–)")
axes[0].legend(loc="upper center", bbox_to_anchor=(1.1, -0.2), ncol=3, fontsize=S.FS_SMALL)
print("TOF ranges", cpu["tof_us"].min(), cpu["tof_us"].max(), gpu["tof_us"].min(), sim["tof_us"].min())
S.save(fig, "F13")
