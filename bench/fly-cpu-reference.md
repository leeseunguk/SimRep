# CPU f64 flight reference

Date: 2026-10-23. WSL2, Ryzen 9 5950X (32 logical CPUs), Rust 1.95.0.
No Windows executable was launched for this benchmark.

## Workload

`fly-bench` launches 40 amu, +1 e ions at 100 eV, x=1 mm, with radii
0.1-2 mm and deterministic azimuths. A 91x21x1 cylindrical grid (1 mm cells)
samples an einzel-like paraxial Gaussian potential centered at x=45 mm:
`phi=exp(-(x-45)^2/(2*8^2))`, `basis=phi-r^2*phi''/4`. Fast adjustment applies
30 V to that unit basis. The detector is x=89 mm.

Flights use f64 interpolating cubic B-splines, adaptive Dormand-Prince RK45,
Rayon, position/velocity absolute tolerances 1e-8 in mm and mm/us, relative
tolerance 1e-8, initial step 0.005 us and maximum step 0.1 us. Setup is timed
separately; flight timing includes particle validation, Rayon dispatch,
integration, event localization and result allocation, but not particle/grid
construction or result aggregation.

This is **not the same field or workload as the recorded SIMION einzel
example**. Ratios below provide timing context only, not a matched-input
speedup claim. The phase-2 `tof` 10x milestone and SIMION trajectory agreement
are not established by this synthetic benchmark.

## Final release output

Commands, executed serially after building:

```sh
cargo build --release -p simrep-cli
target/release/simrep fly-bench --ions 10000
target/release/simrep fly-bench --ions 100000
```

```text
CPU f64 RK45, rayon, interpolated 91x21 cylindrical einzel-like grid
ions: 10000, detector arrivals: 10000
setup: 0.000976 s, fly: 0.329469 s
throughput: 30352 ions/s, accepted steps: 610969, rejected: 105071
trajectory checksum (TOF us + final y mm): 4.170875020468e4
SIMION recorded einzel baseline: 2.200 s; timing ratio: 6.68x (different synthetic field, not accuracy parity)
CPU f64 RK45, rayon, interpolated 91x21 cylindrical einzel-like grid
ions: 100000, detector arrivals: 100000
setup: 0.006632 s, fly: 3.223495 s
throughput: 31022 ions/s, accepted steps: 6108876, rejected: 1047619
trajectory checksum (TOF us + final y mm): 4.171338037460e5
SIMION recorded einzel baseline: 21.500 s; timing ratio: 6.67x (different synthetic field, not accuracy parity)
```

## Analytic accuracy

`cargo test -p simrep-fly -- --nocapture`:

```text
parallel plates: TOF 2.036101143574241e0 us, relative error 1.332e-15
harmonic frequency: relative error 2.488e-12
Boris: 1000 orbits, max relative energy 1.471e-13, radius 8.149e-14, one-orbit period 8.225e-9
interpolated flight: h=0.25000 mm, RMS position error=8.049289920e-6 mm (17 launch phases)
interpolated flight: h=0.12500 mm, RMS position error=4.721344785e-7 mm (17 launch phases)
interpolated flight: h=0.06250 mm, RMS position error=2.942401858e-8 mm (17 launch phases)
interpolated flight: h=0.03125 mm, RMS position error=2.353934601e-9 mm (17 launch phases)
observed position convergence order 4.0916
observed position convergence order 4.0041
observed position convergence order 3.6438
```

The 1000-orbit Boris test uses 1000 steps per orbit; the separate period test
uses 20,000 steps over one orbit. Boris preserves energy, not exact gyro-phase.

Final `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets --
-D warnings`, `cargo test --workspace`, and release build all exited 0.
Workspace tests included 12 flight tests and the concurrently merged CPU
multigrid tests. DX12 GPU smoke was skipped (no adapter); Vulkan smoke ran on
llvmpipe with f32 error 7.152557373046875e-7 and f64 error 0.
The LSP diagnostics service timed out twice; Clippy and Cargo supplied the
successful compiler diagnostics.
