# CPU magnetic compute, 2026-10-24

All seven strict-default cases beat the fresh SIMION compute medians below.
There is no remaining losing case in this control. No numeric tolerance,
step-control default, reconstruction order or accuracy gate was relaxed.

Ryzen 9 5950X; release Rust f64 under WSL2, 16 preconstructed Rayon workers.
SIMION runs natively on Windows with one worker, the original example PAs,
placements and field hooks, nonrelativistic dynamics, and no retained
trajectories. Its reported `Fly completed` interval is compared with Rust
`fly_many` only. Loading, reconstruction, map construction/calibration, pool
creation and result output are excluded.

## Actual matched-population compute

The short original 10-ion magnetic runs can report **0.00 s** in SIMION.
To resolve its 0.01 s timer, these controls repeat the captured original
START population: 100 times for magnet1/mag90, ten times for the sectors and
Helmholtz, once for the 100-ion solenoid. Every entry is an actual run at the
listed population, not an extrapolation. A separate SIMION audit restores
and prints all nine captured phase-space values; the maximum START error is
zero in every case. Timed runs disable those prints.

SIMION columns are medians of three flights. Rust columns are medians of five
flights on the same prepared configuration. Rust uses ordinary default RK45,
automatic step ceilings and the unchanged 1e-12 magnetic tolerances, with the
documented 0.03 us duration for Helmholtz. The existing field-aware automatic
tolerance selection, including its static-electric launch estimate, is
unchanged; see `default-flight.md`.

| Case | Ions | SIMION compute s | CPU compute s | SIMION / CPU |
| --- | ---: | ---: | ---: | ---: |
| magnet1 | 1,000 | 0.35 | 0.165596638 | 2.11 |
| mag90 | 1,000 | 0.25 | 0.178162641 | 1.40 |
| sector 2dp | 150 | 0.24 | 0.032743208 | 7.33 |
| sector 2dc | 500 | 0.23 | 0.024820506 | 9.27 |
| sector 3dp | 150 | 0.17 | 0.062724102 | 2.71 |
| Helmholtz | 270 | 0.60 | 0.012323900 | 48.69 |
| solenoid, mapped | 100 | 4.77 | 0.221888623 | 21.50 |

Fresh SIMION samples, in seconds:

| Case | Three samples |
| --- | --- |
| magnet1 | 0.38, 0.35, 0.35 |
| mag90 | 0.25, 0.25, 0.24 |
| sector 2dp | 0.24, 0.25, 0.24 |
| sector 2dc | 0.26, 0.23, 0.23 |
| sector 3dp | 0.17, 0.17, 0.18 |
| Helmholtz | 0.59, 0.60, 0.60 |
| solenoid | 4.42, 4.77, 5.11 |

## Unchanged accuracy gate

Maximum endpoint distance and absolute TOF error are against the exact
Biot-Savart/native PA reference at a 0.0025 us ceiling and 1e-12 tolerances.
Each repeated ion is checked against its corresponding original reference.
Splat reasons must agree. The retained old-API maximum-error envelopes are
the gates, not SIMION's much larger errors.

| Case | Endpoint mm | Endpoint gate mm | TOF us | TOF gate us |
| --- | ---: | ---: | ---: | ---: |
| magnet1 | 1.27745e-8 | 5.47282e-7 | 7.35311e-9 | 4.01298e-7 |
| mag90 | 5.63213e-9 | 2.54520e-7 | 3.44915e-11 | 2.31283e-9 |
| sector 2dp | 2.69811e-9 | 1.85622e-7 | 6.55120e-11 | 7.11352e-9 |
| sector 2dc | 2.66234e-9 | 3.01237e-7 | 3.00506e-11 | 3.69093e-9 |
| sector 3dp | 7.90770e-8 | 9.77063e-7 | 3.70319e-8 | 4.95515e-8 |
| Helmholtz | 1.46264e-13 | 1.46092e-8 | 0 | 2.69845e-11 |
| solenoid | 4.28512e-7 | 1.22619e-6 | 3.93079e-10 | 1.30382e-9 |

## Implementation and profile

`MagneticField::prepare_coil_map(bounds)` returns an immutable-borrow
`PreparedMagneticField`. It uses the GPU study's 0.0625 mm C2 axial-B/Br-over-r
map, the finite radial-axis limit and the same three filament-free rectangles.
The CPU representation interleaves the two components and directly contracts
the existing nine derivative jets per node, without computing unused scalar
gradients. The accepted solenoid has 3,386,571 nodes and about 488 MB of jets.

Every rectangle receives the same 512 deterministic off-node calibration
probes. It is used only when sampled maximum absolute error is at most
2e-9 G and sampled maximum relative error is at most 5e-10. Rejected regions,
noncoaxial/small coil groups and points outside accepted rectangles use exact
Biot-Savart. Calibration is not a global analytic error bound; the independent
full-flight gate above remains mandatory.

| Region | Maximum sampled absolute G | Maximum sampled relative |
| --- | ---: | ---: |
| inner tube | 1.35616449e-9 | 2.71637035e-10 |
| entrance fringe | 4.56778840e-12 | 9.93027565e-12 |
| exit fringe | 2.46870272e-12 | 7.54533044e-12 |

The source's public `coils` vector cannot change while its prepared wrapper
exists, avoiding a stale cache. Ordinary `MagneticField::sample` and
`PreparedMagneticField::exact` remain exact. CPU CLI flights prepare maps
before their compute interval; `--exact-coils` disables construction. Python
uses `exact_coils=True` for the same opt-out and returns the per-region
`coil_map_calibration` report.

The seven-case profile exposed redundant PA coordinate transforms and
extent/material queries. Magnetic PA sampling now transforms once, shares
that coordinate with the selected PA, and performs extent-only membership
instead of a pole-cell status query. Gradient reconstruction, rotation,
placement scaling, sector/polarity switches, overlap selection and material
collisions are unchanged. Reported endpoint/TOF values and accepted/rejected
step counts are unchanged in all six nonmapped cases.

On the same final solenoid control, exact and mapped five-run medians are
0.914052496 and 0.221888623 s: 4.12 times faster. At 100,000 active points
from the first captured trajectory, exact/mapped field sampling is
3,629.818/75.316 ns, or 48.2 times faster. Whole flights still perform exact
fallback outside the map, geometry tests and RK45 stages; the field ratio is
not the whole-flight ratio.

The pre-optimization repeated mag90 control actually lost: 0.369962812 s
versus SIMION's 0.25 s. Removing the duplicate PA work reduced the strict
control to 0.178162641 s. The old 0.02 us capped sector-3dp loss remains a
historical result, not a timing for these unchanged automatic defaults.

## Mag90 at explicitly matched precision

SIMION's maximum error against the converged native reference is
0.03152719720146 mm and 0.0001281514017943 us. A separate 63-configuration
RK45 sweep tests nine tolerances, 1e-12 through 1e-4, and automatic,
0.05/0.1/0.25/0.5/1/2 us ceilings. Each configuration flies the same 1,000
ions three times and is checked on both maximum errors and splat reasons.
These are cheapest **measured** passing configurations, not a claim of a
continuous global optimum.

| Mode | Tolerance | Ceiling us | CPU median s | Endpoint mm | TOF us | SIMION / CPU |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| unchanged strict defaults, five-run control | 1e-12 | automatic | 0.178162641 | 5.63213e-9 | 3.44915e-11 | 1.40 |
| cheapest measured retaining the old-API gate | 1e-11 | 0.5 | 0.106217708 | 5.50550e-8 | 5.44073e-10 | 2.35 |
| cheapest measured matching SIMION, comparison only | 1e-6 | 1 | 0.042293600 | 0.00337398 | 4.20225e-5 | 5.91 |

The last row passes SIMION's error budget but **does not** pass the much
tighter retained old-API gate. It is not used for the seven-case acceptance
table or any production default. Every tested 1e-5/1e-4 configuration fails
the SIMION endpoint or TOF budget; for example, automatic 1e-5 has
0.0172165 mm endpoint error but 0.000210356 us TOF error, so it is rejected.
Mag90 wins even at the unchanged strict defaults.

## Actual mapped-solenoid throughput

Three-run medians at the larger CPU16 control populations are:

| Ions | CPU compute s | Maximum endpoint mm | Maximum TOF us |
| ---: | ---: | ---: | ---: |
| 1,000 | 1.767366474 | 4.28512e-7 | 3.93079e-10 |
| 10,000 | 17.261339962 | 4.28512e-7 | 3.93079e-10 |

Every ion is actually integrated and checked; neither time is extrapolated.
For context, the earlier native-Windows exact-CPU controls in `fly-gpu.md`
were 6.056711600 and 59.597406400 s. They are historical controls on the same
hardware, not a paired Windows/WSL timing experiment. The paired exact/map
improvement claimed above uses the fresh 100-ion control.

## Reproduction and evidence

```bash
# Fresh SIMION, bounded foreground subprocesses, /mnt/c cwd, closed stdin.
script="$PWD/bench/magnetic_cpu_compute.py"
(cd /mnt/c && timeout 420 python3 "$script" < /dev/null)

# Five-run CPU controls, field profile and three-run mag90 precision sweep.
SIMREP_CPU_MAGNETIC_ORACLE=/mnt/c/Users/lsu10/simrep-oracle/physics-validation-20261023 \
  cargo test --release -p simrep-fly --test magnetic_performance magnetic_cpu_ \
  -- --nocapture --test-threads=1

# Additional actual solenoid populations, three measured flights apiece.
SIMREP_CPU_MAGNETIC_ORACLE=/mnt/c/Users/lsu10/simrep-oracle/physics-validation-20261023 \
SIMREP_MAGNETIC_CASE=solenoid SIMREP_CPU_MAGNETIC_COUNTS=1000,10000 \
  cargo test --release -p simrep-fly --test magnetic_performance \
  magnetic_cpu_map_and_profile -- --nocapture --test-threads=1
```

SIMION raw audits, timing logs, copied configurations and `summary.json` are
under `physics-validation-20261023/cpu-magnetic-compute`. CPU evidence is
`/tmp/simrep-cpu-magnetic-final-profile.log` and
`/tmp/simrep-cpu-magnetic-throughput.log`.

The CPU-only construction currently mirrors `src/gpu/coil_map.rs` because
that module has a concurrent owner. The orchestrator should later deduplicate
the coaxial-domain detection, value construction and calibration into shared
CPU-only code; this change deliberately does not edit `src/gpu/`.

## Validation and Windows cleanup

Formatting and both requested Clippy checks exit 0, including the Python
feature so its flight implementation is actually compiled:

```text
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo clippy -p simrep-py --all-targets --features python -- -D warnings
cargo test --workspace --release
235 passed; 0 failed; 0 ignored (46 test summaries)
cargo build --release -p simrep-cli
```

The release suite includes calibrated-map axis/rotation, exact-outside-map,
rejected-calibration and noncoaxial fallback regressions. Existing uniform B,
analytic fields, scalar PA mirrors/placement, collisions and software-Vulkan
fixtures pass. The opt-in magnetic replay separately passes both the
seven-case gates and the 63-configuration mag90 sweep. Logs are
`/tmp/simrep-cpu-magnetic-validation.log`.

A freshly built Python wheel in an isolated virtual environment passes four
live flight/physics tests, including default mapped coils, the explicit exact
opt-out, calibration-report values and terminal agreement. Log:
`/tmp/simrep-cpu-magnetic-python.log`. Real CLI default-map and `--exact-coils`
flights both finish at the requested 0.01 us; the maximum numeric terminal
difference is 6.96e-21. Their log and CSVs are
`/tmp/simrep-cpu-magnetic-cli.log` and
`/tmp/simrep-cpu-magnetic-{default,exact}.csv`. Tiny smoke-flight compute times
are not additional benchmark samples.

The language-server request timed out after 30 seconds. Compiler and Clippy
diagnostics are clean; no LSP success is claimed.

Every SIMION call is foreground, timeout-bounded, uses a `/mnt/c` cwd and
closed stdin, and retains stdout/stderr. After the last oracle call,
`tools/win/winproc.ps1 cleanup` exits 0. Full output is retained at
`/tmp/simrep-cpu-magnetic-winproc.log`; its final summary is:

```text
cmd.exe: 5
conhost.exe: 24
node.exe: 5
powershell.exe: 2
simrep-app.exe: 1
orphan conhost: 0
busy conhost (>50% of a core): 2
stopped orphan conhost: 0
```

The two busy conhosts have live WSL parents and were left untouched. No
SIMION, Windows Cargo or Rust compiler process appears in that final scan.
No commit was created.
