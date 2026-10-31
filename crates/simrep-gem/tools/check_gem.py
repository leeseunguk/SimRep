#!/usr/bin/env python3
"""Extend the installed GEM oracle harness with fractional sidecar comparison."""
import os
import json
import re
import struct
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(REPO / "tools/simion-oracle"))
import check_gem as checker

BASELINE = {
    "geometry/8.1/cone.gem": 225,
    "geometry/8.1/detect.gem": 104,
    "geometry/8.1/disc_segmented.gem": 110,
    "geometry/8.1/duct_rectangular_tapered.gem": 27,
    "geometry/8.1/esa.gem": 61,
    "geometry/8.1/skimmer.gem": 713,
    "geometry/8.1/sphere_aniso.gem": 144,
    "geometry/8.1/trap4.gem": 112,
    "geometry/cone.gem": 161,
    "geometry/disc_segmented.gem": 110,
    "geometry/duct_rectangular_tapered.gem": 230,
    "geometry/esa.gem": 9298,
    "geometry/quad.gem": 50,
    "geometry/skimmer.gem": 458,
    "geometry/sphere_aniso.gem": 99,
    "geometry/trap1.gem": 116,
    "geometry/trap2.gem": 334,
    "geometry/trap3.gem": 370,
    "geometry/trap4.gem": 5228,
    "magnetic_sector/sector_2dc_conical_inv.gem": 34,
    "poisson/pierce_gun/pierce_2dcyl.gem": 4,
    "poisson/pierce_gun/pierce_2dcylbig.gem": 51,
    "poisson/pierce_gun/pierce_2dpla.gem": 4,
    "poisson/pierce_gun/pierce_3dpla.gem": 164,
    "repulsion/pierce2d.gem": 4,
}

def cause(name):
    stem = Path(name).stem
    if stem == "sphere_aniso":
        return "Rotated cell length: reciprocal forward-axis metric, not inverse RMS."
    if stem == "detect":
        return "Polygon vertex clipping at the rotated detector's conical exclusion."
    if stem == "esa":
        return ("Partial revolution axis exclusion." if "/8.1/" in name else
                "Nested explicit versus inherited inclusion modes; partial revolution axis.")
    if stem == "trap4" and "/8.1/" in name:
        return "Expanded box equality at the revolved hole's grid boundary."
    if stem in ("quad", "trap1", "trap2", "trap3", "trap4"):
        return ("Automatic hyperbola normal offset" +
                (" and inherited bare non-electrode cuts." if stem in ("trap2", "trap3", "trap4") else "."))
    if stem.startswith("pierce"):
        return "Concave polygon segment offsets and grid-boundary rounding."
    if stem == "duct_rectangular_tapered":
        return ("Polygon edge/vertex offsets." if "/8.1/" in name else
                "Polygon offsets and inherited inclusion through extrusions.")
    if stem == "skimmer":
        return ("Polygon edge and vertex offsets." if "/8.1/" in name else
                "Polygon offsets and automatic-mode vertex bounds.")
    if stem == "cone":
        return "Polygon segment offsets, vertex clipping, and swept bounds."
    return "Polygon segment offsets instead of an axis-aligned square expansion."


def surface(path):
    if not path.exists():
        return None
    data = path.read_bytes()
    assert data[:20] == b"SIMEXP\xff\x7f" + struct.pack("<I4sI", 4, b"FHDR", 1)
    offset = 20
    axes = []
    for tag in (b"XOFF", b"YOFF", b"ZOFF"):
        zero, marker, size = struct.unpack_from("<IIQ", data, offset)
        assert (zero, marker) == (0, 0xffffffff)
        assert data[offset + 16:offset + 20] == tag
        offset += 20
        axes.append(dict(struct.iter_unpack("<Qd", data[offset:offset + size])))
        offset += size
    assert data[offset:] == bytes(4)
    return axes


original_compare = checker.compare
original_invoke = checker.invoke


def invoke(argv, cwd, timeout):
    """Keep original SIMION failures and compare an equivalent PA declaration.

    SIMION 2024 rejects two shipped legacy declarations containing string-valued
    substitutions. Only the working-copy declaration is normalized; all Lua
    parameters and geometry remain the original source.
    """
    result = original_invoke(argv, cwd, timeout)
    if not str(argv[0]).endswith("simion.exe") or result["exit"] == 0:
        return result
    source = Path(cwd) / argv[4].replace("\\", "/").rsplit("/", 1)[-1]
    name = f"{source.parent.name}/{source.name}"
    replacements = {
        "einzel/einzel_param.gem": (
            r"pa_define\(.*?surface=fractional\)",
            "pa_define{nx=nx,ny=ny,nz=nz,symmetry=is3d and 'planar' or 'cylindrical',"
            "dx=d_mm,surface='fractional'}"),
        "geometry_optimization/mesh_size.gem": (
            r"pa_define\([^\n]+\)",
            "pa_define{nx=nx,ny=ny,nz=1,symmetry='cylindrical',"
            "dx=dx_mm,dy=dy_mm,surface=surface}"),
    }
    if name not in replacements:
        return result
    pattern, replacement = replacements[name]
    text, count = re.subn(pattern, replacement, source.read_text(), count=1, flags=re.S)
    if count != 1:
        raise ValueError(f"expected one oracle PA declaration in {source}")
    normalized = source.with_suffix(".oracle-compatible.gem")
    normalized.write_text(text)
    adjusted = list(argv)
    adjusted[4] = checker.win_path(str(normalized))
    compatible = original_invoke(adjusted, cwd, timeout)
    compatible["original_source_result"] = result
    compatible["source_adjustment"] = (
        "Equivalent table-form pa_define replaces SIMION's rejected legacy "
        "string substitution; geometry and parameter calculations are unchanged.")
    return compatible


def compare(ours, oracle):
    result = original_compare(ours, oracle)
    a = surface(ours.with_suffix(".pa-surf"))
    b = surface(oracle.with_suffix(".pa-surf"))
    if a is not None or b is not None:
        result["surface_equal"] = a == b
        if a != b:
            changes = []
            for axis in range(3):
                aa = a[axis] if a else {}
                bb = b[axis] if b else {}
                keys = sorted(aa.keys() | bb.keys())
                changes.extend((axis, k, aa.get(k), bb.get(k))
                               for k in keys if aa.get(k) != bb.get(k))
            result["surface_mismatches"] = len(changes)
            result["surface_samples"] = changes[:12]
            result["status"] = "mismatch"
            result["reason"] = f"{len(changes)} fractional surface records differ"
    return result


def write_report():
    root = Path(os.environ.get("SIMREP_ORACLE_DIR", REPO / "target/simrep-oracle")) / "_gem"
    results = json.loads((root / "results.json").read_text())
    report = REPO / "docs/gem-validation.md"
    lines = report.read_text().split("\n## Original mismatch regression ledger\n")[0].splitlines()
    lines[2] = lines[2].replace("tools/simion-oracle/check_gem.py",
                              "crates/simrep-gem/tools/check_gem.py")
    if "Native time is the complete point classification loop, excluding" in lines:
        start = lines.index("Native time is the complete point classification loop, excluding")
        end = lines.index("| GEM | Points | Native compute ms | SIMION process wall ms |")
        lines[start:end] = [
            "Native compute includes row-span preparation, classification, assignment,",
            "and fractional offsets when requested; parsing, allocation, and writing",
            "are excluded. The target comparison is native compute against SIMION's",
            "complete process wall time, not isolated SIMION compute.", ""]
    lines += ["", "## Original mismatch regression ledger", "",
              "| GEM | Before: differing grid points | After: grid / surface records | Root cause |",
              "| --- | ---: | --- | --- |"]
    by_name = {r["file"]: r for r in results}
    for name, before in BASELINE.items():
        r = by_name[name]
        lines.append(f"| `{name}` | {before} | "
                     f"{r.get('mismatches', r['status'])} / "
                     f"{r.get('surface_mismatches', 0)} | {cause(name)} |")
    lines += ["", "## Fractional surface validation", "",
              "| GEM | Grid mismatches | Surface record mismatches | Samples |",
              "| --- | ---: | ---: | --- |"]
    for r in results:
        if "surface_equal" in r:
            lines.append(f"| `{r['file']}` | {r.get('mismatches', 0)} | "
                         f"{r.get('surface_mismatches', 0)} | "
                         f"`{json.dumps(r.get('surface_samples', []))}` |")
    lines += ["", "## SIMION source normalization", "",
              "The installed SIMION 2024 compiler rejects two shipped legacy PA",
              "declarations containing string substitutions. Their original exit",
              "codes and diagnostics remain in `simion.original_source_result`",
              "in results.json. The oracle rerun changes only the working copy's",
              "`pa_define` to the equivalent table form; native conversion uses",
              "the unmodified installed source. These are normalized-oracle passes,",
              "not unmodified-source SIMION passes.", "",
              "| GEM | Original SIMION exit | Native / normalized oracle | Adjustment |",
              "| --- | ---: | --- | --- |"]
    for r in results:
        if "source_adjustment" in r["simion"]:
            lines.append(f"| `{r['file']}` | "
                         f"{r['simion']['original_source_result']['exit']} | "
                         f"{r['status']} / {r.get('mismatches', '')} | "
                         f"{r['simion']['source_adjustment']} |")
    normalized = sum("source_adjustment" in r["simion"] for r in results)
    passed = sum(r["status"] == "pass" for r in results)
    fragments = sum(r["status"] == "include fragment" for r in results)
    unsupported = sum(r["status"] == "unsupported" for r in results)
    lines += ["", "## Lua compatibility expansion", "",
              "The previous baseline was 211 passes and 50 unsupported files:",
              "43 executable Lua sources and 7 include-only fragments.",
              f"Current result: {passed} standalone matches (including {normalized}",
              f"normalized-oracle matches), {fragments} include fragments, and",
              f"{unsupported} unsupported sources.", "",
              "Compatibility fixes include Lua 5.1 statement/expression expansion,",
              "lexical macro functions, loops, recursive tables, method calls,",
              "shape and potential callbacks, full-precision numeric substitutions,",
              "nearest-grid point extents, inclusive expanded sphere boundaries",
              "and tangent-row roundoff, electric ng defaults, and omission of",
              "empty fractional companions.", "",
              "Fragments compile through their existing parents. SIMION gem2pa",
              "also emits no standalone PA for these seven files (exit 0).", "",
              "| GEM | Standalone classification |",
              "| --- | --- |"]
    for r in results:
        if r["status"] not in ("unsupported", "include fragment"):
            continue
        exact = r["reason"]
        lines.append(f"| `{r['file']}` | `{exact.replace('|', '/')}` |")
    report.write_text("\n".join(lines) + "\n")


if __name__ == "__main__":
    checker.compare = compare
    checker.invoke = invoke
    if "--report-only" in sys.argv:
        sys.argv.remove("--report-only")
    else:
        checker.main()
    write_report()
