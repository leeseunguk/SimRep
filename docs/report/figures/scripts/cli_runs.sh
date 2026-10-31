#!/usr/bin/env bash
# Re-run every CLI computation behind the cli-result figures (F04 F05 F08 F11 F12 F14 F15 F16 F19 F20)
# with the current release binary, then rebuild their data/ files and PNGs.
# Usage (from the repository root):  bash docs/report/figures/scripts/cli_runs.sh
# Needs: target/release/simrep and target/release/examples/particles (cargo build --release -p simrep-cli
# --examples), the SIMION oracle tree (SIMREP_ORACLE_DIR, default: target/simrep-oracle), SIMION examples under
# "/mnt/c/Program Files/SIMION-2024/examples", and a Python with numpy/scipy/matplotlib/pillow
# (PY=... to override; default: python3).
set -euo pipefail
ROOT="$(git rev-parse --show-toplevel)"
cd "$ROOT"
SIMREP="$ROOT/target/release/simrep"
D="${SIMREP_ORACLE_DIR:-$ROOT/target/simrep-oracle}"
export SIMREP_ORACLE_DIR="$D"
W="$ROOT/target/report-figs"
FIG="$ROOT/docs/report/figures"
DATA="$FIG/data"
SC="$FIG/scripts"
PY="${PY:-python3}"
export RAYON_NUM_THREADS=16
mkdir -p "$W"/{quad,stair,f05,hex,diel,cmag,sector,sol,beam}
"$SIMREP" --version

# F08 + F04: quadrupole user route (docs/geometry-pipeline.md 'V3 quadrupole user-route validation')
"$SIMREP" refine-geometry "$D/_quad_cut/quadrupole.gem" --electrode 1=1:2.5 --electrode 2=2:-2.5 \
  --electrode 3=3:0 --tolerance 1e-12 --out "$W/quad/quadrupole"
for mode in cuts nocuts; do
  extra=$([ "$mode" = cuts ] && echo "--cuts $W/quad/quadrupole.cuts.json" || echo "--no-cuts")
  "$SIMREP" fly --device cpu --pa "$W/quad/quadrupole.pa0" $extra --particles "$D/_quad_gpu/particles.ion" \
    --voltage 1=2.5,2=-2.5,3=0 --bounds=0,-18,-18,60,18,18 --tolerance 1e-11 --max-step-us 0.025 \
    --output "$W/quad/$mode.csv"
done
cp "$D/_quad_cut/physical-8-global.csv" "$DATA/F08-physical-8-global.csv"
cp "$W/quad/cuts.csv" "$DATA/F08-simrep-cuts.csv"; cp "$W/quad/nocuts.csv" "$DATA/F08-simrep-nocuts.csv"
{ echo "ion,tof_us,x_mm,y_mm,z_mm,ke_ev"
  "$PY" -c "import json,sys;print('\n'.join(l[6:] for l in json.load(open(sys.argv[1]))['stdout'].split('\r\n') if l.startswith('FINAL,')))" \
    "$D/_quad_gpu/quality-209-readout.json"; } > "$DATA/F08-simion-tq209.csv"
python3 "$D/_quad_cut/compare_quadrupole.py" "$D/_quad_cut/physical-8-global.csv" "$W/quad/cuts.csv"
cp "$W/quad/quadrupole.pa#" "$W/stair/quadrupole.pa#"
"$SIMREP" refine "$W/stair/quadrupole.pa#" --tolerance 1e-12   # staircase comparison field for F04

# F05: residual after k V-cycles (the script itself runs the CLI)
cp "$W/quad/quadrupole.pa#" "$W/f05/quadrupole.pa#"; cp "$D/_bench3d/sc3d_x2.pa#" "$W/f05/sc3d_x2.pa#"
(cd "$SC" && "$PY" F05_residual.py --run)

# F19: GEM Lua hexapole
cp "$D/_gem/examples/geometry/hexapole.gem" "$W/hex/"
(cd "$W/hex" && "$SIMREP" gem2pa hexapole.gem hexapole.pa#)
python3 tools/simion-oracle/compare_pa.py "$W/hex/hexapole.pa#" "$D/_gem/examples/geometry/hexapole.oracle.pa#"

# F14: dielectric sphere_3dp, epsilon 10 inside (the example Lua's kinside), CPU 1e-12
cp "$D/dielectric/sphere_3dp.gem" "$D/dielectric/sphere_3dp-dielectric.pa" "$W/diel/"
(cd "$W/diel" && "$SIMREP" gem2pa sphere_3dp.gem sphere_3dp.pa \
  && "$PY" "$SC/pa_prep.py" refinable sphere_3dp.pa \
  && "$PY" "$SC/pa_prep.py" relabel sphere_3dp-dielectric.pa sphere_3dp-dielectric-eps10.pa 1:1 5:10 \
  && cp sphere_3dp.pa sphere_3dp-eps10.pa \
  && "$SIMREP" refine sphere_3dp-eps10.pa --permittivity sphere_3dp-dielectric-eps10.pa --device cpu --tolerance 1e-12)

# F16: C magnet (planar Az), mu_r 1000 core and I = 2000 A over 2500 mm^2, as c_magnet_2dp.lua does
MA="$D/magnetic-potential-audit"
(cd "$W/cmag" \
  && "$PY" "$SC/pa_prep.py" relabel "$MA/c_magnet_2dp-mu.pa" mu-phys.pa '*:1000' 1:1 \
  && "$PY" "$SC/pa_prep.py" scale "$MA/c_magnet_2dp-jz.pa" jz-phys.pa 0.8 \
  && printf '{"permeability":"mu-phys.pa","current_density":"jz-phys.pa"}' > c_magnet_phys.json \
  && cp "$MA/c_magnet_2dp-Az.input.pa" Az-phys.pa \
  && "$PY" "$SC/pa_prep.py" refinable Az-phys.pa && "$PY" "$SC/pa_prep.py" vmax Az-phys.pa 1e6 \
  && "$SIMREP" refine Az-phys.pa --magnetic-material c_magnet_phys.json --device cpu --tolerance 1e-10)

# F11: 90-degree magnetic sector, positions every 0.2 us
PV="$D/physics-validation-20261023"
SEC=("$SIMREP" fly --device cpu --run-file "$PV/sector_2dp/run.json" --particles "$PV/sector_2dp/ions.ion"
     --bounds=-20,-120,-120,20,120,120 --nonrelativistic)
"${SEC[@]}" --output "$DATA/F11-sector2dp-simrep-end.csv"
echo "t_request_us,ion,tof_us,x_mm,y_mm,z_mm,reason" > "$DATA/F11-sector2dp-samples.csv"
for t in $(seq 0.2 0.2 18.6); do
  "${SEC[@]}" --duration-us "$t" --allow-time-limit --output "$W/sector/t.csv" > /dev/null
  awk -F, -v t="$t" 'NR>1{print t","$1","$2","$3","$4","$5","$7}' "$W/sector/t.csv" >> "$DATA/F11-sector2dp-samples.csv"
done
{ echo "ion,tof_us,x_mm,y_mm,z_mm,ke_ev"; grep '^FINAL,' "$PV/sector_2dp/flight.log" | cut -d, -f2- | tr -d '\r'; } \
  > "$DATA/F11-sector2dp-simion-final.csv"

# F12: solenoid with calibrated coil maps vs exact Biot-Savart
SOL=("$SIMREP" fly --device cpu --run-file "$PV/solenoid/run.json" --particles "$PV/solenoid/ions.ion"
     --bounds=-150,-50,-50,150,50,50 --nonrelativistic)
{ echo "\$ simrep fly ... --output map.csv"; "${SOL[@]}" --output "$DATA/F12-solenoid-map.csv" 2>&1
  echo "\$ ... --exact-coils --output exact.csv"; "${SOL[@]}" --exact-coils --output "$DATA/F12-solenoid-exact.csv" 2>&1; } \
  > "$DATA/F12-solenoid-cli-log.txt"
cp "$PV/solenoid/native-reference.csv" "$DATA/F12-solenoid-native-reference.csv"

# F15: cold uniform beam, detector planes 0.4..4.8 mm (beam ion file is data/F15-beam.ion)
echo "plane_x_mm,ion,tof_us,x_mm,y_mm,z_mm,reason" > "$DATA/F15-beam-planes.csv"
for d in 0.4 0.8 1.2 1.6 2.0 2.4 2.8 3.2 3.6 4.0 4.4 4.8; do
  "$SIMREP" fly --device cpu --particles "$DATA/F15-beam.ion" --charge-mode beam --beam-current-a 1e-8 \
    --bounds=-1,-20,-20,10,20,20 --plane "1,0,0,$d" --nonrelativistic --output "$W/beam/p$d.csv" > /dev/null
  awk -F, -v d="$d" 'NR>1{print d","$1","$2","$3","$4","$5","$7}' "$W/beam/p$d.csv" >> "$DATA/F15-beam-planes.csv"
done

# F20: FLY2 initial conditions (native generator used by tools/simion-oracle/check_fly2.py)
"$PY" - <<'EOF'
import os, re, subprocess
EX = "/mnt/c/Program Files/SIMION-2024/examples"
O = os.path.join(os.environ["SIMREP_ORACLE_DIR"], "_fly2")
DATA = "docs/report/figures/data"
hdr = "tob_us,mass_amu,charge_e,x_mm,y_mm,z_mm,az_deg,el_deg,ke_ev,cwf,color\n"
def rows(text):
    out = []
    for line in text.splitlines():
        if re.match(r"^[+-]?(?:\d|\.\d)", line):
            try:
                r = [float(x) for x in line.split(",")]
            except ValueError:
                continue
            if len(r) == 11:
                out.append(r)
    return out
for name, seeds, tag in (("bradbury_nielsen_grid/bngrid.fly2", [1], "bngrid"), ("plot/tof_histogram.fly2", range(1, 9), "tofhist")):
    nat = []
    for s in seeds:
        nat += rows(subprocess.run([os.path.abspath("target/release/examples/particles"), f"{EX}/{name}", "", str(s)],
                                   capture_output=True, text=True, cwd=O, check=True).stdout)
    orc = rows(open(f"{O}/{name.replace('/', '__')}.oracle.txt").read())
    for side, rr in (("simrep", nat), ("simion", orc)):
        with open(f"{DATA}/F20-{tag}-{side}.csv", "w") as f:
            f.write(hdr)
            f.writelines(",".join(repr(v) for v in r) + "\n" for r in rr)
EOF

# Plots
cd "$SC"
"$PY" F04_quad_section.py --extract "$W"
"$PY" F08_quad_endpoints.py
"$PY" F11_sector.py
"$PY" F12_coil_map.py
"$PY" F14_dielectric_sphere.py --extract "$W/diel/sphere_3dp-eps10.pa" "$D/dielectric/sphere_3dp.simrep-strict.pa"
"$PY" F15_beam_spread.py
"$PY" F16_c_magnet.py --extract "$W/cmag/Az-phys.pa" "$MA/c_magnet_2dp-Az.strict.pa" "$MA/c_magnet_2dp-mu.pa" "$MA/c_magnet_2dp-jz.pa"
"$PY" F19_gem_hexapole.py --extract "$W/hex/hexapole.pa#" "$D/_gem/examples/geometry/hexapole.oracle.pa#"
"$PY" F20_fly2.py
