"""F04: quadrupole cross-section at x = 30 mm, grid staircase vs cut-cell boundary.

Step 1 (--extract): read the real CLI outputs and write data/F04-slice.json
  cut-cell: `simrep refine-geometry quadrupole.gem ... --out target/report-figs/quad/quadrupole`
            (bases .pa1..pa3 and quadrupole.cuts.json, v3)
  staircase: `simrep refine target/report-figs/stair/quadrupole.pa# --tolerance 1e-12`
            (the same nodal electrode flags, ordinary grid-node boundary)
  Potential = 2.5 V * pa1/10000 - 2.5 V * pa2/10000 + 0 V * pa3 (fast adjust, SIMION convention).
Step 2: plot. Grey squares = node cells whose four in-plane corners are electrode nodes
(SIMION's occupied-cell convention); dots = cut-cell link/surface intersections stored in
cuts.json; dashed circle = the GEM rod surface (r = 5.75 mm at 10.75 mm).
"""
import json
import struct
import sys
from array import array
from pathlib import Path

import numpy as np
from matplotlib.lines import Line2D
from matplotlib.patches import Circle, Patch, Rectangle

import _style as S

OUT = S.DATA_DIR / "F04-slice.json"
XI = 60  # x = 30 mm


def read_pa(path):
    with open(path, "rb") as f:
        mode, _sym, _vmax, nx, ny, nz, _mirror = struct.unpack("<iidiiii", f.read(32))
        if mode == -2:
            f.read(24)
        v = array("d")
        v.fromfile(f, nx * ny * nz)
    return np.array(v).reshape(nz, ny, nx)


def extract(root: Path):
    quad, stair = root / "quad", root / "stair"
    cj = json.load(open(quad / "quadrupole.cuts.json"))
    nx, ny, nz = cj["dims"]
    h = cj["spacing_mm"][0]
    node = np.zeros(nx * ny * nz, dtype=int)
    for a, b, v in cj["node_electrode_runs"]:
        node[a:b] = 0 if v is None else v
    node = node.reshape(nz, ny, nx)[:, :, XI]

    def pot(d):
        b = [read_pa(d / f"quadrupole.pa{i}")[:, :, XI] for i in (1, 2)]
        return (2.5 * b[0] - 2.5 * b[1]) / 10000.0

    hits = []
    for rec in cj["link_intersections"]:
        n = rec["node"]
        i, j, k = n % nx, (n // nx) % ny, n // (nx * ny)
        if i != XI:
            continue
        # directions -x,+x,-y,+y,-z,+z ; keep in-plane (y, z) links
        for di, (dy, dz) in zip((2, 3, 4, 5), ((-1, 0), (1, 0), (0, -1), (0, 1))):
            fr = rec["fractions"][di]
            if fr is not None:
                hits.append([(j + dy * fr) * h, (k + dz * fr) * h, rec["electrodes"][di]])
    data = {
        "x_mm": XI * h, "h_mm": h, "ny": ny, "nz": nz,
        "electrode": node.tolist(),
        "potential_cut": pot(quad).tolist(),
        "potential_stair": pot(stair).tolist(),
        "cut_hits_yz_mm": hits,
        "sources": {
            "cuts": str(quad / "quadrupole.cuts.json"),
            "cut_bases": str(quad / "quadrupole.pa1..pa3"),
            "stair_bases": str(stair / "quadrupole.pa1..pa3"),
        },
    }
    OUT.write_text(json.dumps(data))
    print("wrote", OUT, "hits", len(hits))


if "--extract" in sys.argv:
    extract(Path(sys.argv[sys.argv.index("--extract") + 1]))

d = json.loads(OUT.read_text())
h = d["h_mm"]
E = np.array(d["electrode"])
ys = np.arange(d["ny"]) * h
zs = np.arange(d["nz"]) * h
hits = np.array(d["cut_hits_yz_mm"])

fig, axs = S.figure(8.4, ncols=3, gridspec_kw={"width_ratios": [1, 1, 0.05], "wspace": 0.12})
axes, cax = axs[:2], axs[2]
levels = np.linspace(-2.5, 2.5, 21)
for ax, key, title in zip(axes, ("potential_stair", "potential_cut"),
                          ("(a) 격자점 경계 (계단형, SIMION 방식)", "(b) cut-cell 경계 (정확한 표면)")):
    P = np.array(d[key])  # [k (z), j (y)]
    cf = ax.contourf(ys, zs, P, levels=levels, cmap="RdBu_r", extend="both")
    ax.contour(ys, zs, P, levels=levels[::2], colors="k", linewidths=0.35, alpha=0.55)
    if key == "potential_stair":
        for k in range(d["nz"] - 1):
            for j in range(d["ny"] - 1):
                if E[k, j] and E[k, j + 1] and E[k + 1, j] and E[k + 1, j + 1]:
                    ax.add_patch(Rectangle((j * h, k * h), h, h, facecolor="#9aa3ab", edgecolor="#6b747c", lw=0.3))
    else:
        for cy, cz in ((10.75, 0.0), (0.0, 10.75)):
            ax.add_patch(Circle((cy, cz), 5.75, facecolor="#9aa3ab", edgecolor="none", zorder=3))
        ax.plot(hits[:, 0], hits[:, 1], "o", ms=2.4, color=S.SIMION, mec="white", mew=0.3, zorder=5)
    for cy, cz in ((10.75, 0.0), (0.0, 10.75)):
        ax.add_patch(Circle((cy, cz), 5.75, fill=False, ls="--", lw=0.9, color=S.INK, zorder=4))
    ax.set_xlim(0, 18)
    ax.set_ylim(0, 18)
    ax.set_aspect("equal")
    ax.set_title(title, fontsize=S.FS_BASE)
    ax.set_xlabel("y (mm)")
axes[0].set_ylabel("z (mm)")
axes[1].tick_params(labelleft=False)
cb = fig.colorbar(cf, cax=cax)
cb.set_label("전위 (V)")
cb.set_ticks([-2.5, -1.25, 0, 1.25, 2.5])
handles = [
    Patch(facecolor="#9aa3ab", edgecolor="#6b747c", label="전극(왼쪽: 네 꼭짓점이 전극점인 셀)"),
    Line2D([], [], marker="o", ls="none", ms=3.5, color=S.SIMION, label="cut-cell 표면 교차점(cuts.json)"),
    Line2D([], [], ls="--", color=S.INK, lw=0.9, label="GEM 막대 표면 r = 5.75 mm"),
]
fig.legend(handles=handles, loc="lower center", ncol=3, bbox_to_anchor=(0.47, -0.05), fontsize=S.FS_SMALL, columnspacing=1.0, handlelength=1.4)
S.save(fig, "F04")
print("hits:", len(hits), "electrode nodes in slice:", int((E > 0).sum()))
