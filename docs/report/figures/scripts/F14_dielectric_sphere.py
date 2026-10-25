"""F14: dielectric sphere (SIMION example sphere_3dp, epsilon = 10 inside) in a plate field.

--extract <ours.pa> <simion_strict.pa>: save the z = 0 slices to data/F14-sphere3dp-slices.npz.
  ours:   simrep gem2pa sphere_3dp.gem -> refinable copy -> simrep refine --permittivity
          sphere_3dp-dielectric-eps10.pa --device cpu --tolerance 1e-12 (see README for the commands)
  SIMION: sphere_3dp.simrep-strict.pa (SIMION 2024 refine, convergence 1e-7 V, epsilon 10 via the
          example Lua; docs/dielectric.md 'Strict oracle and compute medians')
Grid 201x101x101, 0.5 mm, y/z mirrors; plate electrodes at x = 0 mm (100 V) and x = 100 mm (0 V)
(a vacuum solve of the same PA is exactly linear, 1 V/mm); sphere centre x = 50 mm, radius 10 mm
(GEM coordinates in mm; the dielectric GEM shifts sampling by half a cell).
"""
import struct
import sys

import numpy as np
from matplotlib.patches import Circle

import _style as S

NPZ = S.DATA_DIR / "F14-sphere3dp-slices.npz"
H = 0.5


def read(path):
    b = open(path, "rb").read()
    mode, _s, vmax, nx, ny, nz, _m = struct.unpack_from("<iidiiii", b, 0)
    off = 32 + (24 if mode == -2 else 0)
    v = np.frombuffer(b, dtype="<f8", count=nx * ny * nz, offset=off)
    v = np.where(v > vmax, v - 2 * vmax, v).reshape(nz, ny, nx)
    return v


if "--extract" in sys.argv:
    i = sys.argv.index("--extract")
    o, s = read(sys.argv[i + 1]), read(sys.argv[i + 2])
    np.savez_compressed(NPZ, ours=o[0], simion=s[0], max_abs_diff_full_grid=float(np.abs(o - s).max()))

z = np.load(NPZ)
o, s = z["ours"], z["simion"]
ny, nx = o.shape
x = np.arange(nx) * H
y = np.arange(ny) * H
cx, r = 50.0, 10.0  # GEM sphere(50,0,0,10) in mm
print("full-grid max |ours - SIMION| =", float(z["max_abs_diff_full_grid"]), "V")

fig, (a, b) = S.figure(8.0, ncols=2, gridspec_kw={"width_ratios": [1.3, 1.0], "wspace": 0.3})
xm = x <= 100
cf = a.contourf(x[xm], y, o[:, xm], levels=np.linspace(0, 100, 21), cmap="viridis")
a.contour(x[xm], y, o[:, xm], levels=np.arange(5, 100, 5), colors="white", linewidths=0.4, alpha=0.8)
a.add_patch(Circle((cx, 0), r, fill=False, ls="--", lw=0.9, color="white"))
a.set_aspect("equal")
a.set_xlim(0, 100)
a.set_ylim(0, 50)
a.set_xlabel("x (mm)")
a.set_ylabel("y (mm)")
a.set_title("(a) SimRep 전위, z = 0 단면 (ε = 10 구)", fontsize=S.FS_BASE)
cb = fig.colorbar(cf, ax=a, orientation="horizontal", fraction=0.06, pad=0.22, aspect=30)
cb.set_label("전위 (V)")
cb.set_ticks([0, 25, 50, 75, 100])

# on-axis field E_x = -dV/dx (central differences), y = z = 0
ex_o = -np.gradient(o[0], H)
ex_s = -np.gradient(s[0], H)
sel = (x >= 20) & (x <= 80)
b.plot(x[sel], ex_s[sel], color=S.SIMION, lw=2.2, label="SIMION 2024 (수렴 기준 1e-7 V)")
b.plot(x[sel], ex_o[sel], color=S.OURS, lw=1.1, ls="--", label="SimRep (상대 잔차 1e-12)")
b.axvspan(cx - r, cx + r, color=S.GRID, zorder=0)
b.text(cx, 0.45, "구 내부\n(ε = 10)", ha="center", fontsize=S.FS_SMALL, color=S.INK_MUTED)
ic = int(round(cx / H))
b.set_xlabel("x (mm, 축 위 y = z = 0)")
b.set_ylabel("전기장 Eₓ (V/mm)")
b.set_title(f"(b) 축 위 전기장, 중심 {ex_o[ic]:.4f} / {ex_s[ic]:.4f} V/mm", fontsize=S.FS_BASE)
b.set_ylim(0, float(max(ex_o[sel].max(), ex_s[sel].max())) * 1.08)
b.axhline(1.0, color=S.INK_MUTED, lw=0.7, ls=":")
b.text(62, 0.88, "인가 전기장 1 V/mm", fontsize=S.FS_SMALL, color=S.INK_MUTED)
b.legend(loc="upper center", bbox_to_anchor=(0.5, -0.2), fontsize=S.FS_SMALL, ncol=1)
b.grid(True)
b.set_axisbelow(True)
S.save(fig, "F14")
print("center Ex ours/simion:", ex_o[ic], ex_s[ic], "applied field 100 V / 100 mm = 1 V/mm")
