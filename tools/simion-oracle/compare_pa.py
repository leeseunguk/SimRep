#!/usr/bin/env python3
"""Inspect SIMION PA files or compare decoded potentials (including electrodes).

Uses only the Python standard library. PA0 electrode voltages may have been
fast-adjusted after refinement; compare equal voltage settings, not raw PA0s.
"""
import argparse
import array
import json
import math
import struct
import sys


def read_pa(path):
    with open(path, "rb") as stream:
        mode, symmetry, maximum, nx, ny, nz, mirror = struct.unpack(
            "<iidiiii", stream.read(32))
        if mode not in (-1, -2):
            raise ValueError("unsupported PA mode")
        spacing = struct.unpack("<ddd", stream.read(24)) if mode == -2 else (1, 1, 1)
        values = array.array("d")
        values.fromfile(stream, nx * ny * nz)
        if sys.byteorder != "little":
            values.byteswap()
        trailer = stream.read()
    return {
        "path": path, "dims": [nx, ny, nz], "symmetry": symmetry,
        "mirror": mirror, "spacing": spacing, "max_voltage": maximum,
        "values": values, "trailer": trailer,
    }


def summary(pa):
    maximum = pa["max_voltage"]
    electrodes = {}
    low, high = math.inf, -math.inf
    for raw in pa["values"]:
        value = raw - 2 * maximum if raw > maximum else raw
        low, high = min(low, value), max(high, value)
        if raw > maximum:
            electrodes[value] = electrodes.get(value, 0) + 1
    info = {k: v for k, v in pa.items() if k not in ("values", "trailer")}
    info.update(min=low, max=high, electrode_values=electrodes)
    trailer = pa["trailer"]
    if trailer:
        count, reference = struct.unpack_from("<id", trailer)
        offsets, found, position = [], 0, 12
        code = "<q" if len(pa["values"]) >= 2**31 else "<i"
        while len(offsets) < 32 or found < count:
            value, = struct.unpack_from(code, trailer, position)
            position += struct.calcsize(code)
            offsets.append(value)
            found += value != -1
        info["pa0"] = {
            "reference": reference,
            "offsets": {i: v for i, v in enumerate(offsets) if v != -1},
            "extra_trailer_bytes": len(trailer) - position,
        }
    return info


def compare(left, right):
    for key in ("dims", "symmetry", "mirror", "spacing"):
        if left[key] != right[key]:
            raise ValueError(f"incompatible {key}: {left[key]} != {right[key]}")
    maximum, square, mask_errors = 0.0, 0.0, 0
    for a, b in zip(left["values"], right["values"]):
        ea, eb = a > left["max_voltage"], b > right["max_voltage"]
        mask_errors += ea != eb
        a -= 2 * left["max_voltage"] if ea else 0
        b -= 2 * right["max_voltage"] if eb else 0
        difference = abs(a - b)
        maximum = max(maximum, difference)
        square += difference * difference
    return {
        "left": left["path"], "right": right["path"],
        "nodes": len(left["values"]), "max_difference_v": maximum,
        "rms_difference_v": math.sqrt(square / len(left["values"])),
        "electrode_mask_mismatches": mask_errors,
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("left")
    parser.add_argument("right", nargs="?")
    args = parser.parse_args()
    left = read_pa(args.left)
    result = compare(left, read_pa(args.right)) if args.right else summary(left)
    print(json.dumps(result, indent=2))


if __name__ == "__main__":
    main()
