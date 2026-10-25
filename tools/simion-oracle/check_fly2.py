#!/usr/bin/env python3
"""Compare installed FLY2 initial conditions, without flying any particles."""
import argparse
import collections
import json
import math
import re
import shutil
import subprocess
import sys
from pathlib import Path

from check_gem import invoke
from run_oracle import win_path


def rows(text):
    result = []
    for line in text.splitlines():
        if not re.match(r"^[+-]?(?:\d|\.\d)", line):
            continue
        try:
            row = [float(x) for x in line.split(",")]
        except ValueError:
            continue
        if len(row) != 11:
            continue
        az, el = map(math.radians, row[6:8])
        result.append(row[:6] + [math.cos(el)*math.cos(az), math.sin(el),
                                 -math.cos(el)*math.sin(az)] + row[8:])
    return result


def ks(a, b):
    a, b = sorted(a), sorted(b)
    i = j = 0
    d = 0.
    while i < len(a) or j < len(b):
        v = min(a[i] if i < len(a) else math.inf,
                b[j] if j < len(b) else math.inf)
        # Exported oracle values have 14 significant decimal digits.
        tolerance = 1e-10 * max(1., abs(v))
        while i < len(a) and a[i] <= v + tolerance:
            i += 1
        while j < len(b) and b[j] <= v + tolerance:
            j += 1
        d = max(d, abs(i/len(a)-j/len(b)))
    return d


def compare(a, b, random):
    if len(a) != len(b):
        return dict(status="mismatch", reason=f"particle count {len(a)} != {len(b)}")
    if not a:
        return dict(status="match", count=0)
    failures = [(i, k, x, y) for i, (ra, rb) in enumerate(zip(a, b))
                    for k, (x, y) in enumerate(zip(ra, rb))
                    if not math.isclose(x, y, rel_tol=2e-11,
                                        abs_tol=1e-24 if k == 10 else 1e-13)]
    if not random or not failures:
        return dict(status="mismatch" if failures else "match", count=len(a),
                    differences=len(failures), samples=failures[:8])
    # Family-wise alpha <= 0.01 for 154 files and 20 tested scalar projections.
    alpha = .01 / (154*20)
    bound = math.sqrt(-.5*math.log(alpha/2))*math.sqrt(2/len(a))
    projections = list(zip(*a)), list(zip(*b))
    # Also test transverse radii and direction/position correlation.
    for data, out in zip((a, b), projections):
        out.extend([
            [r[4]**2+r[5]**2 for r in data],
            [r[7]**2+r[8]**2 for r in data],
            [sum(r[3+k]*r[6+k] for k in range(3)) for r in data],
        ])
    metrics = []
    failures = []
    for k, (aa, bb) in enumerate(zip(*projections)):
        ma, mb = sum(aa)/len(aa), sum(bb)/len(bb)
        va = sum((v-ma)**2 for v in aa)/len(aa)
        vb = sum((v-mb)**2 for v in bb)/len(bb)
        d = ks(aa, bb)
        mean_limit = 6*math.sqrt((va+vb)/len(aa)) + 1e-9*max(1., abs(ma), abs(mb))
        # Compare second moments using their empirical fourth-moment uncertainty.
        sa, sb = sum(x*x for x in aa)/len(aa), sum(x*x for x in bb)/len(bb)
        variance_sa = sum((x*x-sa)**2 for x in aa)/len(aa)
        variance_sb = sum((x*x-sb)**2 for x in bb)/len(bb)
        second_limit = 6*math.sqrt((variance_sa+variance_sb)/len(aa)) + 1e-9*max(1., abs(sa), abs(sb))
        metrics.append(dict(column=k, ks=d, mean_native=ma, mean_simion=mb,
                            second_native=sa, second_simion=sb))
        if d > bound or abs(ma-mb) > mean_limit or abs(sa-sb) > second_limit:
            failures.append(k)
    return dict(status="mismatch" if failures else "statistical match", count=len(a),
                ks_bound=bound, failed_columns=failures, metrics=metrics)


def lua_string(value):
    return json.dumps(str(value).replace("\\", "/"))


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--examples", type=Path, default=Path("/mnt/c/Program Files/SIMION-2024/examples"))
    ap.add_argument("--root", type=Path, default=Path("/mnt/c/Users/lsu10/simrep-oracle/_fly2"))
    ap.add_argument("--native", type=Path, default=Path("target/release/examples/particles"))
    ap.add_argument("--simion", default="/mnt/c/Program Files/SIMION-2024/simion.exe")
    ap.add_argument("--filter", default="")
    ap.add_argument("--seeds", type=int, default=8)
    ap.add_argument("--reuse-oracle", action="store_true")
    ap.add_argument("--report", type=Path, default=Path("docs/fly2-validation.md"))
    args = ap.parse_args()
    args.native = args.native.resolve()
    args.root.mkdir(parents=True, exist_ok=True)
    results_path = args.root / "results.json"
    previous = {r["file"]: r for r in json.loads(results_path.read_text())} if results_path.exists() else {}
    records = []
    for source in sorted(args.examples.rglob("*.fly2")):
        name = str(source.relative_to(args.examples))
        if args.filter and not re.search(args.filter, name):
            if name in previous:
                records.append(previous[name])
            continue
        text = source.read_text()
        random = bool(re.search(r"\b(?:\w*distribution|rand|math\.random)\s*[({]", text))
        for imported in re.findall(r"(?:import|dofile)\s*['\"]([^'\"]+)", text):
            included=source.parent/imported
            if included.exists():
                random |= bool(re.search(r"\b(?:\w*distribution|rand|math\.random)\s*[({]",included.read_text()))
        seeds = range(1, args.seeds+1) if random else range(1, 2)
        native_rows = []
        oracle_rows = []
        record = dict(file=name, random=random, seeds=list(seeds))
        oracle_file = args.root / (name.replace("/", "__") + ".oracle.txt")
        context_file = args.root / (name.replace("/", "__") + ".context.json")
        trace_file = args.root / (name.replace("/", "__") + ".fields.csv")
        # PA coordinate modes need the actual IOB placement, not an assumed identity.
        iob = source.with_suffix(".iob")
        needs_context = bool(re.search(r"coordinates\s*=\s*[1-9]|simion\.wb", text))
        cached_iob = Path("/mnt/c/Users/lsu10/simrep-oracle/_iob/all") / name[:-5].replace("/", "__") / iob.name
        if needs_context and cached_iob.exists():
            iob = cached_iob
        if "simion.wb" in text and iob.exists():
            # Source generation needs the saved PA fields, not workbench
            # programs (which may fly, refine, or import missing sibling files).
            # Keep the original FLY2 and its imported libraries unchanged.
            work = args.root / "workbenches" / name[:-5].replace("/", "__")
            work.mkdir(parents=True, exist_ok=True)
            for dependency in iob.parent.iterdir():
                if dependency.is_file() and (dependency.suffix.lower() == ".iob"
                        or re.search(r"\.pa(?:[0-9_]+)?$", dependency.name, re.I)):
                    destination = work / dependency.name
                    if not destination.exists():
                        shutil.copyfile(dependency, destination)
            (work / (iob.stem + ".lua")).write_text("simion.workbench_program()\n")
            (work / (iob.stem + ".fly2")).write_text("particles {coordinates=0}\n")
            iob = work / iob.name
        script = ""
        if needs_context and iob.exists():
            script += f"simion.wb:load({lua_string(win_path(str(iob)))});"
            script += (
                "local f=assert(io.open(" + lua_string(win_path(str(context_file))) + ",'w'));"
                "f:write('[');"
                "local i=0;while true do i=i+1;local p=simion.wb.instances[i];if not p then break end;"
                "if i>1 then f:write(',') end;"
                "local x,y,z=p:pa_to_wb_coords(p.ox/p.pa.dx_mm,p.oy/p.pa.dy_mm,p.oz/p.pa.dz_mm);"
                "local x0,y0,z0=p:pa_to_wb_coords(0,0,0);"
                "local axes={};for a=1,3 do local q={0,0,0};q[a]=1;"
                "local xx,yy,zz=p:pa_to_wb_coords(unpack(q));"
                "local d=({p.pa.dx_mm,p.pa.dy_mm,p.pa.dz_mm})[a]*p.scale;"
                "axes[a]=string.format('[%.17g,%.17g,%.17g]',(xx-x0)/d,(yy-y0)/d,(zz-z0)/d);end;"
                "f:write(string.format('{\"origin_mm\":[%.17g,%.17g,%.17g],\"axes\":[%s],"
                "\"scale\":%.17g,\"spacing_mm\":[%.17g,%.17g,%.17g],"
                "\"dimensions\":[%d,%d,%d],\"working_origin_mm\":[%.17g,%.17g,%.17g],\"pa\":null}',"
                "x,y,z,table.concat(axes,','),p.scale,p.pa.dx_mm,p.pa.dy_mm,p.pa.dz_mm,"
                "p.pa.nx,p.pa.ny,p.pa.nz,p.ox,p.oy,p.oz));end;f:write(']');f:close();")
            script += f"assert(require('lfs').chdir({lua_string(win_path(str(source.parent)))}));"
            if "simion.wb" in text:
                # Replay the oracle's read-only field queries to isolate FLY2
                # semantics from the flight engine's different PA reconstruction.
                script += "local native_simion=simion;local trace=assert(io.open(" + lua_string(win_path(str(trace_file))) + ",'w'));"
                script += """
local instances={}
local i=0
while true do
  i=i+1
  local p=native_simion.wb.instances[i]
  if not p then break end
  local index=i
  local original=p.pa
  local function record(kind,x,y,z)
    local wx,wy,wz=p:pa_to_wb_coords(x,y,z)
    local v,ex,ey,ez,e=0,0,0,0,false
    local rx,ry,rz
    if kind=='potential_vc' then v=original:potential_vc(x,y,z);rx=v
    elseif kind=='electrode' then e=original:electrode(x,y,z);rx=e
    else
      rx,ry,rz=original:field_vc(x,y,z)
      local x0,y0,z0=p:pa_to_wb_coords(0,0,0)
      local local_e={rx/(p.scale*original.dx_mm),ry/(p.scale*original.dy_mm),rz/(p.scale*original.dz_mm)}
      for a=1,3 do
        local q={0,0,0};q[a]=1
        local ax,ay,az=p:pa_to_wb_coords(unpack(q))
        local d=p.scale*({original.dx_mm,original.dy_mm,original.dz_mm})[a]
        ex=ex+local_e[a]*(ax-x0)/d
        ey=ey+local_e[a]*(ay-y0)/d
        ez=ez+local_e[a]*(az-z0)/d
      end
    end
    trace:write(string.format('%d,%.17g,%.17g,%.17g,%.17g,%.17g,%.17g,%.17g,0,0,0,%d\\n',index,wx,wy,wz,v,ex,ey,ez,e and 1 or 0))
    return rx,ry,rz
  end
  local pa=setmetatable({}, {__index=function(_,k)
    if k=='potential_vc' or k=='field_vc' or k=='electrode' then
      return function(_,x,y,z) return record(k,x,y,z) end
    end
    return original[k]
  end})
  instances[i]=setmetatable({pa=pa},{__index=function(_,k)
    local v=p[k]
    if type(v)=='function' then return function(_,...) return v(p,...) end end
    return v
  end})
end
simion=setmetatable({wb={instances=instances}}, {__index=native_simion})
"""
        for seed in seeds:
            script += f"math.randomseed({seed});simion.seed({seed});"
            script += f"simion.iob2.load_fly2_file({lua_string(win_path(str(source)))}):create_ionfile(io.stdout);"
        if "simion.wb" in text and needs_context and iob.exists():
            script += "trace:close();"
        if (args.reuse_oracle and oracle_file.exists()
                and previous.get(name, {}).get("simion_exit")==0
                and previous.get(name, {}).get("seeds")==list(seeds)):
            oracle = dict(exit=0, stdout=oracle_file.read_text())
        else:
            oracle = invoke([args.simion, "--nogui", "--noprompt", "--lua", script], source.parent, 180)
            oracle_file.write_text(oracle["stdout"])
        record["simion_exit"] = oracle["exit"]
        if oracle["exit"] != 0:
            record.update(status="unsupported", reason="SIMION export: " + oracle["stdout"][-2000:])
        else:
            oracle_rows = rows(oracle["stdout"])
            context = str(context_file) if context_file.exists() else ""
            if needs_context and context:
                placements=json.loads(context_file.read_text())
                match=re.search(r"coordinates\s*=\s*(\d+)",text)
                mode=int(match[1]) if match else 2 if len(placements)>1 and placements[1]["dimensions"][0]>11 else 1
                if mode:
                    placement=placements[mode-1]
                    for row in oracle_rows:
                        p=row[3:6]
                        direction=row[6:9]
                        row[3:6]=[placement["origin_mm"][a]+placement["scale"]*sum(
                            placement["axes"][b][a]*p[b]*placement["spacing_mm"][b] for b in range(3)) for a in range(3)]
                        row[6:9]=[sum(placement["axes"][b][a]*direction[b] for b in range(3)) for a in range(3)]
            for seed in seeds:
                argv=[str(args.native), str(source), context, str(seed)]
                if trace_file.exists():
                    argv.append(str(trace_file))
                native = invoke(argv, args.root, 180)
                if native["exit"] != 0:
                    record.update(status="unsupported", reason=native["stdout"][-2000:])
                    break
                native_rows.extend(rows(native["stdout"]))
            else:
                record.update(compare(native_rows, oracle_rows, random))
        records.append(record)
        print(name, record["status"], record.get("reason", "")[:180], flush=True)
        results_path.write_text(json.dumps(records, indent=2))
    results_path.write_text(json.dumps(records, indent=2))
    counts = collections.Counter(r["status"] for r in records)
    lines = ["# FLY2 oracle validation", "", "Direct SIMION `create_ionfile` export; no particle flights.",
             "", "| Result | Files |", "| --- | ---: |"]
    lines += [f"| {status} | {counts[status]} |" for status in ("match", "statistical match", "mismatch", "unsupported")]
    lines += ["", f"Random definitions aggregate seeds 1 through {args.seeds}. SIMION seeds both",
              "`math.randomseed` and `simion.seed`; native uses explicit SplitMix64 seeds.",
              "Streams are intentionally not expected to match sample-for-sample.",
              "Each scalar physical column plus radial and position-direction projections",
              "is checked with two-sample KS and six-standard-error first/second moment",
              "bounds. KS uses a Bonferroni family-wise alpha of 0.01 over 154 x 20 tests.",
              "Deterministic rows are compared in order at 2e-11 relative / 1e-13 absolute",
              "tolerance (the oracle writes 14 significant digits). CWF uses 1e-24",
              "absolute tolerance so tiny emission currents cannot be mistaken for zero.",
              "PA-coordinate ION exports are normalized to physical WB units using the",
              "actual IOB placement. Field-emission sources replay the SIMION field-query",
              "trace, checking query order and coordinates, to test FLY2 independently",
              "of the flight engine's potential reconstruction. Their IOB working copy",
              "loads saved PA fields with its workbench program and automatic particle",
              "source disabled; the original",
              "FLY2 and cathodelib.lua are executed unchanged.",
              "", "| File | Result | Particles (all seeds) / reason |", "| --- | --- | --- |"]
    for r in records:
        detail = str(r.get("count", r.get("reason", r.get("samples", "")))).replace("|", "/").replace("\n", " ")
        lines.append(f"| `{r['file']}` | {r['status']} | {detail[:1000]} |")
    args.report.write_text("\n".join(lines)+"\n")
    print(json.dumps(counts), flush=True)
    return int(bool(counts["mismatch"]))


if __name__ == "__main__":
    sys.exit(main())
