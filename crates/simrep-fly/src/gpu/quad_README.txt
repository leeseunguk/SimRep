Quadrupole GPU/SIMION reproduction, 2026-10-23

The original solved template is read-only:
  C:\Users\lsu10\Documents\SimRep\quadrupole-2\quadrupole.pa0 + .pa1/.pa2/.pa3
The working copy and all measured evidence are:
  C:\Users\lsu10\simrep-oracle\_quad_gpu\

All commands below run in WSL, with Windows processes foreground, bounded,
closed stdin, and a C-drive cwd. Set ROOT to the checkout containing these
Rust GPU modules and helper scripts. The measured checkout was:
  /home/lsu10/.omo/wt/t2ddf4d1928/m

ROOT=/home/lsu10/.omo/wt/t2ddf4d1928/m
S="$ROOT/crates/simrep-fly/src/gpu"
Q=/mnt/c/Users/lsu10/simrep-oracle/_quad_gpu
mkdir -p "$Q"

1. Generate 1000 ions: +1 e, 100 amu, 100 eV, x=1 mm, +X direction,
   disk radius=3*sqrt(i/999) mm, angle=i*2.399963229728653 rad.
   python3 "$S/quad_beam.py" "$Q/particles.ion" --ions 1000

2. Copy the template, adapt a known single-instance IOB, fast adjust
   electrodes to +2.5/-2.5/0 V, and measure SIMION TQual 3 and 209:
   (cd /mnt/c && timeout 340 python3 "$S/quad_oracle.py" \
      --beam "$Q/particles.ion" < /dev/null)
   The helper uses tools/simion-oracle/run_oracle.py, not a detached process.
   compute.iob has no readout Lua; readout.iob reports high-precision finals.
   Bounds are [0,-18,-18] to [60,18,18] mm; origin is 0, scale=1.
   Setup records the checked PA node transform: (1,0,0) -> (0.5,0,0) mm.

3. Run CPU/GPU coarse and independent 2x/4x re-solves (release test):
   export CARGO_TARGET_DIR='C:\Users\lsu10\simrep-gpu-target'
   export SIMREP_GPU_QUAD_REFERENCE='C:\Users\lsu10\simrep-oracle\_quad_gpu'
   export WSLENV=CARGO_TARGET_DIR:SIMREP_GPU_QUAD_REFERENCE
   manifest="$(wslpath -w "$ROOT/Cargo.toml")"
   (cd /mnt/c && set -o pipefail && timeout 900 \
      /mnt/c/Users/lsu10/.cargo/bin/cargo.exe test --release \
      --manifest-path "$manifest" -p simrep-fly --lib \
      quadrupole_grid_time_reference -- --nocapture < /dev/null 2>&1 | tr -d '\r')
   quad_reference.rs contains the complete reference construction:
   a fine node is fixed only when all its nonzero-weight coarse corners are
   fixed; interpolate only the initial guess, independently solve Laplace
   to relative residual 1e-12, then
   reconstruct compact C2 fields. Fine spacing is 0.25/0.125 mm.
   Flight tolerances are 1e-12, with 4x steps 0.05 and 0.025 us.
   The explicit coarse speed settings are 1e-8 and max step 0.1 us.

4. Compare:
   python3 "$S/quad_compare.py" "$Q"

5. GPU/CPU collision-rule agreement (does not overwrite the references):
   export SIMREP_GPU_QUAD_RULES='C:\Users\lsu10\simrep-oracle\_quad_gpu'
   export WSLENV=CARGO_TARGET_DIR:SIMREP_GPU_QUAD_RULES
   (cd /mnt/c && set -o pipefail && timeout 120 \
      /mnt/c/Users/lsu10/.cargo/bin/cargo.exe test --release --all-features \
      --manifest-path "$manifest" -p simrep-fly --lib \
      quadrupole_collision_rule_agreement -- --nocapture < /dev/null 2>&1 | tr -d '\r')
   rule-{nearest-node,node-cell}-{cpu,gpu}.csv and agreement text files retain
   the measurements. The original reference snapshot used nearest-node
   geometry. Use a separate work directory for new all-corner references.
   After merging the CPU default migration, the optional agreement fixture's
   legacy branch must explicitly call with_nearest_node_collisions().

The 4x time refinement is converged (~6e-10 mm RMS), but 2x->4x changes
endpoints by 1.145 mm RMS. Do not call this a converged spatial reference.
Coarse CPU/GPU endpoint RMS vs 4x is 2.022 mm, versus SIMION 0.822/0.790 mm.
No matched-precision quadrupole speedup has been demonstrated.
Do not run an 8x reference without coordinating the shared RAM budget.

Final Windows process hygiene:
  p="$(wslpath -w "$ROOT/tools/win/winproc.ps1")"
  (cd /mnt/c && timeout 60 \
    /mnt/c/Windows/System32/WindowsPowerShell/v1.0/powershell.exe \
    -NoProfile -ExecutionPolicy Bypass -File "$p" cleanup < /dev/null 2>&1 | tr -d '\r')
