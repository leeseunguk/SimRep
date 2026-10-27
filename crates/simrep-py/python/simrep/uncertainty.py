"""Grid uncertainty and ASME V&V 20 arithmetic; sizes are finest first.

The grid functions call the Rust implementation, so Python and Rust use the
same fits. GCI is relative, ``u_num`` is absolute. Uncertainty estimates must
use a consistent coverage convention before combining them.
"""
import json
import math

from . import _native


def observed_order(h, values):
    """Celik (2008) order, Richardson limit, and relative GCI on three grids.

    Oscillatory or diverging triplets return None for p, extrapolated and GCI.
    """
    return json.loads(_native.grid_uncertainty(_levels(h, values), False))


def grid_study(h, values):
    """Eca-Hoekstra (2014) Appendix A fits, including noisy grid sequences."""
    return json.loads(_native.grid_uncertainty(_levels(h, values), True))


def validation(simulation, datum, u_num, u_input=0.0, u_datum=0.0):
    """Return E=S-D and u_val=RSS(u_num,u_input,u_datum), in datum units."""
    if not all(math.isfinite(x) for x in
               (simulation, datum, u_num, u_input, u_datum)):
        raise ValueError("validation inputs must be finite")
    if min(u_num, u_input, u_datum) < 0:
        raise ValueError("uncertainties must be nonnegative")
    return {"E": simulation - datum,
            "u_val": math.hypot(u_num, u_input, u_datum)}


def _levels(h, values):
    h, values = list(h), list(values)
    if len(h) != len(values):
        raise ValueError("sizes and values must have equal lengths")
    return list(zip(h, values))
