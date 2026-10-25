# Verification paths

Run commands from the repository root unless a command changes directory.
The root Cargo workspace contains `crates/*`; it does not include the Tauri
app, Python extension behavior, or optional CAD features.

## Default workspace

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo clippy -p simrep-py --features python -- -D warnings
cargo test --workspace --release
```

The 14 external-fixture/benchmark tests below are reported as **ignored** by
default, even when their environment variables are set. Run them with both
the variables and `--ignored`. An explicit run without a required variable
fails instead of counting an unexecuted check as passed. Other GPU harnesses
can report adapter-dependent skips separately; workspace success alone is
not evidence of validation on physical GPU hardware.

## Ignored CPU oracle and sweep tests

Use existing, locally captured SIMION fixtures; these are not downloaded by
the tests. Replace the example absolute paths with your fixture locations.
`ORACLE` contains the magnetic case directories; `FIXTURES` contains
`_benchfly/einzel.pa0`, its fast-adjust siblings, and `_benchfly/einzel_10000.ion`.
`MBMR` is the installed SIMION diffusion-statistics file. Four Python oracle
tools - `tools/simion-oracle/check_fly2.py` (or `--root`),
`tools/simion-oracle/physics_validate.py` (or `--root`),
`tools/simion-oracle/quad_collision_probe.py` (or `--source`) and
`bench/default_flight.py` - read the same collection through
`SIMREP_ORACLE_CACHE` and exit with the expected layout when neither the
variable nor the explicit directory is given. The other scripts under
`tools/simion-oracle` still use their own historical default paths or
arguments; check each script's `--help` before running it.

```sh
export ORACLE=/path/to/physics-validation-20261023
export FIXTURES=/path/to/simrep-oracle
export MBMR=/path/to/collision_sds/mbmr.dat
export SIMREP_ORACLE_CACHE="$FIXTURES"

SIMREP_DEFAULT_SWEEP="$FIXTURES" cargo test --release -p simrep-fly \
  --test default_sweep einzel_step_control_sweep -- --ignored --nocapture
SIMREP_CPU_MAGNETIC_ORACLE="$ORACLE" cargo test --release -p simrep-fly \
  --test magnetic_performance magnetic_cpu_map_and_profile -- --ignored --nocapture
SIMREP_CPU_MAGNETIC_ORACLE="$ORACLE" cargo test --release -p simrep-fly \
  --test magnetic_performance magnetic_cpu_mag90_matched_precision -- --ignored --nocapture
SIMREP_PHYSICS_ORACLE="$ORACLE" cargo test --release -p simrep-fly \
  --test magnetic_performance magnetic_compute_and_convergence -- --ignored --nocapture
SIMREP_MBMR="$MBMR" cargo test --release -p simrep-fly \
  --test physics sds_runtime_icdf_transport_and_exact_drift -- --ignored --nocapture
```

## Ignored GPU oracle and benchmark tests

Use a physical Vulkan GPU. Native f64 and timestamp queries are required by
the tests whose ignore reasons name them. Copy fixtures into writable working
directories: some tests write reference CSVs or collision distributions.
Template layouts and capture recipes are in
[`bench/fly-gpu.md`](../../bench/fly-gpu.md) and
[`quad_README.txt`](../../crates/simrep-fly/src/gpu/quad_README.txt).
The latter's test invocations now also require `--ignored`.

```sh
export LIVE_TEMPLATES=/path/to/solved-app-templates
export BENCH_TEMPLATES=/path/to/solved-benchmark-templates
export QUAD=/path/to/quadrupole-reference
# ORACLE and MBMR are defined in the CPU oracle section above.

SIMREP_GPU_LIVE_BENCH="$LIVE_TEMPLATES" cargo test --release -p simrep-fly \
  --lib live_refly_benchmark -- --ignored --nocapture
SIMREP_GPU_BENCH="$BENCH_TEMPLATES" cargo test --release -p simrep-fly \
  --lib template_compute_benchmark -- --ignored --nocapture
SIMREP_GPU_KERNEL_BENCH="$LIVE_TEMPLATES" cargo test --release -p simrep-fly \
  --lib kernel_experiments -- --ignored --nocapture
SIMREP_GPU_PHYSICS="$ORACLE" cargo test --release -p simrep-fly \
  --lib gpu_physics_reference_agreement -- --ignored --nocapture
SIMREP_GPU_PHYSICS="$ORACLE" SIMREP_GPU_MBM="$MBMR" cargo test --release -p simrep-fly \
  --lib gpu_physics_collision_distributions -- --ignored --nocapture
SIMREP_GPU_PHYSICS="$ORACLE" SIMREP_GPU_TIMING=1 cargo test --release -p simrep-fly \
  --lib gpu_physics_compute_benchmark -- --ignored --nocapture
SIMREP_GPU_MBM="$MBMR" cargo test --release -p simrep-fly \
  --lib gpu_sds_exact_damped_rotation_and_diffusion -- --ignored --nocapture
SIMREP_GPU_QUAD_REFERENCE="$QUAD" cargo test --release -p simrep-fly \
  --lib quadrupole_grid_time_reference -- --ignored --nocapture
SIMREP_GPU_QUAD_RULES="$QUAD" cargo test --release -p simrep-fly \
  --lib quadrupole_collision_rule_agreement -- --ignored --nocapture
```

## Python extension and API

Python 3.10+, Rust, and venv support are required. This builds the actual PyO3
extension and CLI used by parity tests; the default empty Rust library does
not verify the Python API. See [`docs/python.md`](../python.md) for Windows
commands and `SIMREP_BIN` when the CLI uses a different output directory.

```sh
python3 -m venv /tmp/simrep-python
. /tmp/simrep-python/bin/activate
python -m pip install maturin numpy pytest scipy
maturin develop --release --manifest-path crates/simrep-py/Cargo.toml
cargo build --release -p simrep-cli
python -m pytest crates/simrep-py/tests -q
```

## CAD features

Install dynamically linked OCCT 7.6/7.7 development headers and libraries.
On Ubuntu, use the packages below. On Windows, use a matching MSVC x64 SDK,
set `OCCT_INCLUDE_DIR` and `OCCT_LIB_DIR`, and load the OCCT/third-party DLL
paths with the SDK's `env.bat` before testing. See
[`docs/step-import.md`](../step-import.md) and
[`docs/dev/windows-occt.md`](windows-occt.md) for the complete prerequisites.
Keep feature builds separate from default builds while CLI integration tests
are running so they cannot replace the same CLI executable.

```sh
sudo apt-get install --no-install-recommends \
  libocct-data-exchange-dev libocct-modeling-algorithms-dev
CARGO_TARGET_DIR=target/verify-cad cargo test --release -p simrep-cad --features occt
CARGO_TARGET_DIR=target/verify-cad cargo test --release -p simrep-cli --features cad
```

## Frontend and Tauri app

Node and npm are required for the frontend. Tauri additionally requires the
platform's native desktop build prerequisites (MSVC/WebView2 on Windows, or
GTK/WebKitGTK development libraries on Linux). The Tauri crate is a separate
workspace; see [`docs/dev/frontend.md`](frontend.md) for platform setup.

```sh
(cd app && npm ci && npm run typecheck && npm run build)
cargo test --release --manifest-path app/src-tauri/Cargo.toml
# With the OCCT SDK configured, also exercise the app's CAD feature:
cargo test --release --manifest-path app/src-tauri/Cargo.toml --features cad
```

## GPU on real hardware

Confirm `gpu-info` names a physical adapter, not `llvmpipe`/`Device type: Cpu`.
Run smoke checks and the relevant ignored GPU tests above on that adapter.
On a native Vulkan host:

```sh
cargo run --release -p simrep-cli -- gpu-info --backend vulkan
cargo run --release -p simrep-cli -- gpu-smoke --backend vulkan
cargo test --release -p simrep-fly --lib
cargo test --release -p simrep-solve
```

On this workstation, run Windows processes from WSL to reach the physical
GPU, keeping artifacts on a Windows drive and forwarding the Windows path
verbatim. Pick any Windows-visible directory for the build output and any
WSL path to a Windows `cargo.exe`:

```bash
export CARGO_TARGET_DIR='D:\simrep-target'   # any Windows path; /mnt/d/simrep-target in WSL
export CARGO_EXE='cargo.exe'                  # Windows PATH entry, or e.g. /mnt/c/Users/you/.cargo/bin/cargo.exe
export WSLENV="${WSLENV:+$WSLENV:}CARGO_TARGET_DIR"
manifest="$(wslpath -w "$PWD/Cargo.toml")"
target="$(wslpath "$CARGO_TARGET_DIR")"
(
  cd "$(dirname "$target")"
  set -eo pipefail
  timeout 900 "$CARGO_EXE" build --release \
    --manifest-path "$manifest" -p simrep-cli </dev/null 2>&1 | tr -d '\r'
  for backend in dx12 vulkan; do
    timeout 120 "$target/release/simrep.exe" \
      gpu-info --backend "$backend" </dev/null 2>&1 | tr -d '\r'
    timeout 120 "$target/release/simrep.exe" \
      gpu-smoke --backend "$backend" </dev/null 2>&1 | tr -d '\r'
  done
  timeout 1800 "$CARGO_EXE" test --release \
    --manifest-path "$manifest" -p simrep-fly -p simrep-solve </dev/null 2>&1 | tr -d '\r'
)
```

Follow [`docs/dev/windows-gpu.md`](windows-gpu.md) for `WSLENV` forwarding
of fixture variables when running the ignored tests and for process cleanup.
WSL software Vulkan success does not establish real GPU performance or
native f64 support on the target GPU.

## Published quantitative claims

```sh
python3 tools/check_claims.py
```

This standard-library-only check reads [`../claims.json`](../claims.json).
Its separate `readme` and `report_table13` lists register each published
number with an exact text fragment, source selector, derivation, and rounding
quantum. Report cells are selected from chapter 8's summary table in
`docs/report/manuscript.json`, not by JSON line number. The shared `sources`
map names the source file and section and extracts the underlying values
with Markdown regular expressions, CSV columns, or JSON selectors. The PDF
page count is counted from its uncompressed `/Type /Page` dictionaries.

The checker prints one PASS or FAIL per claim, with the derived value and
source, and exits nonzero on a mismatch, missing fragment/source, or
unregistered quantity. README prose is scanned for numbers with quantity
units (including both range endpoints); table 13 is scanned for every
numeric token. Code examples, links, scientific/product identifiers such as
`3D`, `4중극`, and `HS1`, list ordinals, and chapter/figure cross-references
are excluded. Additional bare counts, viewport widths, and the Python
version requirement are registered explicitly.

Rounding is decimal half-up at the registered power-of-ten quantum; a range
checks both endpoints. A literal upper-bound claim also checks the unrounded
source value. The earlier `6.4e-5 V 이내` wording failed this check because
the source maximum `6.423112427e-5 V` exceeded the literal bound. README and
table 13 now read `최대 6.42e-5 V 차이`, the rounded maximum, and the check
passes 58/58.

When a measurement or publication changes, update its source-backed claim
and rounding deliberately; do not copy the displayed number into a source
selector or widen rounding to hide a mismatch. This command verifies the
consistency of recorded evidence, not a fresh SIMION run, performance
measurement, installer build, or visual review.

## Frontend tests without a GPU

These run on any machine with Node 22+ (WSL here); no GPU, Rust toolchain or
native window is involved. Details and the demo-mode fixtures are in
[`docs/dev/frontend.md`](frontend.md) ("4. 검사").

```sh
cd app
npm ci
npm run typecheck                 # app + test/config sources (tsconfig.test.json)
npm test                          # vitest unit tests (src/**/*.test.ts)
npx playwright install chromium   # once per Playwright version
npm run e2e                       # Playwright smoke on demo mode (headless Chromium)
```

`npm test` covers the packet decoders (STR4 with its f64 end-state cases,
SPA1, SPQ1; `npm run check:flight` runs only that file, so it is not separate
evidence), analysis and CSV, plot helpers, refine progress and the
residual-chart model, the stores, i18n key/placeholder parity, the refusal
messages against every `Reason::new("…")` code in `app/src-tauri/src`, the
UI option lists against `src/scene/engineOptions.json`, and the demo fixtures.
`npm run e2e` serves the UI with `vite --mode demo` (mocked Tauri IPC
answering with packets recorded from the real native core) and clicks
through: start screen, einzel template, refine (live convergence chart),
fly, TOF histogram, end-state CSV export, dark/light theme
(`e2e/smoke.spec.ts`), plus refine completion and cancel-after-start
(`e2e/refine.spec.ts`). The tests step the recorded refine messages
themselves (`?replay=manual`) and await commands by count, so no assertion
depends on replay timing.

The option sync has a native half: `cargo test --manifest-path
app/src-tauri/Cargo.toml` (`options::tests`) proves every listed value is
accepted by the command that takes it and that `engineOptions.json` is
current. Both halves must pass; demo-mode tests do not exercise the Rust side.
