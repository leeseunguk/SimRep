"""Shared plot style for the SIMION_Rep report figures.

Every figure is drawn at its final printed width (15.5 cm on A4) so that
font sizes in points are the sizes the reader sees. Korean text uses
Malgun Gothic from the Windows font folder.

Run any figure script with the venv that has matplotlib/numpy/scipy, e.g.
    uv run --with matplotlib --with numpy --with scipy python3 F02_speed.py
"""
from __future__ import annotations

import os
from pathlib import Path

import matplotlib

matplotlib.use("Agg")
import matplotlib.pyplot as plt  # noqa: E402
from matplotlib import font_manager  # noqa: E402

HERE = Path(__file__).resolve().parent
FIG_DIR = HERE.parent
DATA_DIR = FIG_DIR / "data"
REPO = FIG_DIR.parents[2]

FONT_PATHS = [
    "/mnt/c/Windows/Fonts/malgun.ttf",
    "/mnt/c/Windows/Fonts/malgunbd.ttf",
]

# Final printed width in the report and the derived figure width in inches.
WIDTH_CM = 15.5
WIDTH_IN = WIDTH_CM / 2.54
DPI = 300

# Design tokens: one ink ramp plus two series colours that stay distinct in
# greyscale print. "ours" is SimRep, "simion" is the SIMION 2024 oracle.
INK = "#1f2933"
INK_MUTED = "#5b6773"
RULE = "#c9d1d9"
GRID = "#e6eaee"
OURS = "#1d5fbf"
OURS_LIGHT = "#8db3e8"
SIMION = "#c4561d"
SIMION_LIGHT = "#efb08c"
REF = "#2f855a"
ACCENTS = ["#1d5fbf", "#c4561d", "#2f855a", "#7b4fb3", "#a1741f", "#3b8b9c"]

FS_BASE = 8.5
FS_SMALL = 7.5
FS_TITLE = 9.5


def setup() -> None:
    for p in FONT_PATHS:
        if os.path.exists(p):
            font_manager.fontManager.addfont(p)
    plt.rcParams.update(
        {
            "font.family": ["Malgun Gothic", "DejaVu Sans"],
            "font.size": FS_BASE,
            "axes.titlesize": FS_TITLE,
            "axes.labelsize": FS_BASE,
            "xtick.labelsize": FS_SMALL,
            "ytick.labelsize": FS_SMALL,
            "legend.fontsize": FS_SMALL,
            "axes.unicode_minus": False,
            "axes.edgecolor": INK_MUTED,
            "axes.labelcolor": INK,
            "axes.linewidth": 0.6,
            "xtick.color": INK_MUTED,
            "ytick.color": INK_MUTED,
            "xtick.major.width": 0.6,
            "ytick.major.width": 0.6,
            "text.color": INK,
            "axes.spines.top": False,
            "axes.spines.right": False,
            "axes.grid": False,
            "grid.color": GRID,
            "grid.linewidth": 0.6,
            "legend.frameon": False,
            "savefig.dpi": DPI,
            "savefig.bbox": "tight",
            "savefig.pad_inches": 0.04,
            "figure.dpi": 100,
            "mathtext.fontset": "dejavusans",
            "mathtext.default": "regular",
        }
    )


SUP = str.maketrans("-0123456789", "\u207b\u2070\u00b9\u00b2\u00b3\u2074\u2075\u2076\u2077\u2078\u2079")


def log_ticks(axis) -> None:
    """Label decades as plain text 10^n (no mathtext, so no missing minus glyph)."""
    import math

    from matplotlib.ticker import FuncFormatter, LogLocator, NullFormatter

    def fmt(v, _):
        if v <= 0:
            return ""
        e = math.log10(v)
        if abs(e - round(e)) > 1e-9:
            return ""
        e = int(round(e))
        if -2 <= e <= 3:
            return f"{v:g}"
        return "10" + str(e).translate(SUP)

    axis.set_major_locator(LogLocator(base=10, numticks=30))
    axis.set_major_formatter(FuncFormatter(fmt))
    axis.set_minor_formatter(NullFormatter())


def figure(height_cm: float, ncols: int = 1, nrows: int = 1, **kw):
    setup()
    return plt.subplots(nrows, ncols, figsize=(WIDTH_IN, height_cm / 2.54), **kw)


def save(fig, fig_id: str) -> Path:
    out = FIG_DIR / f"{fig_id}.png"
    fig.savefig(out, dpi=DPI)
    plt.close(fig)
    print("wrote", out)
    return out
