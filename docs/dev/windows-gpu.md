# Windows GPU development from WSL2

The WSL2 environment has CUDA interop but no hardware Vulkan adapter. A
software Vulkan adapter such as Mesa llvmpipe may still be present; its
`Device type: Cpu` must not be mistaken for the RTX 3070. Run CPU builds
and tests with Linux Cargo; run hardware GPU checks as Windows processes. The
supported native backends are DX12 and Vulkan, with a high-performance adapter
preference. The initial hardware is an NVIDIA GeForce RTX 3070 (8 GB).

## Build and run

Run these commands in Bash, at the repository root. Windows Rust is installed
at `%USERPROFILE%\.cargo\bin\cargo.exe`. Build artifacts stay on C: rather
than the slow `\\wsl.localhost\...` filesystem.

```bash
cargo build && cargo test

export CARGO_TARGET_DIR='%USERPROFILE%\simrep-target'
# WSLENV explicitly forwards this variable to Windows. Do not use /p:
# CARGO_TARGET_DIR is already a Windows path.
export WSLENV="${WSLENV:+$WSLENV:}CARGO_TARGET_DIR"
set -o pipefail

timeout 900 /mnt/c/Users/<user>/.cargo/bin/cargo.exe build \
  --manifest-path "$(wslpath -w "$PWD/Cargo.toml")" \
  </dev/null 2>&1 | tr -d '\r'

timeout 120 /mnt/c/Users/<user>/simrep-target/debug/simrep.exe gpu-info \
  --backend dx12 </dev/null 2>&1 | tr -d '\r'
timeout 120 /mnt/c/Users/<user>/simrep-target/debug/simrep.exe gpu-info \
  --backend vulkan </dev/null 2>&1 | tr -d '\r'

timeout 120 /mnt/c/Users/<user>/simrep-target/debug/simrep.exe gpu-smoke \
  --backend dx12 </dev/null 2>&1 | tr -d '\r'
timeout 120 /mnt/c/Users/<user>/simrep-target/debug/simrep.exe gpu-smoke \
  --backend vulkan </dev/null 2>&1 | tr -d '\r'

timeout 900 /mnt/c/Users/<user>/.cargo/bin/cargo.exe test -p simrep-solve \
  --manifest-path "$(wslpath -w "$PWD/Cargo.toml")" \
  </dev/null 2>&1 | tr -d '\r'
```

`--backend auto` is the default and considers DX12 and Vulkan together.
Select each backend explicitly to establish its own f64 capabilities.
`gpu-info` and `gpu-smoke` exit unsuccessfully if no adapter is found;
they never claim a skipped computation passed.

The output pipe and closed stdin are intentional. Direct Windows executables
under a WSL pseudo-terminal can stall during the console cursor handshake;
using a pipe avoids that handshake. `pipefail` preserves a failed or timed-out
Windows command's exit status.

## What the smoke check establishes

The compute grid is 64 x 64 x 64 with unit spacing and a seven-point
Laplacian. It computes `r = b - Laplacian(x)` on interior nodes and writes zero
on boundary nodes. Deterministic nonconstant potential and RHS values exercise
all six neighbors, cancellation, indexing, and boundary treatment. All 262,144
output nodes are compared with a CPU f64 reference on the same exactly
f32-representable inputs.

- f32 uses WGSL, with maximum absolute error tolerance `2e-6`.
- If the adapter exposes `SHADER_F64`, the device enables it and also runs a
  native f64 GLSL kernel, with tolerance `1e-12`. WGSL has no f64 type; Naga
  translates the GLSL kernel to the native backend's shader representation.
  This path is not portable to browser WebGPU.
- Missing `SHADER_F64` is printed as an explicit f64 skip. Future residual
  correction on that backend must use the CPU f64 path.
- GPU kernel time uses `TIMESTAMP_QUERY` around the compute pass if supported.
  Host submission-through-readback time is reported separately. Neither
  includes shader compilation; the host interval may include queued buffer
  initialization work. These are single-dispatch smoke timings, not a
  throughput benchmark.
- Non-finite results or errors above tolerance make the command fail.

Linux `cargo test` runs CPU reference and CLI tests plus a standalone GPU
harness. That harness prints explicit skips when DX12/Vulkan adapters are
absent, and reports the number of actual GPU checks executed. A skipped check
is not counted as a successful GPU validation. Device, shader, submission, or
numerical errors on an available adapter fail the test process.

## Verified results (2026-10-23)

Dependencies: wgpu `30.0.1` (current stable from `cargo search wgpu`), clap
`4.6.7`. Linux Cargo was `1.95.0`; Windows Cargo was `1.97.1`.
Both Windows backends selected `NVIDIA GeForce RTX 3070`, `DiscreteGpu`.
The Vulkan driver reported `NVIDIA (591.86)`; DX12 reported `32.0.15.9186`.

Actual `gpu-smoke --backend dx12` output (adapter feature list omitted):

```text
Adapter: NVIDIA GeForce RTX 3070
Backend: Dx12
Device type: DiscreteGpu
Driver: 32.0.15.9186 ()
SHADER_F64: false
Grid: 64^3, unit-spacing residual b - Laplacian(x)
f32 PASS: GPU kernel = 0.013 ms, submit + readback = 3.992 ms, max abs error vs CPU f64 = 5.364418030e-7 (tolerance 2.0e-6, 262144 cells)
f64 SKIP: SHADER_F64 unavailable; residual correction must use CPU f64.
```

Actual `gpu-smoke --backend vulkan` output (adapter feature list omitted):

```text
Adapter: NVIDIA GeForce RTX 3070
Backend: Vulkan
Device type: DiscreteGpu
Driver: NVIDIA (591.86)
SHADER_F64: true
Grid: 64^3, unit-spacing residual b - Laplacian(x)
f32 PASS: GPU kernel = 0.014 ms, submit + readback = 3.745 ms, max abs error vs CPU f64 = 5.364418030e-7 (tolerance 2.0e-6, 262144 cells)
f64 PASS: GPU kernel = 0.020 ms, submit + readback = 0.815 ms, max abs error vs CPU f64 = 0.000000000e0 (tolerance 1.0e-12, 262144 cells)
```

Use **Windows Vulkan** for native f64 residual correction on this machine.
The measured zero f64 discrepancy is for these exactly representable test
inputs, not a claim of zero error for arbitrary simulations.

Linux `cargo fmt --all -- --check`, `cargo build && cargo test`, and
`cargo clippy --workspace --all-targets -- -D warnings` all exited 0.
The Linux test harness found a CPU software Vulkan implementation:

```text
SKIP gpu_smoke Backends(DX12): no adapter found
gpu_smoke Vulkan: llvmpipe (LLVM 20.1.2, 256 bits)
PASS f32: max abs error = 7.152557373046875e-7
PASS f64: max abs error = 0e0
GPU checks: 2 executed, 1 skipped
```

Windows `cargo.exe build`, both `gpu-info` commands, both `gpu-smoke`
commands, and workspace `cargo.exe test` all exited 0. Windows tests reported:

```text
gpu_smoke Dx12: NVIDIA GeForce RTX 3070
PASS f32: max abs error = 5.364418029785156e-7
SKIP f64: SHADER_F64 unavailable
gpu_smoke Vulkan: NVIDIA GeForce RTX 3070
PASS f32: max abs error = 5.364418029785156e-7
PASS f64: max abs error = 0e0
GPU checks: 3 executed, 1 skipped
```

## Windows process hygiene

Every Windows invocation above is direct, foreground, and bounded by
`timeout`. Do not use `cmd.exe /c start` or detached `Start-Process`.
A timeout of the WSL wrapper does not guarantee that its Windows child died.
If a command stalls, identify only the processes started for that command:

```bash
timeout 60 /mnt/c/Windows/System32/WindowsPowerShell/v1.0/powershell.exe \
  -NoProfile -Command \
  'Get-CimInstance Win32_Process | Where-Object { $_.Name -in @("cargo.exe", "simrep.exe") } | Select-Object ProcessId,CreationDate,CommandLine | Format-List' \
  </dev/null 2>&1 | tr -d '\r'
```

Then run `timeout 30 /mnt/c/Windows/System32/taskkill.exe /T /F /PID <pid>`
with the actual task-owned Windows PID, piping output as above. Never kill
unrelated Windows processes. After any crash or timeout, and before finishing,
run the repository's console cleanup and retain its process counts:

```bash
timeout 60 /mnt/c/Windows/System32/WindowsPowerShell/v1.0/powershell.exe \
  -NoProfile -ExecutionPolicy Bypass \
  -File "$(wslpath -w $REPO/tools/win/winproc.ps1)" \
  cleanup </dev/null 2>&1 | tr -d '\r'
```

No task-owned `cargo.exe`, `rustc.exe`, `simrep.exe`, or other long-running
Windows process should remain.
