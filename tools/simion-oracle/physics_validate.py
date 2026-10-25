#!/usr/bin/env python3
"""Fresh, instrumented SIMION magnetic/HS1/SDS validation.

Run from WSL, after building release simrep and physics_probe. All SIMION
processes are foreground, timeout-bounded, with a /mnt/c cwd and closed stdin.
Original example files are never edited. mbmr.dat stays outside the repository.
"""
import argparse
import csv
import json
import math
import os
import re
import shutil
import struct
import subprocess
from pathlib import Path

from scipy.stats import ks_2samp

REPO = Path(__file__).resolve().parents[2]
# Collected SIMION oracle inputs; required via SIMREP_ORACLE_CACHE.
# Resolved in main() so --help works without it.
ORACLE = None


def oracle_cache():
    root = os.environ.get("SIMREP_ORACLE_CACHE")
    if root:
        return Path(root)
    raise SystemExit(
        "SIMREP_ORACLE_CACHE is not set. Set it to the directory holding the "
        "collected SIMION oracle inputs: contour/contourlib81.lua, magnet/, "
        "magnetic_sector/, helmholtz_coil/, solenoid/, collision_hs1/, and "
        "collision_sds/mbmr.dat. Results are written to --root (default "
        "$SIMREP_ORACLE_CACHE/physics-validation-20261023).")
SIMION = "/mnt/c/Program Files/SIMION-2024/simion.exe"


def win(path):
    return subprocess.check_output(["wslpath", "-w", str(path)], text=True).strip()


def call(args, cwd, timeout=45):
    p = subprocess.run(args, cwd=cwd, stdin=subprocess.DEVNULL,
                       stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                       text=True, timeout=timeout)
    if p.returncode:
        raise RuntimeError(f"{args}\n{p.stdout}\n{p.stderr}")
    return p.stdout, p.stderr


def simion(args, cwd, log, threads=None):
    base = [SIMION, "--nogui", "--noprompt", "--default-num-particles", "100000"]
    if threads is not None:
        base += ["--num-threads", str(threads)]
    out, err = call(base + args, cwd)
    log.write_text(out + err)
    return out


def records(text, tag):
    return [line.strip().split(",")[1:] for line in text.splitlines()
            if line.startswith(tag + ",")]

def check_replay(recording, timing):
    def splats(text):
        return [line for line in text.splitlines() if re.match(r"status2,\s*\d+ Splat:", line)]
    a, b = splats(recording), splats(timing)
    if not a or a != b:
        raise RuntimeError("seeded SIMION recording/timing runs did not replay identical splat records")
    return len(a)


def copy_case(name, folder, destination):
    dest = destination / name
    dest.mkdir(parents=True, exist_ok=True)
    for path in folder.iterdir():
        if path.is_file() and (path.suffix.lower() in (".iob", ".fly", ".fly2", ".rec", ".txt")
                              or ".pa" in path.name):
            shutil.copyfile(path, dest / path.name)
    return dest


def header(path):
    raw = path.read_bytes()[:56]
    mode, symmetry, _, nx, ny, nz, mirror = struct.unpack("<iidiiii", raw[:32])
    return dict(magnetic=bool(mirror & 8), ng=(mirror >> 4) & 131071,
                dimensions=[nx, ny, nz], cylindrical=symmetry == 0, mode=mode)


def magnetic(root, selected=None, stage="all"):
    report = {}
    if stage not in ("native", "flight"):
        (root / "contour").mkdir(exist_ok=True)
        shutil.copyfile(ORACLE / "contour/contourlib81.lua", root / "contour/contourlib81.lua")
    for folder, stem in [("magnet", "magnet1"), ("magnet", "mag90"),
                         ("magnetic_sector", "sector_2dp"),
                         ("magnetic_sector", "sector_2dc"),
                         ("magnetic_sector", "sector_3dp"),
                         ("helmholtz_coil", "helmholtz"), ("solenoid", "solenoid")]:
        if selected is not None and stem != selected:
            continue
        dest = root / stem if stage in ("native", "flight") else copy_case(stem, ORACLE / folder, root)
        # Preserve the shipped model code through an absolute import.
        original = ORACLE / folder / (stem + ".lua")
        # Wrap rather than replace the model's initialize/terminate hooks.
        program = ("simion.workbench_program()\nsim_segment_global=1\n"
                   + (f"simion.import [[{win(original)}]]\n" if original.exists() else "")
                   + """
local initialize, terminate = segment.initialize, segment.terminate
local written = {}
function segment.initialize()
  if initialize then initialize() end
  print(string.format('START,%d,%.17g,%.17g,%.17g,%.17g,%.17g,%.17g,%.17g,%.17g,%.17g,%.17g',
    ion_number,ion_time_of_birth,ion_mass,ion_charge,ion_px_mm,ion_py_mm,ion_pz_mm,
    ion_vx_mm,ion_vy_mm,ion_vz_mm,ion_ke))
end
local function record_final()
  if written[ion_number] then return end
  written[ion_number] = true
  print(string.format('FINAL,%d,%.17g,%.17g,%.17g,%.17g,%.17g',
    ion_number,ion_time_of_flight,ion_px_mm,ion_py_mm,ion_pz_mm,ion_ke))
end
function segment.terminate()
  if terminate then terminate() end
  record_final()
end
""")
        if stem == "helmholtz":
            program += """
local tstep, actions = segment.tstep_adjust, segment.other_actions
function segment.tstep_adjust()
  if tstep then tstep() end
  ion_time_step=math.min(ion_time_step,0.03-ion_time_of_flight)
end
function segment.other_actions()
  if actions then actions() end
  if ion_time_of_flight >= 0.03-1e-12 then record_final(); ion_splat=1 end
end
"""
        (dest / (stem + ".lua")).write_text(program)
        if stage in ("native", "flight"):
            meta = (dest / "metadata.log").read_text()
            out = (dest / "flight.log").read_text()
        else:
            meta = simion(["lua", win(REPO / "tools/simion-oracle/physics_readout.lua"),
                           stem + ".iob"], dest, dest / "metadata.log")
            out = simion(["fly", "--programs", "1", "--omit-relativistic-effects", "1",
                          "--retain-trajectories", "0", stem + ".iob"], dest, dest / "flight.log")
        if stage == "prepare":
            print("Prepared foreground SIMION metadata and flights:", stem, flush=True)
            continue
        instances = {}
        for row in records(meta, "INSTANCE"):
            n = int(row[0])
            values = list(map(float, row[2:]))
            path = dest / f"instance{n}.pa"
            instances[n] = dict(path=str(path), origin_mm=values[:3],
                                axes=[values[3 + i * 3:6 + i * 3] for i in range(3)],
                                scale=values[12], **header(path))
        lines, expected, labels = [], [], []
        for row in records(meta, "FIELD"):
            n = int(row[0])
            inst = instances[n]
            if not inst["magnetic"]:
                continue
            values = [inst["ng"]] + inst["origin_mm"] + sum(inst["axes"], []) + [inst["scale"]]
            lines.append(",".join(["PA", inst["path"]] + list(map(str, values + list(map(float, row[1:4]))))))
            expected.append(list(map(float, row[4:7])))
            labels.append("PA")
        if stem == "magnet1":
            for row in records(meta, "ANALYTIC"):
                lines.append(",".join([row[0], row[0]] + row[1:4]))
                expected.append(list(map(float, row[4:7])))
                labels.append(row[0])
        if stage == "flight":
            field_report = json.loads((dest / "comparison.json").read_text())["field"]
        else:
            p = subprocess.run([str(REPO / "target/release/examples/physics_probe")],
                               input="\n".join(lines) + "\n", text=True, capture_output=True, timeout=600)
            if p.returncode:
                raise RuntimeError(p.stdout + p.stderr)
            errors = {}
            for line, reference, label in zip(p.stdout.splitlines(), expected, labels):
                actual = list(map(float, line.split(",")[2:]))
                error = math.dist(actual, reference)
                errors.setdefault(label, []).append((error, error / max(math.hypot(*reference), 1e-15)))
            field_report = {label: dict(points=len(values),
                                       max_absolute_gauss=max(v[0] for v in values),
                                       max_relative=max(v[1] for v in values))
                            for label, values in errors.items()}
        # Full original example flights, with high precision START/FINAL records.
        starts = records(out, "START")
        finals = records(out, "FINAL")
        ions = []
        for row in starts:
            v = list(map(float, row[7:10]))
            speed = math.hypot(*v)
            az = math.degrees(math.atan2(-v[2], v[0]))
            el = math.degrees(math.asin(v[1] / speed)) if speed else 0.0
            ions.append(",".join(row[1:7] + [str(az), str(el), row[10], "1", "1"]))
        (dest / "ions.ion").write_text("\n".join(ions) + "\n")
        bounds = list(map(float, records(meta, "BOUNDS")[0]))
        pas = []
        for inst in instances.values():
            if inst["magnetic"] and stem not in ("helmholtz", "solenoid"):
                pa = {k: inst[k] for k in ("path", "origin_mm", "axes", "scale")}
                if stem == "sector_2dc":
                    pa["angular_sector"] = [0, 0.0, 90.0]
                if stem == "sector_3dp":
                    pa["polarity_flip_axis"] = 0
                pas.append(pa)
        coils = []
        if stem == "helmholtz":
            coils = [dict(kind="loop", center_mm=[0, 0, z], normal=[0, 0, 1],
                          radius_mm=30, current_a=100) for z in (-15, 15)]
        if stem == "solenoid":
            coils = [dict(kind="solenoid", first_mm=[-50, 0, 0], last_mm=[50, 0, 0],
                          radius_mm=10, current_a=0.7958, turns=100)]
        run = dict(physics=dict(magnetic_pas=pas, coils=coils))
        if coils:
            inst = next(inst for inst in instances.values() if inst["magnetic"])
            run["physics"]["magnetic_support"] = {
                k: inst[k] for k in ("path", "origin_mm", "axes", "scale")}
        (dest / "run.json").write_text(json.dumps(run))
        args = [str(REPO / "target/release/simrep"), "fly", "--particles", str(dest / "ions.ion"),
                "--run-file", str(dest / "run.json"), "--bounds=" + ",".join(map(str, bounds)),
                "--duration-us", "10000", "--nonrelativistic", "--max-step-us", "0.02",
                "--tolerance", "1e-10",
                "--output", str(dest / "native.csv")]
        for inst in instances.values():
            if not inst["magnetic"]:
                args += ["--pa", inst["path"], "--origin=" + ",".join(map(str, inst["origin_mm"])),
                         "--axes=" + ",".join(map(str, sum(inst["axes"], []))), "--scale", str(inst["scale"])]
        # Helmholtz's Lua intentionally stops at 0.03 us rather than a surface.
        if stem == "helmholtz":
            args[args.index("--duration-us") + 1] = "0.03"
            args += ["--allow-time-limit"]
        native_out, native_err = call(args, REPO, timeout=600)
        (dest / "native.log").write_text(native_out + native_err)
        native = list(csv.DictReader((dest / "native.csv").open()))
        reference_args = list(args)
        reference_args[reference_args.index("--max-step-us") + 1] = "0.0025"
        reference_args[reference_args.index("--output") + 1] = str(dest / "native-reference.csv")
        reference_args[reference_args.index("--tolerance") + 1] = "1e-12"
        reference_out, reference_err = call(reference_args, REPO, timeout=600)
        (dest / "native-reference.log").write_text(reference_out + reference_err)
        converged = list(csv.DictReader((dest / "native-reference.csv").open()))
        convergence = max(math.dist([float(a[k]) for k in ("x_mm", "y_mm", "z_mm")],
                                    [float(b[k]) for k in ("x_mm", "y_mm", "z_mm")])
                          for a, b in zip(native, converged))
        paired = []
        final_map = {int(row[0]): list(map(float, row[1:])) for row in finals}
        for row in native:
            reference = final_map.get(int(row["ion"]))
            if reference is not None:
                paired.append(dict(position_mm=math.dist([float(row[k]) for k in ("x_mm", "y_mm", "z_mm")], reference[1:4]),
                                   tof_us=abs(float(row["tof_us"]) - reference[0]),
                                   ke_ev=abs(float(row["ke_ev"]) - reference[4])))
        times = re.findall(r"Fly completed\.\s*\d+ splats,\s*([0-9.]+) seconds", out)
        report[stem] = dict(field=field_report, ions=len(native), paired=len(paired),
                            native_reference_max_position_difference_mm=convergence,
                            max_position_difference_mm=max((p["position_mm"] for p in paired), default=None),
                            max_tof_difference_us=max((p["tof_us"] for p in paired), default=None),
                            max_ke_difference_ev=max((p["ke_ev"] for p in paired), default=None),
                            simion_compute_s=float(times[-1]) if times else None,
                            native_compute_s=float(re.search(r"(?:^|, )fly_s: ([0-9.]+)", native_err).group(1)))
        (dest / "comparison.json").write_text(json.dumps(report[stem], indent=2) + "\n")
        print(stem, json.dumps(report[stem]), flush=True)
    return report


def collisions(root, count):
    report = {}
    for name in ("hs1", "sds"):
        # Use the field-free HS1 geometry for both models, isolating transport
        # from geometry, space charge and other example-specific Lua actions.
        dest = copy_case("collision_" + name, ORACLE / "collision_hs1", root)
        model_path = ORACLE / ("collision_hs1/collision_hs1.lua" if name == "hs1"
                               else "collision_sds/collision_sds.lua")
        initialize = "" if name == "hs1" else """
  model.pressure=function() return 760 end
  model.temperature=function() return 273.15 end
  model.velocity=function() return 0,0,0 end
"""
        adjustable = ["_pressure_pa=0.53", "_temperature_k=273", "_gas_mass_amu=4",
                      "_sigma_m2=2.27e-18", "_random_seed=238", "_mark_collisions=0", "_trace_level=0",
                      "_vx_bar_gas_mmusec=0", "_vy_bar_gas_mmusec=0", "_vz_bar_gas_mmusec=0"] if name == "hs1" else [
                          "SDS_pressure_torr=760", "SDS_temperature_K=273.15",
                          "SDS_collision_gas_mass_amu=28.94515", "SDS_collision_gas_diameter_nm=0.366",
                          "SDS_diffusion=1", "SDS_enable=1"]
        model_args = sum((["--adjustable", setting] for setting in adjustable), [])
        step = 0.05 if name == "hs1" else 50.0
        program = f"""simion.workbench_program()
sim_segment_global=1
adjustable physics_record = 1
local model=simion.import [[{win(model_path)}]]
local initialize_run, other_actions = segment.initialize_run,segment.other_actions
local initialize = segment.initialize
local seeded = false
function segment.initialize_run()
  seeded=false
{initialize}
  if initialize_run then initialize_run() end
end
function segment.initialize()
  if initialize then initialize() end
  if not seeded then seed(237); seeded=true end
end
function segment.tstep_adjust()
  ion_time_step=math.min({step},50-ion_time_of_flight)
end
function segment.other_actions()
  if other_actions then other_actions() end
  if ion_time_of_flight >= 50-1e-9 then
    if physics_record ~= 0 then
      print(string.format('FINAL,%d,%.17g,%.17g,%.17g,%.17g,%.17g',
        ion_number,ion_time_of_flight,ion_px_mm,ion_py_mm,ion_pz_mm,ion_ke))
    end
    ion_splat=1
  end
end
function segment.terminate() end
"""
        (dest / "collision_hs1.lua").write_text(program)
        energy = "0.1" if name == "hs1" else "0"
        (dest / "ions.ion").write_text(("0,200,1,0,0,0,0,0," + energy + ",1,1\n") * count)
        common = ["fly", "--programs", "1", "--particles", "ions.ion",
                  "--grouped", "0", "--omit-relativistic-effects", "1", "--retain-trajectories", "0"]
        record_command = " ".join(common + model_args + ["collision_hs1.iob"])
        compute_command = " ".join(common + ["--adjustable", "physics_record=0"]
                                   + model_args + ["collision_hs1.iob"])
        # The documented seed(n) repeats within one process; the standalone
        # seed probe observes different sequences in separate processes.
        (dest / "batch.lua").write_text(
            f"simion.command [[{record_command}]]\nprint('PHYSICS_COMPUTE_BEGIN')\n"
            f"simion.command [[{compute_command}]]\n")
        batch_out = simion(["lua", "batch.lua"], dest, dest / "batch.log", threads=1)
        out, compute_out = batch_out.split("PHYSICS_COMPUTE_BEGIN", 1)
        (dest / "flight.log").write_text(out)
        (dest / "compute.log").write_text(compute_out)
        finals = [list(map(float, row[1:])) for row in records(out, "FINAL")]
        replayed = check_replay(out, compute_out)
        args = [str(REPO / "target/release/simrep"), "fly", "--particles", str(dest / "ions.ion"),
                "--collision", name, "--seed", "813", "--duration-us", "50",
                "--allow-time-limit", "--step-us", str(step), "--max-step-us", str(step),
                "--output", str(dest / "native.csv")]
        if name == "sds":
            args += ["--mbmr", str(ORACLE / "collision_sds/mbmr.dat"), "--temperature-k", "273.15"]
        native_out, native_err = call(args, REPO)
        (dest / "native.log").write_text(native_out + native_err)
        native = [[float(row[k]) for k in ("tof_us", "x_mm", "y_mm", "z_mm", "ke_ev")]
                  for row in csv.DictReader((dest / "native.csv").open())]
        if len(finals) != count or len(native) != count:
            raise RuntimeError("missing collision terminal records")
        comparisons = {}
        for i, label in enumerate(("tof_us", "x_mm", "y_mm", "z_mm", "ke_ev")):
            # Fixed-time TOF is a known atom. Round only below 1e-7 us;
            # retain the unrounded range to expose localization differences.
            a = [round(row[i], 7) if i == 0 else row[i] for row in finals]
            b = [round(row[i], 7) if i == 0 else row[i] for row in native]
            ks = ks_2samp(a, b, method="exact")
            comparisons[label] = dict(ks_d=float(ks.statistic), p_value=float(ks.pvalue),
                                      simion_mean=sum(a) / count, native_mean=sum(b) / count)
        times = re.findall(r"Fly completed\.\s*\d+ splats,\s*([0-9.]+) seconds", compute_out)
        report[name] = dict(ions=count, step_us=step, duration_us=50, oracle_seed=237, native_seed=813, simion_threads=1,
                            oracle_adjustables=adjustable, replayed_splat_records=replayed, distributions=comparisons,
                            raw_simion_tof_range=[min(r[0] for r in finals), max(r[0] for r in finals)],
                            simion_compute_s=float(times[-1]) if times else None,
                            native_compute_s=float(re.search(r"(?:^|, )fly_s: ([0-9.]+)", native_err).group(1)))
        print(name, json.dumps(report[name]), flush=True)
    return report


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--root", type=Path, default=None,
                    help="Output directory (default: $SIMREP_ORACLE_CACHE/physics-validation-20261023)")
    ap.add_argument("--ions", type=int, default=4096)
    ap.add_argument("--only", choices=("magnetic", "collision", "all"), default="all")
    ap.add_argument("--case", help="Run one magnetic IOB stem, e.g. sector_2dp")
    ap.add_argument("--check-replay", action="store_true",
                    help="Check existing collision recording/timing log replay without running SIMION")
    ap.add_argument("--stage", choices=("prepare", "native", "flight", "all"), default="all",
                    help="Separate foreground Windows runs from potentially long native PA setup")
    args = ap.parse_args()
    global ORACLE
    ORACLE = oracle_cache()
    if args.root is None:
        args.root = ORACLE / "physics-validation-20261023"
    args.root.mkdir(parents=True, exist_ok=True)
    if args.check_replay:
        for model in ("hs1", "sds"):
            folder = args.root / ("collision_" + model)
            n = check_replay((folder / "flight.log").read_text(), (folder / "compute.log").read_text())
            print(model, "replayed identical SIMION splat records:", n)
        return
    results = dict(date="2026-10-23", root=str(args.root))
    if args.only in ("magnetic", "all"):
        results["magnetic"] = magnetic(args.root, args.case, args.stage)
    if args.only in ("collision", "all"):
        results["collision"] = collisions(args.root, args.ions)
    target = args.root / ("results-" + args.only + ".json")
    target.write_text(json.dumps(results, indent=2) + "\n")
    print("Wrote", target)


if __name__ == "__main__":
    main()
