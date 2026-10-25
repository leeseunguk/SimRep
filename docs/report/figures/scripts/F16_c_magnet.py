"""F16: magnetostatics, SIMION example c_magnet_2dp (planar Az, iron C core mu_r = 1000, coil I = 2000 A).

--extract <ours_Az.pa> <simion_strict_Az.pa> <mu_labels.pa> <jz_labels.pa>
    saves data/F16-c-magnet.npz (Az arrays and the material/current label maps).
  ours:   `simrep refine Az-phys.pa --magnetic-material c_magnet_phys.json --device cpu --tolerance 1e-10`
          on copies of the audit export with the example Lua's own conversions applied
          (mu label 1 -> 1, else 1000; jz labels scaled by I/area = 2000 A / 2500 mm^2).
  SIMION: c_magnet_2dp-Az.strict.pa (SIMION 2024, convergence 1e-7; docs/magnetostatics.md audit).
B = curl(Az ez): Bx = dAz/dy, By = -dAz/dx, ng = 1, grid 1 mm -> gauss.
"""
import struct
import sys

import numpy as np
from matplotlib.colors import LogNorm

import _style as S

NPZ = S.DATA_DIR / "F16-c-magnet.npz"


def read(path):
    b = open(path, "rb").read()
    mode, _s, vmax, nx, ny, nz, _m = struct.unpack_from("<iidiiii", b, 0)
    off = 32 + (24 if mode == -2 else 0)
    v = np.frombuffer(b, dtype="<f8", count=nx * ny * nz, offset=off)
    return np.where(v > vmax, v - 2 * vmax, v).reshape(ny, nx)


if "--extract" in sys.argv:
    i = sys.argv.index("--extract")
    o, s, mu, jz = (read(p) for p in sys.argv[i + 1:i + 5])
    np.savez_compressed(NPZ, ours=o, simion=s, mu=mu, jz=jz)

z = np.load(NPZ)
o, s, mu, jz = z["ours"], z["simion"], z["mu"], z["jz"]
ny, nx = o.shape


def bmag(a):
    by_, bx_ = np.gradient(a, 1.0)  # d/dy, d/dx in mm
    return np.hypot(by_, -bx_)


Bo, Bs = bmag(o), bmag(s)
x = np.arange(nx)
y = np.arange(ny)
I, mu_in, gap = 2000.0, 1000.0, 20.0
nu0 = 1 / (4 * np.pi * 1e-7)
Lc, La = (100 + gap) * 2 + 150 * 2 + 100 * 2, gap * 2
B_amp = 2 * I / (nu0 / mu_in * Lc + nu0 * La) / 1e-3 * 1e4  # gauss, Lua's own estimate
print("max|Az ours - SIMION strict| =", float(np.abs(o - s).max()))
print("B(175,0) ours / SIMION / Ampere estimate:", Bo[0, 175], Bs[0, 175], B_amp)

fig, (a, b) = S.figure(7.6, ncols=2, gridspec_kw={"width_ratios": [1.15, 1.0], "wspace": 0.38})
im = a.pcolormesh(x, y, np.maximum(Bo, 1.0), norm=LogNorm(1, 3e4), cmap="magma", shading="nearest")
a.contour(np.arange(mu.shape[1]) + 0.5, np.arange(mu.shape[0]) + 0.5, mu, levels=[1.5], colors="white", linewidths=0.6)
a.contour(np.arange(jz.shape[1]) + 0.5, np.arange(jz.shape[0]) + 0.5, np.abs(jz), levels=[0.5], colors="#5ad1ff", linewidths=0.6)
a.set_aspect("equal")
a.set_xlabel("x (mm)")
a.set_ylabel("y (mm, y = 0 거울면)")
a.set_title("(a) SimRep |B| (흰 선: 철심, 파란 선: 코일)", fontsize=S.FS_BASE)
cb = fig.colorbar(im, ax=a, orientation="horizontal", fraction=0.06, pad=0.2, aspect=30)
cb.set_label("|B| (G, 로그)")
b.plot(x, Bs[0], color=S.SIMION, lw=2.2, label="SIMION 2024 (수렴 기준 1e-7)")
b.plot(x, Bo[0], color=S.OURS, lw=1.1, ls="--", label="SimRep (상대 잔차 1e-10)")
b.axhline(B_amp, color=S.INK_MUTED, lw=0.8, ls=":")
b.text(248, B_amp * 1.45, f"암페어 법칙 근사\n{B_amp:.0f} G", fontsize=S.FS_SMALL, color=S.INK_MUTED, ha="right")
b.set_yscale("log")
S.log_ticks(b.yaxis)
b.set_xlabel("x (mm, y = 0 축 위)")
b.set_ylabel("|B| (G, 로그 축)")
b.set_title(f"(b) 틈 중앙 x = 175 mm: {Bo[0, 175]:.1f} / {Bs[0, 175]:.1f} G", fontsize=S.FS_BASE)
b.set_ylim(0.15, 2e4)
b.legend(loc="upper center", bbox_to_anchor=(0.5, -0.2), fontsize=S.FS_SMALL)
b.grid(True)
b.set_axisbelow(True)
S.save(fig, "F16")
