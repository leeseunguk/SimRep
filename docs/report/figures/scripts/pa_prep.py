"""Working-copy preparation used before some CLI refines (never applied to the original example files).

  pa_prep.py refinable FILE            clear the PA 'non-refinable' header bit (mirror word bit 28),
                                       as docs/magnetostatics.md does for its writable test copies
  pa_prep.py vmax FILE VALUE           raise the header max_voltage and re-encode electrode nodes
                                       (needed when the solved potential exceeds the stored range)
  pa_prep.py relabel IN OUT FROM:TO... replace non-electrode label values, e.g. 5:10 (dielectric
                                       epsilon set by the example Lua) or 1:1 *:1000 (C-magnet mu)
  pa_prep.py scale IN OUT FACTOR       multiply all non-electrode values (C-magnet J = I/area)
"""
import struct
import sys

import numpy as np


def read(path):
    b = bytearray(open(path, "rb").read())
    mode, _s, vmax, nx, ny, nz, _m = struct.unpack_from("<iidiiii", b, 0)
    off = 32 + (24 if mode == -2 else 0)
    v = np.frombuffer(bytes(b), dtype="<f8", count=nx * ny * nz, offset=off).copy()
    return b, off, vmax, v


def write(b, off, v, path):
    b[off:off + v.size * 8] = v.astype("<f8").tobytes()
    open(path, "wb").write(b)


cmd = sys.argv[1]
if cmd == "refinable":
    b = bytearray(open(sys.argv[2], "rb").read())
    m = struct.unpack_from("<i", b, 28)[0]
    struct.pack_into("<i", b, 28, m & ~(1 << 28))
    open(sys.argv[2], "wb").write(b)
elif cmd == "vmax":
    b, off, vmax, v = read(sys.argv[2])
    new = float(sys.argv[3])
    el = v > vmax
    v = np.where(el, v - 2 * vmax + 2 * new, v)
    struct.pack_into("<d", b, 8, new)
    write(b, off, v, sys.argv[2])
elif cmd == "relabel":
    b, off, vmax, v = read(sys.argv[2])
    assert not (v > vmax).any(), "cell arrays must not contain electrode nodes"
    out = v.copy()
    rules = [r.split(":") for r in sys.argv[4:]]
    default = next((float(t) for f, t in rules if f == "*"), None)
    if default is not None:
        out[:] = default
    for f, t in rules:
        if f != "*":
            out[v == float(f)] = float(t)
    write(b, off, out, sys.argv[3])
elif cmd == "scale":
    b, off, vmax, v = read(sys.argv[2])
    assert not (v > vmax).any()
    write(b, off, v * float(sys.argv[4]), sys.argv[3])
else:
    raise SystemExit(__doc__)
