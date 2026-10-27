#!/usr/bin/env python3
"""Compare SimRep run artifacts. Standard library only; exit 0/1/2 = same/different/error."""

import argparse
import csv
import io
import json
import math
from pathlib import Path
import re
import struct
import sys
from typing import Any


TIMING_KEYS = {
    "compute_ms", "prep_ms", "setup_ms", "total_ms", "elapsed_ms",
    "end_to_end_ms", "kernel_ms", "submit_readback_ms",
}
PA_SUFFIX = re.compile(r"\.pa(?:#|_|[0-9]+)?$", re.IGNORECASE)
CONTINUOUS_KEYS = {
    "fractions", "normals", "origin_mm", "spacing_mm", "bounds_mm",
    "reference_voltage", "electrode_voltages", "values", "surface_tolerance_mm",
    "snap_distance_mm", "chordal_tolerance_mm", "arrival_tof_us", "final_residual",
    "tolerance", "max_voltage", "potential_min", "potential_max", "potential_mean",
}


def finite(value):
    if not math.isfinite(value):
        raise ValueError("non-finite numeric value")
    return value


def ulp_distance(a, b):
    def ordered(value):
        bits = struct.unpack("<q", struct.pack("<d", value))[0]
        return -(bits + (1 << 63)) if bits < 0 else bits
    return abs(ordered(a) - ordered(b))


class Comparison:
    def __init__(self, options):
        self.options = options
        self.differences = 0
        self.details = []
        self.numeric_values = 0
        self.differing_values = 0

    def close(self, a, b, tolerance=None):
        if a == b:
            return True
        if tolerance is None:
            tolerance = self.options.tolerance
        return (
            abs(a - b) <= tolerance
            or abs(a - b) <= self.options.relative_tolerance * max(abs(a), abs(b))
            or (self.options.ulps > 0 and ulp_distance(a, b) <= self.options.ulps)
        )

    def difference(self, path, before, after):
        self.differences += 1
        if len(self.details) < self.options.max_diffs:
            self.details.append({"path": path, "before": before, "after": after})

    def compare(self, a, b, path="$", tolerance=None, continuous=False):
        if isinstance(a, dict) and isinstance(b, dict):
            for key in sorted(a.keys() | b.keys()):
                child = f"{path}.{key}"
                if key not in a or key not in b:
                    self.difference(child, a.get(key, "<missing>"), b.get(key, "<missing>"))
                else:
                    self.compare(a[key], b[key], child, tolerance, continuous or key in CONTINUOUS_KEYS)
        elif isinstance(a, list) and isinstance(b, list):
            if len(a) != len(b):
                self.difference(f"{path}.length", len(a), len(b))
            for i, (old, new) in enumerate(zip(a, b)):
                self.compare(old, new, f"{path}[{i}]", tolerance, continuous)
        elif type(a) is bool or type(b) is bool:
            if type(a) is not type(b) or a != b:
                self.difference(path, a, b)
        elif isinstance(a, (float, int)) and isinstance(b, (float, int)):
            finite(a)
            finite(b)
            self.numeric_values += 1
            self.differing_values += a != b
            # Integer counts, node indices and electrode numbers stay exact.
            if (not continuous and isinstance(a, int) and isinstance(b, int) and a != b) or not self.close(a, b, tolerance):
                self.difference(path, a, b)
        elif type(a) is not type(b) or a != b:
            self.difference(path, a, b)

    def result(self):
        return {
            "identical": self.differences == 0,
            "differences": self.differences,
            "numeric_values": self.numeric_values,
            "differing_values": self.differing_values,
            "details": self.details,
        }


def read_pa(data):
    if len(data) < 32:
        raise ValueError("truncated PA header")
    mode, symmetry, maximum, nx, ny, nz, flags = struct.unpack_from("<iidiiiI", data)
    if mode not in (-1, -2) or symmetry not in (0, 1):
        raise ValueError("unsupported PA mode or symmetry")
    if not (3 <= nx <= 90000 and 3 <= ny <= 90000 and 1 <= nz <= 90000):
        raise ValueError("invalid PA dimensions")
    if finite(maximum) <= 0 or not math.isfinite(2 * maximum):
        raise ValueError("invalid PA max_voltage")
    size = 56 if mode == -2 else 32
    if len(data) < size:
        raise ValueError("truncated extended PA header")
    spacing = struct.unpack_from("<ddd", data, 32) if mode == -2 else (1.0, 1.0, 1.0)
    if any(not 1e-6 <= finite(h) <= 900 for h in spacing):
        raise ValueError("invalid PA spacing")
    if (symmetry == 0 and (nz != 1 or not flags & 2)) or (nz == 1 and flags & 4):
        raise ValueError("invalid PA symmetry/mirroring")
    count = nx * ny * nz
    end = size + count * 8
    if end > len(data):
        raise ValueError("truncated PA potentials")
    header: dict[str, Any] = {
        "mode": mode, "symmetry": symmetry, "max_voltage": maximum,
        "dims": [nx, ny, nz], "mirror_flags": flags, "spacing_mm": list(spacing),
    }
    trailer = None
    if end != len(data):
        offset = end

        def take(fmt):
            nonlocal offset
            width = struct.calcsize(fmt)
            if offset + width > len(data):
                raise ValueError("truncated PA0 trailer")
            result = struct.unpack_from(fmt, data, offset)[0]
            offset += width
            return result

        adjustable = take("<i")
        if not 1 <= adjustable <= 1001:
            raise ValueError("invalid PA0 adjustable-electrode count")
        reference = finite(take("<d"))
        if reference <= 0:
            raise ValueError("invalid PA0 reference voltage")
        indices = []
        found = 0
        while len(indices) < 32 or found < adjustable:
            if len(indices) >= 1001:
                raise ValueError("invalid PA0 electrode table")
            index = take("<q" if count >= 1 << 31 else "<i")
            if not -1 <= index < count:
                raise ValueError("invalid PA0 electrode index")
            indices.append(index)
            found += index != -1
        if found != adjustable:
            raise ValueError("PA0 electrode count does not match table")
        scalable = None
        if offset != len(data):
            flag = take("<i")
            if flag not in (0, 1):
                raise ValueError("invalid PA0 scalable flag")
            scalable = [finite(take("<d")) for _ in indices] if flag else False
        if offset != len(data):
            raise ValueError("unexpected data after PA0 trailer")
        trailer = {"reference_voltage": reference, "electrode_indices": indices, "scalable": scalable}
    return header, memoryview(data)[size:end], trailer


def compare_pa(old, new, comparison):
    ah, av, at = read_pa(old)
    bh, bv, bt = read_pa(new)
    comparison.compare(ah, bh, "$.header")
    comparison.compare(at, bt, "$.pa0")
    changed = outside = masks = compared = 0
    maximum = rms = 0.0
    worst = None
    for i, ((raw_a,), (raw_b,)) in enumerate(zip(struct.iter_unpack("<d", av), struct.iter_unpack("<d", bv))):
        finite(raw_a)
        finite(raw_b)
        ea, eb = raw_a > ah["max_voltage"], raw_b > bh["max_voltage"]
        a = raw_a - 2 * ah["max_voltage"] if ea else raw_a
        b = raw_b - 2 * bh["max_voltage"] if eb else raw_b
        compared += 1
        if ea != eb:
            masks += 1
            comparison.difference(f"$.electrodes[{i}]", ea, eb)
        delta = abs(a - b)
        finite(delta)
        # Online RMS avoids squaring large finite voltages.
        rms = math.hypot(rms * math.sqrt((compared - 1) / compared), delta / math.sqrt(compared))
        if delta > maximum:
            maximum = delta
            nx, ny, _ = ah["dims"]
            grid = [i % nx, i // nx % ny, i // (nx * ny)]
            worst = {
                "index": i, "grid": grid,
                "position_from_origin_mm": [v * h for v, h in zip(grid, ah["spacing_mm"])],
                "before_v": a, "after_v": b,
            }
        changed += a != b
        if not comparison.close(a, b):
            outside += 1
            comparison.difference(f"$.potentials[{i}]", a, b)
    # Validate even unmatched tails when dimensions differ.
    for values in (av, bv):
        for (value,) in struct.iter_unpack("<d", values[compared * 8:]):
            finite(value)
    return {
        "nodes_compared": compared, "differing_nodes": changed,
        "outside_tolerance_nodes": outside, "electrode_mask_differences": masks,
        "max_abs_dv": maximum, "rms_dv": rms, "worst": worst,
    }


def load_json(data):
    def unique(pairs):
        result = {}
        for key, value in pairs:
            if key in result:
                raise ValueError(f"duplicate JSON key: {key}")
            result[key] = value
        return result

    def invalid_constant(value):
        raise ValueError(f"non-finite JSON number: {value}")

    result = json.loads(data, object_pairs_hook=unique, parse_constant=invalid_constant)

    def validate(value):
        if isinstance(value, float):
            finite(value)
        elif isinstance(value, dict):
            for child in value.values():
                validate(child)
        elif isinstance(value, list):
            for child in value:
                validate(child)
    validate(result)
    return result


def normalize_cuts(value):
    if not isinstance(value, dict) or type(value.get("version")) is not int or value["version"] not in (1, 2, 3):
        raise ValueError("unsupported cuts sidecar version")
    dims = value.get("dims", [])
    if len(dims) != 3 or any(type(n) is not int or n <= 0 for n in dims):
        raise ValueError("invalid cuts dimensions")
    count = math.prod(dims)
    for runs_key, dense_key in (
        ("node_electrode_runs", "node_electrodes"),
        ("solver_fixed_runs", "solver_fixed"),
    ):
        if runs_key in value:
            if dense_key in value:
                raise ValueError(f"both {runs_key} and {dense_key} present")
            expanded = []
            for run in value.pop(runs_key):
                if not isinstance(run, list) or len(run) != 3:
                    raise ValueError(f"invalid {runs_key} row")
                start, end, item = run
                if type(start) is not int or type(end) is not int or start != len(expanded) or not start < end <= count:
                    raise ValueError(f"invalid {runs_key} coverage")
                expanded.extend([item] * (end - start))
            value[dense_key] = expanded
        if dense_key in value and len(value[dense_key]) != count:
            raise ValueError(f"invalid {dense_key} length")
    if "node_electrodes" not in value:
        raise ValueError("missing node electrode data")
    if any(n is not None and (type(n) is not int or not 1 <= n <= 1000) for n in value["node_electrodes"]):
        raise ValueError("invalid node electrode number")
    if any(type(n) is not bool for n in value.get("solver_fixed", [])):
        raise ValueError("invalid solver fixed mask")
    links = {}
    dense_keys = {"cut_fractions": "fractions", "cut_electrodes": "electrodes", "cut_normals": "normals"}
    if "link_intersections" in value:
        if any(key in value for key in dense_keys):
            raise ValueError("both sparse and dense cuts present")
        for link in value.pop("link_intersections"):
            link = dict(link)
            node = link.pop("node", None)
            if type(node) is not int or not 0 <= node < count or str(node) in links:
                raise ValueError("invalid or duplicate cut node")
            links[str(node)] = link
    else:
        arrays = {}
        for key, canonical in dense_keys.items():
            if key in value:
                arrays[canonical] = value.pop(key)
                if len(arrays[canonical]) != count:
                    raise ValueError(f"invalid {key} length")
                if any(not isinstance(row, list) or len(row) != 6 for row in arrays[canonical]):
                    raise ValueError(f"{key} requires six directions per node")
        if not {"fractions", "electrodes"} <= arrays.keys():
            raise ValueError("missing cut fractions/electrodes")
        for node in range(count):
            row = {key: array[node] for key, array in arrays.items()}
            if any(item is not None for item in row["fractions"]):
                links[str(node)] = row
            elif any(item is not None for item in row["electrodes"]) or any(
                item is not None for item in row.get("normals", [])
            ):
                raise ValueError("cut electrodes/normals without fractions")
    for link in links.values():
        fractions, electrodes = link.get("fractions", []), link.get("electrodes", [])
        if len(fractions) != 6 or len(electrodes) != 6:
            raise ValueError("cut links require six directions")
        if not any(t is not None for t in fractions):
            raise ValueError("empty sparse cut link")
        for t, n in zip(fractions, electrodes):
            if (t is None) != (n is None):
                raise ValueError("cut fraction/electrode mismatch")
            if t is not None and (
                type(t) not in (int, float) or not 0 <= t <= 1
                or type(n) is not int or not 1 <= n <= 1000
            ):
                raise ValueError("invalid cut fraction/electrode")
        if "normals" in link:
            normals = link["normals"]
            if not isinstance(normals, list) or len(normals) != 6 or any(
                normal is not None and (
                    not isinstance(normal, list) or len(normal) != 3
                    or any(type(v) not in (int, float) for v in normal)
                    or fractions[direction] is None
                )
                for direction, normal in enumerate(normals)
            ):
                raise ValueError("invalid cut normals")
    value["link_intersections"] = links
    return value


def omit_keys(value, ignored):
    if isinstance(value, dict):
        return {key: omit_keys(child, ignored) for key, child in value.items() if key not in ignored}
    if isinstance(value, list):
        return [omit_keys(child, ignored) for child in value]
    return value


def read_csv(data):
    reader = csv.DictReader(io.StringIO(data.decode("utf-8-sig"), newline=""), strict=True)
    fields = reader.fieldnames
    required = {"ion", "tof_us", "x_mm", "y_mm", "z_mm", "ke_ev", "reason"}
    if not fields or len(set(fields)) != len(fields) or not required <= set(fields):
        raise ValueError("expected a flight CSV header (ion, tof_us, x_mm, y_mm, z_mm, ke_ev, reason)")
    rows = []
    for line, row in enumerate(reader, 2):
        if None in row or any(item is None for item in row.values()):
            raise ValueError(f"invalid CSV column count at line {line}")
        for key, item in row.items():
            if key == "ion" or (key == "electrode_number" and item):
                row[key] = int(item)
            elif key.endswith(("_us", "_mm", "_ev", "_mm_per_us")):
                row[key] = finite(float(item))
        rows.append(row)
    return fields, rows


def compare_csv(old, new, comparison):
    af, ar = read_csv(old)
    bf, br = read_csv(new)
    comparison.compare(sorted(af), sorted(bf), "$.columns")
    if len(ar) != len(br):
        comparison.difference("$.rows.length", len(ar), len(br))
    for i, (a, b) in enumerate(zip(ar, br)):
        for key in sorted(a.keys() | b.keys()):
            path = f"$.rows[{i}](ion={a['ion']}).{key}"
            if key not in a or key not in b:
                comparison.difference(path, a.get(key, "<missing>"), b.get(key, "<missing>"))
                continue
            tolerance = None
            if key == "tof_us":
                tolerance = comparison.options.tof_tolerance
            elif key in ("x_mm", "y_mm", "z_mm"):
                tolerance = comparison.options.position_tolerance
            elif key == "ke_ev":
                tolerance = comparison.options.energy_tolerance
            comparison.compare(a[key], b[key], path, tolerance)
    return {"rows_before": len(ar), "rows_after": len(br)}


def file_kind(path):
    name = path.name.lower()
    if PA_SUFFIX.search(name):
        return "pa"
    if name.endswith(".cuts.json"):
        return "cuts"
    if name.endswith(".adaptive.json"):
        return "adaptive"
    if name.endswith(".json"):
        return "json"
    if name.endswith(".csv"):
        return "flight_csv"
    return None


def compare_file(before, after, options) -> dict[str, Any]:
    kind = file_kind(before)
    if kind is None or kind != file_kind(after):
        raise ValueError(f"unsupported or mismatched file types: {before}, {after}")
    old, new = before.read_bytes(), after.read_bytes()
    comparison = Comparison(options)
    extra: dict[str, Any] = {}
    if kind == "pa":
        extra["potentials"] = compare_pa(old, new, comparison)
    elif kind == "flight_csv":
        extra["flight"] = compare_csv(old, new, comparison)
    else:
        a, b = load_json(old), load_json(new)
        if kind == "cuts":
            a, b = normalize_cuts(a), normalize_cuts(b)
        ignored = set(options.ignore_key)
        if kind == "json":
            ignored |= TIMING_KEYS
        comparison.compare(omit_keys(a, ignored), omit_keys(b, ignored))
    if options.require_byte_identity and old != new:
        comparison.difference("$.bytes", "before bytes", "different after bytes")
    return {
        "before": str(before), "after": str(after), "kind": kind,
        "byte_identical": old == new, **comparison.result(), **extra,
    }


def compare_paths(before, after, options) -> dict[str, Any]:
    if not before.exists() or not after.exists():
        raise ValueError("both input paths must exist")
    if before.is_dir() != after.is_dir():
        raise ValueError("compare two files or two directories")
    if before.is_dir():
        left = {str(p.relative_to(before)): p for p in before.rglob("*") if p.is_file() and file_kind(p)}
        right = {str(p.relative_to(after)): p for p in after.rglob("*") if p.is_file() and file_kind(p)}
        if not left and not right:
            raise ValueError("no PA, cuts, flight CSV, or JSON artifacts found")
        files = []
        for name in sorted(left.keys() | right.keys()):
            if name not in left or name not in right:
                files.append({
                    "path": name, "kind": file_kind(Path(name)), "identical": False,
                    "byte_identical": False, "differences": 1,
                    "details": [{"path": "$", "before": "present" if name in left else "missing",
                                 "after": "present" if name in right else "missing"}],
                })
            else:
                files.append({"path": name, **compare_file(left[name], right[name], options)})
    else:
        files = [compare_file(before, after, options)]
    return {
        "schema_version": 1,
        "identical": all(f["identical"] for f in files),
        "byte_identical": all(f["byte_identical"] for f in files),
        "files_compared": len(files), "files": files,
    }


def parser():
    result = argparse.ArgumentParser(description=__doc__)
    result.add_argument("before", nargs="?", type=Path)
    result.add_argument("after", nargs="?", type=Path)
    result.add_argument("--json", action="store_true", help="emit one machine-readable report")
    result.add_argument("--selftest", action="store_true", help="run synthetic format and CLI tests")
    result.add_argument("--tolerance", "--abs-tolerance", type=float, default=0.0,
                        help="absolute tolerance for floating values, in their stored units (default: 0)")
    result.add_argument("--relative-tolerance", type=float, default=0.0)
    result.add_argument("--ulps", type=int, default=0, help="maximum binary64 ULP distance (default: 0)")
    result.add_argument("--require-byte-identity", action="store_true",
                        help="fail on any byte difference, even within numeric tolerances")
    result.add_argument("--tof-tolerance", type=float, help="absolute flight TOF tolerance (us)")
    result.add_argument("--position-tolerance", type=float, help="absolute flight position tolerance (mm)")
    result.add_argument("--energy-tolerance", type=float, help="absolute flight energy tolerance (eV)")
    result.add_argument("--ignore-key", action="append", default=[],
                        help="ignore this JSON key at every depth; repeat as needed")
    result.add_argument("--max-diffs", type=int, default=8, help="maximum reported details per file")
    return result


def selftest():
    import contextlib
    import copy
    import tempfile
    import unittest

    def pa(values=None, mode=-1, trailer=b"", flags=1600):
        values = [0.0] * 9 if values is None else values
        header = struct.pack("<iidiiiI", mode, 1, 100000.0, 3, 3, 1, flags)
        if mode == -2:
            header += struct.pack("<ddd", 0.5, 0.5, 1.0)
        return header + struct.pack("<9d", *values) + trailer

    def cuts() -> dict[str, Any]:
        return {
            "version": 3, "dims": [3, 3, 1], "origin_mm": [0.0] * 3,
            "spacing_mm": [1.0] * 3, "symmetry": "planar", "electrode_numbers": [1],
            "node_electrodes": [None] * 8 + [1], "solver_fixed": [False] * 8 + [True],
            "cut_fractions": [[0.5, None, None, None, None, None]] + [[None] * 6 for _ in range(8)],
            "cut_electrodes": [[1, None, None, None, None, None]] + [[None] * 6 for _ in range(8)],
        }

    csv_header = "ion,tof_us,x_mm,y_mm,z_mm,ke_ev,reason,electrode_number\n"
    csv_row = "1,1.0,2.0,3.0,4.0,5.0,plane,\n"

    class IdentityTests(unittest.TestCase):
        def __init__(self, methodName="runTest"):
            super().__init__(methodName)
            self.temp = tempfile.TemporaryDirectory(prefix="simrep-identity-")
            self.addCleanup(self.temp.cleanup)
            self.root = Path(self.temp.name)
            self.options = parser().parse_args([])

        def compare(self, old, new, suffix=".json") -> dict[str, Any]:
            a, b = self.root / f"before{suffix}", self.root / f"after{suffix}"
            for path, value in ((a, old), (b, new)):
                if not isinstance(value, bytes):
                    value = json.dumps(value).encode()
                path.write_bytes(value)
            return compare_file(a, b, self.options)

        def test_pa_formats_and_trailer(self):
            table = [0] + [-1] * 31
            trailer = struct.pack("<id32ii", 1, 10000.0, *table, 1) + struct.pack("<32d", *([1.0] * 32))
            for mode in (-1, -2):
                for suffix in (".pa", ".pa#", ".pa0", ".pa1", ".pa_"):
                    data = pa(mode=mode, trailer=trailer if suffix == ".pa0" else b"")
                    result = self.compare(data, data, suffix)
                    self.assertTrue(result["identical"])
                    self.assertTrue(result["byte_identical"])
            changed = bytearray(trailer)
            struct.pack_into("<d", changed, 4, 10001.0)
            self.assertFalse(self.compare(pa(trailer=trailer), pa(trailer=bytes(changed)), ".pa0")["identical"])

        def test_pa_potential_metrics_and_tolerance(self):
            values = [0.0] * 9
            values[4] = 3.0
            result = self.compare(pa(), pa(values), ".pa")
            self.assertFalse(result["identical"])
            metrics = result["potentials"]
            self.assertEqual(metrics["differing_nodes"], 1)
            self.assertEqual(metrics["max_abs_dv"], 3.0)
            self.assertAlmostEqual(metrics["rms_dv"], 1.0)
            self.assertEqual(metrics["worst"]["grid"], [1, 1, 0])
            self.options.tolerance = 3.0
            self.assertTrue(self.compare(pa(), pa(values), ".pa")["identical"])

        def test_pa_mask_header_and_signed_zero(self):
            values = [0.0] * 9
            values[0] = 200000.0
            result = self.compare(pa(), pa(values), ".pa#")
            self.assertEqual(result["potentials"]["electrode_mask_differences"], 1)
            self.assertFalse(result["identical"])
            self.assertFalse(self.compare(pa(), pa(flags=1601), ".pa")["identical"])
            values[0] = -0.0
            result = self.compare(pa(), pa(values), ".pa")
            self.assertTrue(result["identical"])
            self.assertFalse(result["byte_identical"])
            self.options.require_byte_identity = True
            self.assertFalse(self.compare(pa(), pa(values), ".pa")["identical"])

        def test_ulp_and_relative_tolerance(self):
            a, b = [1.0] * 9, [math.nextafter(1.0, math.inf)] * 9
            self.assertFalse(self.compare(pa(a), pa(b), ".pa")["identical"])
            self.options.ulps = 1
            self.assertTrue(self.compare(pa(a), pa(b), ".pa")["identical"])
            self.assertEqual(ulp_distance(-1.0, math.nextafter(-1.0, -math.inf)), 1)
            self.assertEqual(ulp_distance(-0.0, 0.0), 0)
            self.options.ulps = 0
            self.options.relative_tolerance = 1e-12
            self.assertTrue(self.compare(pa(a), pa(b), ".pa")["identical"])

        def test_invalid_pa_even_if_byte_identical(self):
            for data in (b"broken", pa()[:-1], pa([math.nan] * 9), pa(trailer=b"x")):
                with self.assertRaises(ValueError):
                    self.compare(data, data, ".pa")

        def test_cuts_runs_sparse_and_key_order(self):
            dense = cuts()
            sparse = copy.deepcopy(dense)
            sparse["node_electrode_runs"] = [[0, 3, None], [3, 8, None], [8, 9, 1]]
            sparse["solver_fixed_runs"] = [[0, 8, False], [8, 9, True]]
            sparse["link_intersections"] = [{
                "node": 0, "fractions": sparse.pop("cut_fractions")[0],
                "electrodes": sparse.pop("cut_electrodes")[0],
            }]
            del sparse["node_electrodes"], sparse["solver_fixed"]
            sparse = dict(reversed(list(sparse.items())))
            self.assertTrue(self.compare(dense, sparse, ".cuts.json")["identical"])
            sparse["link_intersections"][0]["fractions"][0] += 1e-10
            self.assertFalse(self.compare(dense, sparse, ".cuts.json")["identical"])
            self.options.tolerance = 1e-9
            self.assertTrue(self.compare(dense, sparse, ".cuts.json")["identical"])

        def test_cuts_v2_normals_and_cut_nodes(self):
            a = cuts()
            a["version"] = 2
            a["cut_normals"] = [[[1.0, 0.0, 0.0]] + [None] * 5] + [[None] * 6 for _ in range(8)]
            b = copy.deepcopy(a)
            self.assertTrue(self.compare(a, b, ".cuts.json")["identical"])
            b["cut_normals"][0][0][1] = 1e-8
            self.assertFalse(self.compare(a, b, ".cuts.json")["identical"])
            self.options.tolerance = 1e-7
            self.assertTrue(self.compare(a, b, ".cuts.json")["identical"])
            b["node_electrodes"][0] = 1
            self.assertFalse(self.compare(a, b, ".cuts.json")["identical"])

        def test_invalid_cuts_runs_and_links(self):
            a = cuts()
            del a["node_electrodes"]
            a["node_electrode_runs"] = [[1, 9, None]]
            with self.assertRaises(ValueError):
                self.compare(a, a, ".cuts.json")
            a = cuts()
            a["cut_electrodes"][0][0] = None
            with self.assertRaises(ValueError):
                self.compare(a, a, ".cuts.json")
            a = cuts()
            a["cut_fractions"][1] = [None] * 5
            with self.assertRaises(ValueError):
                self.compare(a, a, ".cuts.json")

        def test_csv_values_reasons_rows_and_tolerances(self):
            a = (csv_header + csv_row).encode()
            self.assertTrue(self.compare(a, a, ".csv")["identical"])
            for old, new, option in (("1.0", "1.0001", "tof_tolerance"),
                                     ("2.0", "2.0001", "position_tolerance"),
                                     ("5.0", "5.0001", "energy_tolerance")):
                b = (csv_header + csv_row.replace(old, new)).encode()
                self.assertFalse(self.compare(a, b, ".csv")["identical"])
                setattr(self.options, option, 0.001)
                self.assertTrue(self.compare(a, b, ".csv")["identical"])
            b = (csv_header + csv_row.replace("plane", "electrode")).encode()
            self.assertFalse(self.compare(a, b, ".csv")["identical"])
            b = (csv_header + csv_row + csv_row.replace("1,", "2,", 1)).encode()
            self.assertFalse(self.compare(a, b, ".csv")["identical"])

        def test_invalid_csv(self):
            for data in (b"not,a,flight\n", (csv_header + "1,2\n").encode(),
                         (csv_header + csv_row.replace("1.0", "nan")).encode()):
                with self.assertRaises(ValueError):
                    self.compare(data, data, ".csv")

        def test_csv_row_order_and_exact_ion_ids(self):
            row2 = csv_row.replace("1,", "2,", 1)
            a = (csv_header + csv_row + row2).encode()
            b = (csv_header + row2 + csv_row).encode()
            self.options.tolerance = 100.0
            self.assertFalse(self.compare(a, b, ".csv")["identical"])

        def test_summary_timing_only_is_ignored(self):
            a: dict[str, Any] = {
                "compute_ms": 1.0, "nested": {"prep_ms": 2.0}, "ion_count": 1,
                "arrival_tof_us": {"mean": 3.0}, "output_paths": ["same.pa0"],
            }
            b = copy.deepcopy(a)
            b.update(compute_ms=90.0, nested={"prep_ms": 9.0})
            self.assertTrue(self.compare(a, b)["identical"])
            b["arrival_tof_us"]["mean"] = 4.0
            self.assertFalse(self.compare(a, b)["identical"])
            b["arrival_tof_us"]["mean"] = 3.0
            b["ion_count"] = 2
            self.options.tolerance = 100.0
            self.assertFalse(self.compare(a, b)["identical"])
            b["ion_count"] = 1
            b["output_paths"] = ["other.pa0"]
            self.assertFalse(self.compare(a, b)["identical"])
            self.options.ignore_key = ["output_paths"]
            self.assertTrue(self.compare(a, b)["identical"])

        def test_adaptive_manifest(self):
            a: dict[str, Any] = {
                "schema_version": 1, "base": "lens.pa0", "spacing_mm": [1.0] * 3,
                "patches": [{"level": 1, "file": "lens.patch0.pa0", "factor": 2,
                             "bounds_mm": [[0.0] * 3, [2.0] * 3]}],
            }
            b = copy.deepcopy(a)
            self.assertTrue(self.compare(a, b, ".adaptive.json")["identical"])
            b["patches"][0]["bounds_mm"][0][0] = 0.001
            self.assertFalse(self.compare(a, b, ".adaptive.json")["identical"])
            b = copy.deepcopy(a)
            b["patches"][0]["factor"] = 4
            self.options.tolerance = 10.0
            self.assertFalse(self.compare(a, b, ".adaptive.json")["identical"])

        def test_continuous_json_integers_use_tolerance(self):
            self.options.tolerance = 1.0
            a = {"origin_mm": [0, 0, 0], "dims": [3, 3, 1], "normals": [0, 1, 0]}
            b = {"origin_mm": [1, 0, 0], "dims": [3, 3, 1], "normals": [1, 0, 0]}
            self.assertTrue(self.compare(a, b)["identical"])
            b["dims"][0] = 4
            self.assertFalse(self.compare(a, b)["identical"])

        def test_directory_inventory_and_cli_exit_codes(self):
            a, b = self.root / "a", self.root / "b"
            a.mkdir()
            b.mkdir()
            (a / "lens.pa0").write_bytes(pa())
            (b / "lens.pa0").write_bytes(pa())
            output = io.StringIO()
            with contextlib.redirect_stdout(output):
                self.assertEqual(main([str(a), str(b), "--json"]), 0)
            self.assertTrue(json.loads(output.getvalue())["identical"])
            (b / "lens.pa1").write_bytes(pa())
            with contextlib.redirect_stdout(io.StringIO()):
                self.assertEqual(main([str(a), str(b)]), 1)
            (b / "lens.pa0").write_bytes(b"broken")
            output = io.StringIO()
            with contextlib.redirect_stdout(output):
                self.assertEqual(main([str(a), str(b), "--json"]), 2)
            self.assertIn("error", json.loads(output.getvalue()))

        def test_json_and_options_errors(self):
            for data in (b'{"x": NaN}', b'{"x": 1e999}', b'{"x":1,"x":2}'):
                with self.assertRaises(ValueError):
                    self.compare(data, data)
            with contextlib.redirect_stdout(io.StringIO()):
                self.assertEqual(main(["--tolerance", "-1", "--json"]), 2)

    suite = unittest.defaultTestLoader.loadTestsFromTestCase(IdentityTests)
    output = io.StringIO()
    result = unittest.TextTestRunner(stream=output, verbosity=2).run(suite)
    if not result.wasSuccessful():
        raise ValueError(output.getvalue())
    return {"schema_version": 1, "identical": True, "tests_run": result.testsRun}


def main(argv=None):
    arguments = parser()
    options = arguments.parse_args(argv)
    report: dict[str, Any]
    try:
        for key in ("tolerance", "relative_tolerance", "tof_tolerance", "position_tolerance", "energy_tolerance"):
            value = getattr(options, key)
            if value is not None and (not math.isfinite(value) or value < 0):
                raise ValueError(f"--{key.replace('_', '-')} must be finite and nonnegative")
        if options.ulps < 0 or options.max_diffs < 0:
            raise ValueError("--ulps and --max-diffs must be nonnegative")
        if options.selftest:
            if options.before or options.after:
                raise ValueError("--selftest does not take input paths")
            report = selftest()
        else:
            if options.before is None or options.after is None:
                raise ValueError("provide BEFORE and AFTER paths, or --selftest")
            report = compare_paths(options.before, options.after, options)
        if options.json:
            print(json.dumps(report, allow_nan=False))
        elif options.selftest:
            print(f"PASS: {report['tests_run']} synthetic identity tests")
        else:
            state = "IDENTICAL" if report["identical"] else "DIFFERS"
            print(f"{state}: {report['files_compared']} artifact(s); byte identity: {report['byte_identical']}")
            for file in report["files"]:
                label = file.get("path", file.get("before"))
                print(f"  {'same' if file['identical'] else 'DIFF'} {label}: {file['differences']} difference(s)")
                if "potentials" in file:
                    p = file["potentials"]
                    print(f"    dV max={p['max_abs_dv']:.9g}, RMS={p['rms_dv']:.9g} V; "
                          f"{p['differing_nodes']} changed node(s), {p['outside_tolerance_nodes']} outside tolerance")
                    if p["worst"]:
                        print(f"    worst: node {p['worst']['index']}, grid {p['worst']['grid']}")
                for detail in file["details"]:
                    old, new = repr(detail["before"]), repr(detail["after"])
                    print(f"    {detail['path']}: {old[:160]} -> {new[:160]}")
        return 0 if report["identical"] else 1
    except (OSError, ValueError, TypeError, KeyError, OverflowError, csv.Error, struct.error) as error:
        if options.json:
            print(json.dumps({"schema_version": 1, "identical": False,
                              "error": {"kind": type(error).__name__, "message": str(error)}}))
        else:
            print(f"ERROR: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main())
