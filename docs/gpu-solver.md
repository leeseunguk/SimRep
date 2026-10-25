# GPU potential solver

`dielectric::Permittivity::solve_gpu` additively supports static scalar
variable permittivity through the existing exceptional-row/Galerkin path,
with both native and CPU-f64 residuals. See `docs/dielectric.md`.

This API remains Laplace-only. The additive CPU `mg::solve_poisson` source
path used by PIC is not available here: GPU PIC needs source-aware f64
residuals, weighted correction RHS, and charge deposit/gather kernels.

`Gpu::solve` accepts the CPU solver's `Grid` and `Options`, an x-fastest f64
potential slice, and a boolean Dirichlet mask. It supports 3D planar,
2D planar, and 2D axisymmetric cylindrical grids (including the r=0 axis
and positive-origin annuli). Unmasked outer faces are mirrors. Fixed input
values are preserved bit-for-bit.

## Numerical method

The f32 correction hierarchy uses the same endpoint-preserving multilinear
interpolation as the CPU reference. Interpolation is zero at fine electrodes;
restriction is its transpose. GPU setup assembles local 27-point Galerkin
operators `P^T (-W L) P`, so thin and isolated electrodes do not disappear
when coarsening. The fine stencil is implicit, with axis coefficients and
a mask; coarse rows are explicitly stored. Cylindrical nodal weights and
the axis coefficient four match `docs/cpu-solver.md`.

Coarse storage is stencil-major for coalesced accesses. Vulkan accumulates
each assembled row in a function-local array. Legacy DX12 FXC cannot compile
the nested gather loops with dynamically indexed writable locals, so that
backend accumulates in its invocation's disjoint storage row; source-level
lowering removes the unsupported local array before shader translation.
Static vector components also keep FXC from unrolling the enclosing
variable-length support loops. No floating-point atomics are needed.

Weighted Jacobi uses two distinct buffers with dispatch boundaries between
sweeps. The fine damping is 2/3. Coarse rows use a symmetric absolute-row-sum
preconditioner with damping 4/3, bounding its scaled spectral radius below
two. A V-cycle has three pre- and three post-sweeps by default, and 80
coarsest sweeps. Every active axis is coarsened until its length is at most
four; dimensions need not be cubic, odd, or powers of two.

The outer iterate is f64 and gauge shifted by the first fixed voltage.
Each outer iteration computes a difference-form f64 physical residual,
solves its weighted correction approximately with one f32 V-cycle, and adds
the f32 correction to the f64 iterate. Native GLSL through Naga implements
the f64 kernels on Vulkan; the infinity norm is reduced in f64 on the GPU.
Only the eight-byte convergence scalar is downloaded each iteration.
The report uses the same gauge-shifted Dirichlet RHS scale, absolute
tolerance, and relative tolerance as the CPU solver. Stencil coefficients
are divided by the largest inverse squared spacing internally and physical
residual units are restored in the report. The default relative tolerance
is `1e-10`; exhaustion of `max_cycles` reports nonconvergence.

Without `SHADER_F64`, the full f64 iterate and residual stay on the CPU.
Every cycle downloads one f32 correction (4 bytes/node), updates the host
f64 iterate, computes its residual using Rayon, and uploads a weighted f32
RHS (4 bytes/node). This costs at least 8 bytes/node/cycle of PCIe traffic
plus CPU stencil evaluation and synchronization. It is a correctness
fallback, not the hardware performance path. `solve_with_precision(...,
false)` explicitly exercises it on an f64-capable adapter.

## CLI and timings

```sh
cargo run --release -p simrep-cli -- refine-bench --device cpu --n 128
cargo run --release -p simrep-cli -- refine-bench --device gpu --backend vulkan --n 128
```

CPU remains the default. The benchmark defaults to Vulkan for the GPU because
the RTX 3070's DX12 adapter does not expose `SHADER_F64`. Existing `gpu-info`,
`gpu-smoke`, and `pa-info` commands remain available.

The GPU report separates adapter initialization, setup, upload, solve, and
readback wall times. Solve includes V-cycles, residuals, convergence readbacks,
and any CPU-fallback transfers. The final readback includes the potential
download and restoration of its voltage gauge. Grid generation and analytic
error evaluation are outside solver timing. Shader compilation and hierarchy
construction are cold setup costs; no warm solver cache is assumed.

Explicit coarse storage is about 108 bytes/coarse node plus three f32
vectors per level. Individual buffers must fit the adapter's maximum storage
binding size; a clear preflight error reports oversized matrices.

## Correctness checks

```sh
RAYON_NUM_THREADS=16 cargo test -p simrep-solve --test gpu_multigrid
```

The standalone harness checks DX12 and Vulkan, runs on Vulkan llvmpipe in WSL,
reports an explicit skip when no adapter exists, and fails on an available
adapter's errors.
It compares both CPU-residual and native-f64 paths with `mg::solve` on
anisotropic non-cubic 3D, 2D planar, axisymmetric and annular cylindrical,
and skinny grids. Mirror faces, off-coarse-grid point/wire electrodes,
fixed-value preservation, and a zero-cycle budget are exercised. Comparison
tolerance is `1e-9` times the maximum reference voltage; solve tolerance in
these small tests is `1e-12`.

## Measured Windows Vulkan results (2026-10-23)

RTX 3070 8 GB, Ryzen 9 5950X, Windows release build, Vulkan, native shader
f64, default smoothing and `1e-10` relative tolerance. The CLI reported
32 Rayon threads for host setup/copy operations. Times are milliseconds,
one final run per size; the driver shader cache had been exercised by
earlier development runs, but every process constructed a new hierarchy
and new pipelines. No solver or buffer cache was reused.

| Cube | Nodes | Device init | Setup | Upload | Solve | Readback | Solver total | Cycles / levels | Final relative residual |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| 128 | 2,097,152 | 241.629 | 106.058 | 7.388 | 111.966 | 6.897 | 232.312 | 14 / 7 | 3.309081830e-11 |
| 256 | 16,777,216 | 222.772 | 97.336 | 92.856 | 340.229 | 57.073 | 587.497 | 14 / 8 | 3.350336127e-11 |
| 539 | 156,590,819 | 209.009 | 767.558 | 1338.269 | 2359.281 | 1246.943 | 5712.053 | 12 / 9 | 7.405431823e-11 |

Maximum analytic discretization errors were `2.794196809e-5`,
`6.932718202e-6`, and `1.557122580e-6`, respectively. These are the expected
finite-grid errors, not the iterative solution error. A development run
at 512 cubed also converged in 14 cycles to `3.360554726e-11`.

539 cubed is the largest cubic grid this explicit coarse-matrix layout
can bind on this adapter, and it ran successfully on the 8-GB card.
The next size was rejected before GPU allocation:

```text
error: level 1 matrix needs 2149471188 bytes; storage binding limit is 2147483644
```

The 540-cubed parent needs 271 cubed coarse nodes, each with 27 f32
coefficients. Larger cubes would need matrix partitioning or a different
coarse representation, even if more device memory were available.

At 256 cubed, solve-only time is 6.44 times faster than SIMION's 2.191-second
loose 259-cubed baseline. Including setup, upload, and readback, the factor
is **3.73**, so the **5x loose-baseline end-to-end target is not met**.
The full solver is 24.45 times faster than the 14.365-second `1e-7 V`
SIMION baseline; including adapter initialization reduces that to 17.73.
These are throughput comparisons of similar sizes, **not an identical
geometry comparison**: this benchmark has harmonic cube-face data, whereas
SIMION's recorded case has two spherical shells and multiple solution
arrays. Phase 1's identical-PA acceptance measurement remains separate.

The final 256-cubed command printed:

```text
Adapter: NVIDIA GeForce RTX 3070 (Vulkan, DiscreteGpu)
Device initialization: 222.772 ms
Residual path: GPU f64
Setup: 97.336 ms, upload: 92.856 ms, solve: 340.229 ms, readback: 57.073 ms
Grid: 256^3 (16777216 nodes), Rayon threads: 32
Elapsed (setup + upload + solve + readback): 587.497 ms
Cycles: 14, levels: 8
Relative residual (f64): initial = 1.000000000e0, final = 3.350336127e-11
Max analytic discretization error: 6.932718202e-6
```

Reproduction from the repository root, keeping Windows subprocesses in the
foreground with closed stdin and a C-drive working directory:

```sh
manifest="$(wslpath -w "$PWD/Cargo.toml")"
export CARGO_TARGET_DIR='C:\Users\lsu10\simrep-target'
export WSLENV="$WSLENV:CARGO_TARGET_DIR"
set -o pipefail
(cd /mnt/c && timeout 900 /mnt/c/Users/lsu10/.cargo/bin/cargo.exe build \
  --release -p simrep-cli --manifest-path "$manifest" </dev/null 2>&1 | tr -d '\r')
for n in 128 256 539; do
  (cd /mnt/c && timeout 300 /mnt/c/Users/lsu10/simrep-target/release/simrep.exe \
    refine-bench --device gpu --backend vulkan --n "$n" </dev/null 2>&1 | tr -d '\r')
done
```

WSL `cargo fmt --all -- --check`, `RAYON_NUM_THREADS=16 cargo test --workspace
-- --nocapture`, and `cargo clippy --workspace --all-targets -- -D warnings`
all exited 0. All ten native/fallback GPU comparisons executed on llvmpipe;
their largest difference from the CPU f64 solution was `5.974e-12 V`.
The existing Vulkan f32/f64 smoke checks also passed; WSL DX12 explicitly
skipped because no adapter exists. The Rust LSP daemon timed out twice;
compiler and strict Clippy diagnostics were clean.

Windows `cargo.exe test --release -p simrep-solve --test gpu_multigrid`
also exited 0: five CPU-f64-fallback comparisons executed on DX12 and ten
native/fallback comparisons on Vulkan, all within `5.974e-12 V` of CPU
solutions. An actual DX12 `refine-bench --device gpu --backend dx12 --n 16`
printed `Residual path: CPU f64 (full correction download + residual upload
each cycle)` and converged in 13 cycles to `5.273438080e-11`, confirming that
the missing-feature fallback is operational on hardware.

The final required cleanup command was:

```sh
s="$(wslpath -w tools/win/winproc.ps1)"
(cd /mnt/c && timeout 30 powershell.exe -NoProfile -NonInteractive \
  -ExecutionPolicy Bypass -File "$s" cleanup < /dev/null)
```

Its actual output was:

```text
== process counts ==

Name           Count
----           -----
cmd.exe            5
conhost.exe       32
node.exe           4
powershell.exe     2

== conhost by parent ==

Name                              Count
----                              -----
wsl.exe                              10
wslhost.exe                           4
cmd.exe                               4
powershell.exe                        2
node_repl.exe                         2
lmgrd.exe                             2
dispatcher.exe                        1
wlanext.exe                           1
rlm.exe                               1
node.exe                              1
GoogleDriveFS.exe                     1
matlab-mcp-server-windows-x64.exe     1
codex.exe                             1
sunshine.exe                          1

orphan conhost: 0
stopped orphan conhost: 0
```

No `cargo.exe`, `rustc.exe`, or `simrep.exe` process remained.
