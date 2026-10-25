# Real SIMION PA refinement

Date: 2026-10-23. Ryzen 9 5950X, 32 GB RAM, RTX 3070 8 GB.
Linux CPU release build under WSL2; native Windows release build for Vulkan.
SIMION: `SIMION 2024-09-26-8.2.1.0`.

## Command and format contract

```sh
simrep refine lens.pa#
simrep refine lens.pa --device cpu --tolerance 1e-12
simrep refine lens.pa# --device gpu --backend vulkan --tolerance 1e-12
```

CPU is the default, GPU defaults to Vulkan, and the default relative residual
tolerance is `1e-10`. All accuracy comparisons and the final timings below use
the stricter **`--tolerance 1e-12`**, with the unchanged f64 physical-residual
criterion and a 100-cycle budget.

Ordinary PA files are refined in place. PA# input is preserved. Integer electrode
labels 1..1000 generate PA1..PAn bases with the selected electrode at 10000 V and
all other electrodes at zero; fast adjust divides by that reference voltage.
Other electrode values form PA_, normalized so its largest-absolute electrode
is +10000 V. PA0 starts with adjustable electrodes at zero and the original
scalable voltages. Its trailer records the reference and representative electrode
offsets. A fresh PA0 is therefore not necessarily at the voltages of an earlier
fast-adjusted oracle PA0.

The adapter transposes SIMION cylindrical axial-x/radial-y storage into the
solver's radial-x/axial-y layout, including the two spacings. Planar nz=1 stays
2D. Unmasked faces use the solver's mirror condition; anti-mirror origin planes
have zero potential without changing their electrode flags in the output.
Nonzero electrodes on those planes and non-refinable field maps are rejected.
Files are encoded by `simrep-io`, not a separate CLI format writer.

## Inputs and comparison protocol

- Existing references were read from `C:\Users\lsu10\simrep-oracle\`.
- New references copied only PA# inputs into
  `C:\Users\lsu10\simrep-refine-reference\{default,tight}\<example>\`.
  Default uses SIMION's default convergence; tight uses `--convergence 1e-7`.
  The oracle worker checked SHA256 preservation of 19 original PA/log files.
- CPU output is in `/tmp/simrep-refine-cpu/<example>/`.
  Windows output is in `C:\Users\lsu10\simrep-refine-gpu\_bench3d\`.
- `tools/simion-oracle/compare_pa.py` independently decodes the binary arrays
  using Python's standard library. Max and RMS differences include **every
  grid node**, including electrodes. Every reported electrode-mask mismatch
  count was **zero**.
- Both small examples contain the same 91 x 20 x 1 cylindrical geometry:
  1,820 nodes. The large planar array is 259 cubed: **17,373,979 nodes**.

Reproduction, after making separate working copies:

```sh
cargo build --release -p simrep-cli
RAYON_NUM_THREADS=16 /usr/bin/time -p target/release/simrep refine \
  '/tmp/simrep-refine-cpu/_bench3d/sc3d_x2.pa#' --tolerance 1e-12

export CARGO_TARGET_DIR='C:\Users\lsu10\simrep-target'
export WSLENV="${WSLENV:+$WSLENV:}CARGO_TARGET_DIR"
set -o pipefail
timeout 900 /mnt/c/Users/lsu10/.cargo/bin/cargo.exe build --release -p simrep-cli \
  --manifest-path "$(wslpath -w "$PWD/Cargo.toml")" </dev/null 2>&1 | tr -d '\r'
/usr/bin/time -p timeout 180 /mnt/c/Users/lsu10/simrep-target/release/simrep.exe \
  refine 'C:\Users\lsu10\simrep-refine-gpu\_bench3d\sc3d_x2.pa#' \
  --device gpu --backend vulkan --tolerance 1e-12 </dev/null 2>&1 | tr -d '\r'

timeout 260 python3 tools/simion-oracle/run_oracle.py _bench3d --refine \
  --work-dir /mnt/c/Users/lsu10/simrep-refine-reference/tight/_bench3d \
  --refine-args='--convergence 1e-7' --timeout 180 </dev/null
python3 tools/simion-oracle/compare_pa.py generated.pa500 oracle.pa500
```

## Potential differences

### Against the already-existing files

Voltages below are decoded volts. Existing `_bench3d` potentials are identical
to the new tight reference; their PA0 representative offsets need not match.

| Existing example | File | CPU max (V) | CPU RMS (V) |
|---|---|---:|---:|
| einzel | einzel.pa0 | 110 | 49.84509254 |
| einzel | einzel.pa1 | 20.64808414 | 3.024263575 |
| einzel | einzel.pa2 | 20.15216248 | 3.423603252 |
| einzel | einzel.pa3 | 12.17497988 | 1.779136476 |
| test_plane | einzel.pa0 | 0 | 0 |
| test_plane | einzel.pa1 | 4.282781159e-2 | 8.320976076e-3 |
| test_plane | einzel.pa2 | 1.919294517e-2 | 2.939345046e-3 |
| test_plane | einzel.pa3 | 2.167802413e-2 | 3.470917572e-3 |
| _bench3d | sc3d_x2.pa0 | 2.124805718e-6 | 2.911887006e-7 |
| _bench3d | sc3d_x2.pa_ | 6.374417808e-5 | 8.735661893e-6 |
| _bench3d | sc3d_x2.pa500 | 6.360711359e-5 | 8.691120287e-6 |

Existing einzel PA0 was already fast-adjusted to electrode 2 = 110 V; a new
refine initializes all three adjustable electrodes to zero. That 110 V difference
is a different voltage setting, not a solver error. Its existing basis arrays
also differ substantially from a fresh same-input refine. Their effective
historical convergence setting is not recorded in the current `oracle.json`,
which contains a later fly run; they cannot be treated as a tight reference.
The following fresh comparisons remove both ambiguities.

### Fresh small-array references

These numbers apply separately to **both einzel and test_plane**.

| File | CPU vs SIMION default max (V) | RMS (V) | CPU vs SIMION 1e-7 max (V) | RMS (V) |
|---|---:|---:|---:|---:|
| einzel.pa0 | 0 | 0 | 0 | 0 |
| einzel.pa1 | 4.282781159e-2 | 8.320976076e-3 | 1.345968485e-6 | 3.305747983e-7 |
| einzel.pa2 | 1.919294517e-2 | 2.939345046e-3 | 7.146203416e-7 | 1.914409963e-7 |
| einzel.pa3 | 2.167802413e-2 | 3.470917572e-3 | 4.539110741e-7 | 1.216353375e-7 |

All three CPU bases used nine cycles, with final relative residuals
`2.912875274e-13`, `5.178964914e-13`, and `5.561158543e-13`.
The final small runs took 11.192 ms (einzel) and 12.684 ms (test_plane),
including PA read and all four writes. These are single runs, not timing medians.

### Large-array tight reference and CPU/GPU agreement

GPU differences against the existing files equal the GPU/tight columns here.

| File | CPU vs SIMION 1e-7 max (V) | RMS (V) | Vulkan vs SIMION 1e-7 max (V) | RMS (V) |
|---|---:|---:|---:|---:|
| sc3d_x2.pa0 | 2.124805718e-6 | 2.911887006e-7 | 2.141037243e-6 | 2.971483543e-7 |
| sc3d_x2.pa_ | 6.374417808e-5 | 8.735661893e-6 | 6.423112427e-5 | 8.914451521e-6 |
| sc3d_x2.pa500 | 6.360711359e-5 | 8.691120287e-6 | 6.327717892e-5 | 8.570496430e-6 |

| File | CPU vs Vulkan max (V) | RMS (V) |
|---|---:|---:|
| sc3d_x2.pa0 | 1.906136049e-8 | 6.794676775e-9 |
| sc3d_x2.pa_ | 5.718402463e-7 | 2.038403234e-7 |
| sc3d_x2.pa500 | 3.884042599e-7 | 1.384843019e-7 |

SIMION's convergence objective is a per-iteration voltage change, not a global
potential-error or f64 residual bound. The independent default-to-tight SIMION
comparison quantifies the stopping sensitivity:

| Example | File | SIMION default vs 1e-7 max (V) | RMS (V) |
|---|---|---:|---:|
| einzel and test_plane, each | einzel.pa1 | 4.282915756e-2 | 8.321300447e-3 |
| einzel and test_plane, each | einzel.pa2 | 1.919244879e-2 | 2.939219574e-3 |
| einzel and test_plane, each | einzel.pa3 | 2.167781820e-2 | 3.470839905e-3 |
| _bench3d | sc3d_x2.pa0 | 3.077908198e-2 | 4.340351753e-3 |
| _bench3d | sc3d_x2.pa_ | 9.233725518e-1 | 1.302105656e-1 |
| _bench3d | sc3d_x2.pa500 | 9.244163452e-1 | 1.302928336e-1 |

Tightening SIMION thus reduces the large basis discrepancy from about 0.924 V
to 0.000064 V, while CPU/GPU differences are at most 0.000000572 V.
This is consistent with remaining SIMION stopping error, but **does not prove
a 1e-7 V global potential difference**. A further independent audit attempted
`--convergence 1e-9`; this SIMION executable rejects it:

```text
error,FAIL: invalid value for argument "convergence": convergence objective (1e-09) must be no less than 1e-07.
```

That run exited 1 in 0.073 s and produced no PA outputs. It is not counted as a
successful higher-precision reference.

## End-to-end performance

Final CPU and GPU measurements ran **serially**, one run each. CPU used
`RAYON_NUM_THREADS=16`; Windows used its default host thread count. Vulkan selected
`NVIDIA GeForce RTX 3070 (Vulkan, DiscreteGpu)` and used GPU-native f64 residuals.
Every command constructs its hierarchy and pipelines, reads PA#, solves both
PA_ and PA500, and writes PA_, PA500, and PA0. Files were separate from references.
Writes include encoding and buffered OS I/O, not an explicit durable `fsync`.

| Device | Setup (ms) | Solve including transfers (ms) | PA I/O (ms) | CLI end-to-end (s) | Process wall (s) |
|---|---:|---:|---:|---:|---:|
| WSL CPU | 2376.460 | 5001.813 | 535.809 | 8.032514 | 8.05 |
| Windows Vulkan | 828.705 | 736.262 | 593.353 | 2.194013 | 2.50 |

Setup includes file-layout preparation, GPU adapter initialization when used,
and both solver setups. Solve includes GPU upload, solve, and readback. CLI
end-to-end additionally includes bookkeeping and local buffer teardown; process
wall includes process startup and final teardown after the printed timer.

| Device / basis | Cycles | Final absolute residual (V/mm^2) | Final relative residual |
|---|---:|---:|---:|
| CPU PA_ | 11 | 3.306922736e-9 | 1.102307579e-13 |
| CPU PA500 | 11 | 2.893443707e-9 | 9.644812356e-14 |
| Vulkan PA_ | 16 | 1.724296794e-8 | 5.747655981e-13 |
| Vulkan PA500 | 16 | 1.252749371e-8 | 4.175831236e-13 |

The required saved SIMION process-wall baselines are **2.191 s default** and
**14.365 s at 1e-7**. Against those baselines:

- CPU process wall is 3.67x slower than default and 1.78x faster than 1e-7.
- Vulkan process wall is 1.14x slower than default and **5.75x faster than 1e-7**.
  Using only the CLI timer instead would give 6.55x against 1e-7, but exclude
  process lifetime costs and is not the end-to-end claim here.
- The **5x default-SIMION end-to-end target is not met**. The measured GPU PA
  I/O alone exceeds its 0.4382 s budget. This report does not claim the whole
  phase 1 performance/precision acceptance criteria have been satisfied.

Fresh isolated SIMION re-runs measured 1.879 s default and 14.614 s tight for the
same large input; small default/tight times were einzel 0.193/0.278 s and
test_plane 0.218/0.261 s. Saved baseline values above remain the comparison of
record; the fresh runs show single-run wall-time variability.

## CPU convergence defect found by real input

Before this change the CPU PA_ solve exhausted 100 cycles, taking 27.16 s and
ending at relative residual `3.419829814e-5`. A 65-cubed synthetic octant-shell
regression isolated the cause: one global diagonal-scaled Gershgorin bound
grew through `4, 32, 157.54, 296.61, 2372.86`. Nearly empty coarse interpolation
columns therefore throttled every coarse row's smoother.

CPU coarse smoothing now uses the same **local absolute-row-sum scaling already
used on GPU**, retaining its Chebyshev smoothing and f64 convergence checks.
The 65-cubed case dropped from 67 cycles to eight. Its 32-cycle test budget
detects the old failure. No GPU port is outstanding for this fix.

## Validation and Windows process hygiene

The five CLI regressions run without Windows: PA#/PA0/basis round trip and
superposition versus an ordinary PA full solve (planar and cylindrical);
anisotropic 3D anti-mirror handling; anisotropic radial-y cylindrical harmonic
polynomial; CLI option validation; and nonconvergence preserving plain input.
Workspace tests and `cargo clippy --workspace --all-targets -- -D warnings`
exited 0. WSL also ran ten actual Vulkan/llvmpipe multigrid checks and two smoke
kernels; unavailable DX12 was explicitly skipped, not counted as a GPU pass.
The Rust language server could not run because this stable toolchain lacks
the `rust-analyzer` binary; compilation and Clippy provided static diagnostics.
Workspace `cargo fmt --all --check` also exited 0 after the orchestrator merged
the shared `main.rs` enum formatting. This task changed only the authorized
module declaration, enum variant, and match arm there. The two Rust files owned
by this task also passed direct `rustfmt --check`.

SIMION itself opened the generated Windows PA0 and fast-adjusted **both**
solution types. Assertions checked `(0,0,0) = 123 V` for electrode 500 and
`(258,258,258) = 42 V` for scalable electrode 0:

```text
SIMREP_LOAD_FAST_ADJUST_OK 259x259x259 slot0=42V electrode500=123V
```

All Windows executables were direct foreground children of timeout-bounded WSL
commands with stdin closed. None needed `taskkill`; no detached Windows start
was used. After the last Windows simulator run:

```sh
s="$(wslpath -w "$PWD/tools/win/winproc.ps1")"
(cd /mnt/c && timeout 60 powershell.exe -NoProfile -ExecutionPolicy Bypass \
  -File "$s" cleanup </dev/null)
```

Cleanup exited 0. Its process counts were:

```text
Name           Count
cmd.exe            4
conhost.exe       33
node.exe           4
powershell.exe     1
orphan conhost: 0
stopped orphan conhost: 0
```

`cargo.exe`, `rustc.exe`, `simion.exe`, and `simrep.exe` each had count zero.
The counted PowerShell is the cleanup process itself. Other listed processes
were retained because their parents were alive.
