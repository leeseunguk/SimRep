"""F05: multigrid convergence, relative residual after k V-cycles (CPU f64).

Data: data/F05-residual-history.csv. Each row is a separate run of the current release CLI
    RAYON_NUM_THREADS=16 target/release/simrep refine <pa#> --device cpu --tolerance 1e-12 --max-cycles k
for k = 1, 2, ... until it converges; the CLI prints the f64 physical residual of the first basis
after exactly k cycles. Inputs: the quadrupole .pa# from `simrep refine-geometry` (121x37x37 =
165,649 nodes) and SIMION's _bench3d/sc3d_x2.pa# (259^3 = 17,373,979 nodes).
Run with --run to regenerate the CSV (needs the two .pa# files under target/report-figs/f05/).
"""
import csv
import re
import subprocess
import sys

import numpy as np

import _style as S

CSV = S.DATA_DIR / "F05-residual-history.csv"
if "--run" in sys.argv:
    rows = []
    for case, f, kmax in (("quadrupole", "quadrupole.pa#", 30), ("sc3d_x2", "sc3d_x2.pa#", 30)):
        for k in range(1, kmax + 1):
            out = subprocess.run(
                f"RAYON_NUM_THREADS=16 target/release/simrep refine target/report-figs/f05/{f} --device cpu --tolerance 1e-12 --max-cycles {k}",
                shell=True, capture_output=True, text=True, cwd=S.REPO)
            out = out.stdout + out.stderr
            m = re.search(r"Refining (\S+)\n\s+cycles: (\d+), residual f64: absolute = (\S+) V/mm\^2, relative = (\S+)", out)
            ok = "did not converge" not in out
            rows.append((case, m.group(1), m.group(2), m.group(3), m.group(4), ok))
            if ok:
                break
    with open(CSV, "w") as fh:
        fh.write("case,basis,cycles,absolute_residual_V_per_mm2,relative_residual,converged\n")
        for r in rows:
            fh.write(",".join(map(str, r)) + "\n")

rows = list(csv.DictReader(open(CSV)))
labels = {"quadrupole": ("3D 4중극 PA#, 165,649점", S.OURS, "o"),
          "sc3d_x2": ("3D 이중 구각 sc3d_x2, 17,373,979점", S.SIMION, "s")}
fig, ax = S.figure(7.0)
for case, (label, color, m) in labels.items():
    r = [x for x in rows if x["case"] == case]
    k = np.array([int(x["cycles"]) for x in r])
    rel = np.array([float(x["relative_residual"]) for x in r])
    rate = (rel[-1] / rel[0]) ** (1 / (k[-1] - k[0]))
    ax.semilogy(k, rel, marker=m, color=color, lw=1.4, ms=4, label=f"{label} — {k[-1]}회, 사이클당 감소율 {rate:.3f}")
ax.axhline(1e-12, color=S.INK, lw=0.8, ls="--")
ax.text(0.7, 1.6e-12, "수렴 기준: 상대 잔차 1e-12", fontsize=S.FS_SMALL, color=S.INK)
S.log_ticks(ax.yaxis)
ax.set_xlabel("V-cycle 수 (회)")
ax.set_ylabel("상대 잔차 (물리 f64 잔차의 ∞-노름, –)")
ax.set_xlim(0.5, 13.5)
ax.set_xticks(range(1, 14))
ax.grid(True, which="major")
ax.set_axisbelow(True)
ax.legend(loc="upper right", fontsize=S.FS_SMALL)
S.save(fig, "F05")
