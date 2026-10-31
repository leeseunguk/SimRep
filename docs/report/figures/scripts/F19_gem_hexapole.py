"""F19: GEM Lua hexapole -> PA#, SimRep gem2pa vs SIMION gem2pa, node by node.

--extract <ours.pa#> <simion.pa#>: decode both PA# files (electrode nodes are stored as
value + 2*max_voltage) into electrode-number maps and save data/F19-hexapole-masks.npz.
  ours:   cd target/report-figs/hex && simrep gem2pa hexapole.gem hexapole.pa#
  SIMION: %USERPROFILE%\\simrep-oracle\\_gem\\examples\\geometry\\hexapole.oracle.pa#
          (SIMION 2024 gem2pa of the same installed example, see docs/gem-validation.md)
"""
import struct
import sys
from array import array

import numpy as np
from matplotlib.colors import ListedColormap
from matplotlib.lines import Line2D
from matplotlib.patches import Patch, Rectangle

import _style as S

NPZ = S.DATA_DIR / "F19-hexapole-masks.npz"


def decode(path):
    with open(path, "rb") as f:
        mode, _sym, vmax, nx, ny, nz, _mirror = struct.unpack("<iidiiii", f.read(32))
        dx = struct.unpack("<ddd", f.read(24))[0] if mode == -2 else 1.0
        v = array("d")
        v.fromfile(f, nx * ny * nz)
    a = np.array(v).reshape(ny, nx)
    el = np.where(a > vmax, a - 2 * vmax, 0.0)
    return el, dx


if "--extract" in sys.argv:
    i = sys.argv.index("--extract")
    ours, dx = decode(sys.argv[i + 1])
    simion, dx2 = decode(sys.argv[i + 2])
    assert ours.shape == simion.shape and dx == dx2
    np.savez_compressed(NPZ, ours=ours.astype(np.int8), simion=simion.astype(np.int8), dx=dx)

z = np.load(NPZ)
ours, simion, dx = z["ours"], z["simion"], float(z["dx"])
ny, nx = ours.shape
mism = int((ours != simion).sum())
print("nodes", ours.size, "dims", nx, ny, "mismatch", mism, "electrode nodes", int((ours > 0).sum()))

cmap = ListedColormap(["#ffffff", S.OURS_LIGHT, S.SIMION_LIGHT])
fig, (a, b) = S.figure(7.2, ncols=2, gridspec_kw={"width_ratios": [2.0, 1.0], "wspace": 0.18})
ext = [0, nx * dx, 0, ny * dx]
a.imshow(ours, origin="lower", extent=ext, cmap=cmap, vmin=0, vmax=2, interpolation="nearest")
a.set_xlabel("x (mm)")
a.set_ylabel("y (mm, y 거울 대칭)")
a.set_title(f"(a) SimRep gem2pa 결과 ({nx}×{ny}점, h = {dx:g} mm)", fontsize=S.FS_BASE)
# zoom window on a rod edge
x0, y0, w = 8.12, 2.78, 0.16
a.add_patch(Rectangle((x0, y0), w, w, fill=False, color=S.INK, lw=1.0))
a.annotate("(b)", (x0, y0 + w), xytext=(x0 - 1.4, y0 + 1.2), fontsize=S.FS_SMALL, arrowprops=dict(arrowstyle="-", lw=0.6, color=S.INK))
i0, j0, n = int(round(x0 / dx)), int(round(y0 / dx)), int(round(w / dx))
sub_o, sub_s = ours[j0:j0 + n + 1, i0:i0 + n + 1], simion[j0:j0 + n + 1, i0:i0 + n + 1]
xs = (i0 + np.arange(sub_o.shape[1])) * dx
ys = (j0 + np.arange(sub_o.shape[0])) * dx
b.imshow(sub_o, origin="lower", extent=[xs[0] - dx / 2, xs[-1] + dx / 2, ys[0] - dx / 2, ys[-1] + dx / 2],
         cmap=cmap, vmin=0, vmax=2, interpolation="nearest")
jj, ii = np.nonzero(sub_s > 0)
b.plot(xs[ii], ys[jj], ".", ms=2.2, color=S.INK)
b.set_xlabel("x (mm)")
b.set_title(f"(b) 확대 {w:g}×{w:g} mm: 불일치 {mism}점", fontsize=S.FS_BASE)
b.set_aspect("equal")
handles = [
    Patch(facecolor=S.OURS_LIGHT, label="전극 1 (SimRep)"),
    Patch(facecolor=S.SIMION_LIGHT, label="전극 2 (SimRep)"),
    Line2D([], [], marker=".", ls="none", color=S.INK, ms=6, label="SIMION gem2pa 전극점"),
]
fig.legend(handles=handles, loc="lower center", ncol=3, bbox_to_anchor=(0.5, -0.06), fontsize=S.FS_SMALL)
S.save(fig, "F19")
