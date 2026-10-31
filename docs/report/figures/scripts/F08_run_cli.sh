#!/usr/bin/env bash
set -e
REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../../.." && pwd)"
D="${SIMREP_ORACLE_DIR:-$REPO/target/simrep-oracle}"
cd "$REPO"
mkdir -p target/report-figs/quad
target/release/simrep --version
RAYON_NUM_THREADS=16 target/release/simrep refine-geometry "$D/_quad_cut/quadrupole.gem" --electrode 1=1:2.5 --electrode 2=2:-2.5 --electrode 3=3:0 --tolerance 1e-12 --out target/report-figs/quad/quadrupole
RAYON_NUM_THREADS=16 target/release/simrep fly --device cpu --pa target/report-figs/quad/quadrupole.pa0 --cuts target/report-figs/quad/quadrupole.cuts.json --particles "$D/_quad_gpu/particles.ion" --voltage 1=2.5,2=-2.5,3=0 --bounds=0,-18,-18,60,18,18 --tolerance 1e-11 --max-step-us 0.025 --output target/report-figs/quad/cuts.csv
RAYON_NUM_THREADS=16 target/release/simrep fly --device cpu --pa target/report-figs/quad/quadrupole.pa0 --no-cuts --particles "$D/_quad_gpu/particles.ion" --voltage 1=2.5,2=-2.5,3=0 --bounds=0,-18,-18,60,18,18 --tolerance 1e-11 --max-step-us 0.025 --output target/report-figs/quad/nocuts.csv
