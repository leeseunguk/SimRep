# Report evidence digest

Audit date: 2026-10-24. This is a fact index, not a fresh benchmark. Project
files were read only; the only executable query was
`target/release/simrep --help` (exit 0). No builds or simulations were run.
Source ranges refer to the main checkout at audit time. A source at the end
of a table supplies **every row**, its units, and its stated conditions.
Missing thread counts or isolation guarantees are marked unknown, not inferred.
Historical measurements remain useful only under their original conditions.

## 1. Goals and judging rules

- Build a native replacement for SIMION-2024's core potential-array solving,
  flight, geometry, time-dependent voltage and space-charge workflows; do not
  run arbitrary Lua workbench programs. User logic is configuration plus a
  voltage-expression DSL. Speed means **actual compute only**, excluding
  initialization, input, reconstruction/upload, output; compare at matched
  accuracy. Refine oracle: SIMION's strictest accepted `--convergence 1e-7`
  (voltage-change objective, not global potential error). Flight authority:
  independent spatial/time refinement, not equality to SIMION's private
  electrode treatment. `docs/PLAN.md:5-15` ("1. 목표").
- Reference hardware: Ryzen 9 5950X, 16 cores/32 logical threads, RTX 3070
  8 GB, 32 GB RAM. CPU studies generally use release Linux under WSL2;
  hardware GPU studies use native Windows Vulkan. SIMION is native Windows
  `2024-09-26-8.2.1.0`. Driver 591.86; RTX FP64 throughput is 1/64 FP32.
  WSL lacks the hardware Vulkan route described in the plan; llvmpipe test
  passes are software correctness, not RTX timing.
  `bench/simion-baseline.md:3-7` (preamble);
  `docs/PLAN.md:17-31` ("환경과 제약").
- Oracle harness launches foreground `simion.exe --nogui --noprompt`.
  Its wall clock includes roughly 0.05-0.10 s startup; use SIMION's own
  `Refine completed`/`Fly completed` interval for compute. Refine default
  threads are uncapped; explicit single-thread controls are distinguished.
  Flight was effectively single-threaded in the baseline.
  `bench/simion-baseline.md:3-7,25-37` ("Fly").
- Baseline coverage: phase-zero 28 calls over 10 example groups, all exit 0;
  later advanced-physics coverage 236 calls, all exit 0. The original quad
  IOB did not attach its Lua and is a static-DC oracle, not RF validation.
  `bench/simion-baseline.md:91-115,117-146` ("Oracle coverage").
- Stage goals: large 3D refine at least 5x strict SIMION, flight at least
  10x with accuracy satisfied, equal-memory sphere potential error at least
  10x lower. Octree is explicitly deferred: existing cut cells exceed the
  equal-memory goal; a twofold patch would cost 4.1-5.7x node memory and
  require a hanging-node symmetric operator.
  `docs/PLAN.md:126-155` ("단계별 계획").

## 2. Architecture and entry points

Actual root workspace is `crates/*`; app Rust is a separate workspace.
`Cargo.toml:1-19`; `docs/dev/frontend.md:3-8` (preamble/path table).

| Crate | Role | Exact source |
|---|---|---|
| simrep-core | Grid, PA representation and shared types | `crates/simrep-core/Cargo.toml:1-3`; `src/lib.rs:1` |
| simrep-io | SIMION file I/O, PA/fast adjust, ION/FLY2 and workbench support | `crates/simrep-io/Cargo.toml:1-3`; `docs/fly2.md:1-24`; `docs/dev/frontend.md:196-200` |
| simrep-geom | Solver-independent signed-distance geometry | `crates/simrep-geom/src/lib.rs:1-5` |
| simrep-gem | GEM parser, bounded Lua preprocessing, ordered fills, PA rasterization | `crates/simrep-gem/src/lib.rs:1-5` |
| simrep-lua | Shared bounded Lua 5.1 runtime | `crates/simrep-lua/src/lib.rs:1` |
| simrep-mesh | Triangle import, topology/repair, BVH/SDF, scientific export | `crates/simrep-mesh/src/lib.rs:1-4` |
| simrep-cad | Optional dynamically linked OCCT STEP/IGES B-rep import | `crates/simrep-cad/src/lib.rs:1-4` |
| simrep-solve | CPU/GPU Galerkin multigrid, cut cells, material/Poisson/magnetic solves | `crates/simrep-solve/Cargo.toml:1-3`; `docs/cpu-solver.md:3-23`; `docs/magnetostatics.md:7-61` |
| simrep-fly | Field reconstruction, RK45/Boris, flight, collisions and charge models | `crates/simrep-fly/Cargo.toml:1-3`; `src/lib.rs:1-23` |
| simrep-cli | Batch command-line frontend | `crates/simrep-cli/Cargo.toml:1-3` |
| simrep-py | Opt-in PyO3/NumPy extension and parameter scans | `crates/simrep-py/Cargo.toml:1-3`; `docs/python.md:3-11` |
| simrep-app | Tauri Rust commands, sessions, binary IPC and native jobs | `docs/dev/frontend.md:3-8,130-168` |

- App: Tauri 2 + React + TypeScript/Vite; WebGPU viewport. Binary IPC carries
  PA previews and trajectories; Rust performs simulation natively.
  `docs/dev/frontend.md:1-8,130-168` ("구조 요약").
- Observed `--help` commands: `gpu-info`, `gpu-smoke`, `pa-info`, `gem2pa`,
  `fly-bench`, `fly`, `refine-bench`, `refine`, `refine-sdf`,
  `refine-geometry`, `mesh-import`, `export`, `help`.
  Checkout declaration agrees: `crates/simrep-cli/src/main.rs:26-57`
  (`Command`). **No top-level magnetostatics command is in that enum.**
- Python package is `simrep`; wheels use CPython stable ABI
  `abi3-py310` (Python >=3.10), Maturin/PyO3. Ordinary workspace builds
  compile an empty opt-in extension crate without Python/NumPy/libpython.
  Shared auxiliary frontends run in-process, not CLI subprocesses.
  CLI parity, refine/geometry/mesh/export/flight/physics keywords:
  `docs/python.md:3-95` ("CLI parity"). Native I/O, sampling, solving,
  reconstruction and integration release the GIL:
  `docs/python.md:375-385` ("Flight options").
- **Documentation drift:** PLAN's crate tree omits later GEM/Lua/geom/mesh/CAD/
  Python crates; use the actual workspace above. The only
  `crates/*/README.md` present is `crates/simrep-fly/README.md`.

## 3. Potential solver

### Implemented operator and iteration

- Cartesian 2D (`nz=1`) or 3D second differences; anisotropic spacing allowed.
  Unmasked outer faces are mirror/zero normal derivative; fixed nodes are
  Dirichlet. Cylindrical solver storage is **r,z**, unlike SIMION PA storage
  **axial x, radial y**. At the regular axis the radial coefficient is
  `4/h_r^2`, not planar `2/h^2`. A positive-origin annulus is supported.
  `docs/cpu-solver.md:8-53` ("Discrete operator");
  `bench/refine-real.md:29-34` ("Command and format contract").
- Symmetric correction matrix `A=-WL`; positive nodal quadrature weights,
  fine electrode correction zero, multilinear prolongation `P`, restriction
  exactly `P^T`, coarse `P^T A P` (27-point 3D, 9-point 2D).
  Endpoint-preserving coarsening retains thin/isolated electrodes.
  CPU: all hierarchy and operations f64, Rayon; Chebyshev-Jacobi,
  three pre/three post sweeps; absolute-row-sum coarse scaling; axes coarsen
  to at most four nodes, bottom at most 64 nodes/80 Jacobi sweeps.
  `docs/cpu-solver.md:47-95` ("Coarsening electrodes").
- Convergence: f64 infinity norm of **unweighted physical residual** on free
  nodes, V/length^2; gauge shift by first fixed voltage. Relative scale is
  infinity norm of the gauge-shifted reduced Dirichlet RHS, with a
  unit-voltage stencil fallback only for zero RHS. Test:
  `absolute <= absolute_tolerance + relative_tolerance*scale`.
  Default relative tolerance `1e-10`; residual is not continuum geometry
  error. `docs/cpu-solver.md:97-113` ("Convergence and reporting").
- GPU: f32 Galerkin correction V-cycles inside an f64 outer iterative
  refinement. Jacobi fine damping 2/3, coarse row-sum damping 4/3;
  three pre/post sweeps and 80 bottom sweeps. Vulkan f64 kernels use GLSL
  through Naga; only an eight-byte convergence scalar returns per cycle.
  Without `SHADER_F64`, CPU retains f64 iterate/residual and each cycle moves
  at least eight bytes/node. That fallback is correctness, not performance.
  `docs/gpu-solver.md:17-60` ("Numerical method").

### Strict real-array comparison

Conditions: same 259^3 shell PA (17,373,979 nodes), both PA_ and PA500
bases, native residual `1e-12`, SIMION `--convergence 1e-7`, CPU16 WSL,
RTX3070 Vulkan/native f64 residual, serial single runs. Solve column
**includes transfers**; do not call its ratio a pure-kernel ratio.
`bench/refine-real.md:15-18,163-185` ("End-to-end performance").

| Device | Setup ms | Solve including transfers ms | PA I/O ms | CLI total s | Process wall s |
|---|---:|---:|---:|---:|---:|
| CPU16 | 2376.460 | 5001.813 | 535.809 | 8.032514 | 8.05 |
| Vulkan | 828.705 | 736.262 | 593.353 | 2.194013 | 2.50 |
| SIMION strict | not separated | reported compute 14.23 s | not separated | not reported | 14.365 |

SIMION row: `bench/simion-baseline.md:20-21` ("Refine").
CPU/GPU rows: `bench/refine-real.md:163-178`.

| Array | CPU vs strict SIMION max/RMS V | Vulkan vs strict SIMION max/RMS V |
|---|---:|---:|
| PA0 | 2.124805718e-6 / 2.911887006e-7 | 2.141037243e-6 / 2.971483543e-7 |
| PA_ | 6.374417808e-5 / 8.735661893e-6 | 6.423112427e-5 / 8.914451521e-6 |
| PA500 | 6.360711359e-5 / 8.691120287e-6 | 6.327717892e-5 / 8.570496430e-6 |

Every node included, electrode-mask mismatches zero. Largest CPU/Vulkan
difference 5.718402463e-7 V. Fresh small einzel bases differ from strict
SIMION by maxima 1.345968485e-6 / 7.146203416e-7 / 4.539110741e-7 V.
SIMION default-to-strict large-basis difference reaches 0.9244163452 V.
`bench/refine-real.md:45-50,103-149` ("Potential differences").
SIMION rejects objectives below `1e-7`; a global `1e-7 V` agreement is
**not** proven. `bench/refine-real.md:135-159`.

**Supersession warning:** PLAN's 5.75x = 14.365/2.50 is a process-wall result.
`bench/refine-real.md:187-201` explicitly says the older default-SIMION
whole-command gate was not met. The later user rule in PLAN is compute-only.
Report the intervals above, not an unexplained universal 5.75x compute claim.

### Analytic GPU throughput (not the same shell geometry)

Windows RTX3070 Vulkan, native f64 residual, `1e-10`, default smoothers,
host32; one final run per size, driver cache exercised but fresh hierarchy/
pipelines each process. `docs/gpu-solver.md:100-140`
("Measured Windows Vulkan results").

| Cube nodes/side | Total nodes | Solve ms | Setup/upload/readback ms | Cycles/levels | Final relative residual |
|---|---:|---:|---:|---:|---:|
| 128 | 2,097,152 | 111.966 | 106.058 / 7.388 / 6.897 | 14/7 | 3.309081830e-11 |
| 256 | 16,777,216 | 340.229 | 97.336 / 92.856 / 57.073 | 14/8 | 3.350336127e-11 |
| 539 | 156,590,819 | 2359.281 | 767.558 / 1338.269 / 1246.943 | 12/9 | 7.405431823e-11 |

539^3 is the largest bindable cube in this representation; 540^3 exceeds
the 2,147,483,644-byte storage binding limit in its coarse matrix.
This is a binding-layout limit, not a general VRAM limit.
`docs/gpu-solver.md:120-140`.

### Continuous electrodes

Name: **symmetric linear ghost-fluid**, Gibou et al. equation (21), **not**
nonsymmetric quadratic/Shortley-Weller equation (20). Replace only a
cut link's `c(u_j-u_i)` with `(c/theta)(V-u_i)`; free/free couplings stay
unchanged, retaining weighted SPD and Galerkin energy. Conservative SDF
tracing can find metal between two vacuum endpoints; small cuts are handled
by fixing nodes within `2 h_min^2/L`, not constant-fraction theta clamping.
CPU f64 and GPU mixed precision use the same physical residual.
`docs/cutcell.md:39-108` ("Operator and symmetry invariant", "Roots").

Sphere radii 0.31/0.83, potentials 1/0 V, box [-1,1]^3; serial release
CPU16/RTX3070, `1e-10`, one run. Errors are V.
`docs/cutcell.md:121-151` ("Measured acceptance").

| Device/grid | Cut compute ms | Stair compute ms | Cut max V | Stair max V |
|---|---:|---:|---:|---:|
| CPU 128^3 | 238.368 | 219.064 | 6.711957e-4 | 3.991813e-2 |
| CPU 256^3 | 2228.011 | 2000.849 | 2.678142e-4 | 2.573109e-2 |
| GPU 128^3 | 89.484 | 64.665 | 6.711960e-4 | 3.991813e-2 |
| GPU 256^3 | 285.495 | 218.014 | 2.678142e-4 | 2.573109e-2 |

At CPU residual `1e-12`, n=65/129/257 potential orders across the full
ladder: sphere 2.214, worst-phase planar cylinders 1.918, axisymmetric
cylinders 1.991. Near-surface field orders: 1.362/1.020/1.000. These are
potential-second-order, **not** a second-order near-surface-field claim.
`docs/cutcell.md:153-190` ("Grid convergence").

Equal-memory comparison counts sparse cuts and coarse solver buffers,
not only fine node count; final readback staging shown separately.
`docs/cutcell.md:192-220` ("Equal memory budget").

| Solve | Compute MiB | Including staging MiB | Max error V |
|---|---:|---:|---:|
| Cut 128^3 | 87.35 | 103.35 | 6.711960e-4 |
| Stair 129^3 | 85.48 | 101.86 | 5.231624e-2 |
| Cut 256^3 | 679.06 | 807.06 | 2.678142e-4 |
| Stair 257^3 | 671.44 | 800.94 | 2.241964e-2 |

Larger staircase grids fit within the cut budgets but have 77.94x/83.71x
larger error. This is explicit GPU-buffer accounting, **not process RSS**.

## 4. Flight

### Numerical contract

- Public units: mm, microseconds, V, V/mm, gauss, amu, elementary charge, eV.
  CPU state/stages/fields f64; compensated TOF separate from birth time.
  RK45 is **Dormand-Prince 5(4)** with separate position/physical-velocity
  absolute tolerances and shared relative tolerance; Boris is midpoint
  drift/rotate/drift, second order, fixed requested step shortened by limits.
  Relativistic mode evolves proper velocity `u=gamma*v`; CLI enables it by
  default, API default is nonrelativistic. Collisions require nonrelativistic
  dynamics. `crates/simrep-fly/README.md:3-7,70-147,246-259`;
  `crates/simrep-fly/src/flight.rs:440-636`;
  `docs/python.md:68-81`.
- Current defaults: four smallest physical cells of transit, local
  cyclotron angle <=0.2 rad, voltage phase <=0.2 rad; acceleration limits
  launches from rest, placement scale is included. Numeric tolerances are
  `1e-12`. Static-electric launch estimate clamps tolerance to
  `[1e-12,1e-8]`; magnetic/time-varying/zero-field/missing-scale/rest cases
  retain `1e-12`. An explicit ceiling or tolerance disables this automatic
  relaxation. Coupled space charge retains an extra one-cell ceiling.
  `bench/default-flight.md:100-174` ("Policy and tolerance selection").
- Electrode-containing grids use boundary-aware vacuum runs, one-sided
  clamped cubic estimates of shared nodal first/second derivatives, averaged
  mixed jets, tensor **quintic Hermite** patches. Potential C2, E C1 across
  vacuum cell faces; node values exactly interpolated; Horner evaluation.
  Electrode-free arrays retain interpolating tensor cubic B-splines.
  `bench/fly-real.md:276-300` ("Boundary-aware correction");
  `crates/simrep-fly/src/field/boundary.rs:1-149`.
- Staircase absorption is an **all-corner occupied cell** (four corners in
  active 2D, eight in 3D), not a half-cell voxel around the nearest node.
  Segments are partitioned across faces/mirrors/cylindrical shells to prevent
  tunneling; nearest-node remains an explicit historical option.
  `crates/simrep-fly/README.md:148-166`;
  `docs/dev/frontend.md:289-296` ("이온 비행과 전압 슬라이더").
- V3 `.cuts.json` retains exact source GEM CSG or closed mesh triangles.
  Default source-only files use lossless node runs/sparse links; dense
  fallback is opt-in. V1/V2 are explicitly approximate. Mesh intersections
  are exact **to stored triangles**; CAD flight uses fine tessellation, not
  live OCCT B-rep queries. Exact surfaces force CPU; field reconstruction is
  unchanged by the collision choice.
  `docs/geometry-pipeline.md:90-192` ("Outputs and flying");
  `crates/simrep-fly/src/surface.rs:1-2,221-392,428-446`.

### Einzel: separate historical, corrected-default and native GPU rows

Physical inputs: original 91x20 cylindrical PA, 1 mm; 100 amu/+1e/200 eV,
x=z=0, initial +x, uniform y=-15..15 mm; electrodes 0/110/0 V,
relativistic; workbench x=-101..291, y/z=-19..19 mm.
`bench/fly-real.md:27-63` ("Input and placement").

| Measurement | Population | Compute s | Accuracy/conditions |
|---|---:|---:|---|
| SIMION baseline default quality | 10,000 | 2.13 | wall 2.201 s, retained trajectories |
| SIMION baseline default quality | 100,000 | 21.37 | wall 21.504 s |
| Historical boundary-aware CPU | 10,000 | 0.358728 | prior CLI tolerances, total 0.382090 s |
| Historical boundary-aware CPU | 100,000 | 3.688588 | total 3.810812 s |
| **Current default CPU16** | 10,000 | **0.184954 median** | five fresh serial runs, all-corner, four cells/cap 1e-8 |
| Historical f32 GPU submission clock | 10,000 | 0.007983 | includes combination, not timestamp-only; requested 1e-8/effective floor 2e-7 |
| Native f64 GPU timestamp | 100,000 | 0.637090 | four bounded batches, all physically splat |
| Native f64 GPU timestamp | 1,000,000 | 6.263721 | 31 bounded batches, all physically splat |

Sources: `bench/simion-baseline.md:25-37` ("Fly");
`bench/fly-real.md:333-348` ("Matched coarse-grid speed");
`bench/default-flight.md:3-44,94-98` ("Main integration");
`bench/fly-gpu.md:978-999,1170-1214` ("Native precision study",
"Historical f32 Windows Vulkan release timing"). The f32 row is historical,
not evidence that the precise GPU flyer currently takes eight milliseconds.

Against the preserved independent 8x spatial reference, same 1,000-ion subset:

| Method | Endpoint RMS mm | Max mm | TOF RMS us |
|---|---:|---:|---:|
| Old global spline | 5.895332 | 17.154020 | 0.297908 |
| SIMION T.Qual 209 | 1.106314 | 4.281588 | 0.055319 |
| Boundary-aware, tight preserved CPU | 0.225707 | 1.323463 | 0.010981 |
| **Current default CPU** | **0.226123540** | 1.323331557 | 0.0109990036 |
| Native f64 GPU | 0.225702 | 1.324228 | not tabulated vs reference |
| Historical f32 GPU | 0.224607 | 1.327079 | not tabulated vs reference |

Sources: `bench/fly-real.md:302-331`;
`bench/default-flight.md:15-42`;
`bench/fly-gpu.md:964-976`. Current CPU adds 0.1843% RMS but stays inside
its documented margin. Boundary-aware 1x/2x/4x observed endpoint orders
0.620/1.619 are **not fourth order** near electrodes. Reference is numerical,
not a certified continuum bound.

Strict large-N SIMION timings are **projected** from measured T.Qual209
1,024-ion compute 7.88 s: 769.531 s at 100k and 7,695.313 s at 1M.
Ratios 1,208x/1,229x are projected strict-comparison ratios, not measured
large SIMION runs. `bench/fly-gpu.md:982-1018`.

### Quadrupole: raw PA versus physical geometry

Same 1,000-ion disk, coarse 121x37x37 grid at h=0.5 mm, electrodes
2.5/-2.5/0 V, bounds [0,-18,-18]..[60,18,18] mm.
CPU16 f64; physical route tolerance `1e-11`, max dt 0.025 us.
`bench/fly-real.md:367-411,676-704`;
`docs/geometry-pipeline.md:311-335`.

| Route/reference | Endpoint RMS mm | Exit-class changes |
|---|---:|---:|
| Coarse CPU all-corner vs node-cell local16x | 1.023402 | 22 |
| SIMION T.Qual209 vs same node-cell reference | 1.023425 | 22 |
| Coarse CPU all-corner vs **physical** 8x | 2.382840 | not in this row |
| SIMION T.Qual209 vs **physical** 8x | 2.382644 | not in this row |
| Analytic cut-field/exact-SDF control vs physical8x | 0.00343530 | 0 |
| **Real V3 refine-geometry -> fly --cuts** | **0.00343534** | **0** |

`bench/fly-real.md:442-511,676-704`.
V3 max 0.02018961 mm, TOF RMS/max 0.000185835/0.001466332 us.
Physical4x/8x endpoint RMS change 0.000696157 mm.
The approximately 694x physical accuracy ratio is not an interpolation-only
comparison: the geometry/solve/collision model also improves.
Old V2 0.19562950 mm/one changed exit is superseded; CSG corner loss was
the cause, not insufficient uniform supersampling.

Compute only, serial CPU16, setup/output excluded:
coarse cell-default CPU 0.065829 s, analytic cut-control 0.068113 s,
SIMION T.Qual3 0.100000 s, T.Qual209 9.280000 s.
Raw PA CPU is 1.52x faster than T.Qual3 and 141x at matched T.Qual209
accuracy. `bench/fly-real.md:513-533` ("Compute time at achieved precision").
Do not reuse the old GPU 54.04 ms nearest-node result for this geometry.

Real V3 user route's five-run interleaved CPU16 median compute is
0.130802 s vs no-cuts 0.097657 s (1.339x); setup 0.636995/0.208365 s.
Einzel+36 neutral impacts: 1,036 ions, 0.156970/0.103468 s (1.517x);
slightly above its approximate 1.5x target.
These retained timings load dense fallback and are not newly timed
source-only sidecar costs. `docs/geometry-pipeline.md:574-595`.

### Latest GPU Horner specialization

Windows RTX3070 Vulkan; same solved templates, fixed factors [1,1.05,0,0],
300 sampled trajectories, five warmups/candidate/population, three measured
flights, order alternated, no GPU use before bounded runs. **Compute pass
timestamps only**; excludes combination/upload/readback/compilation.
All final states, TOFs, exact hit classes/indices, steps and trajectory
values replay bitwise. Explicitly unrolls the six x-Horner iterations;
no reconstruction/integrator/tolerance change.
`bench/fly-gpu.md:7-52` ("Cached-power Horner specialization").

| Precision | Template | Ions | General ms | Specialized ms |
|---|---|---:|---:|---:|
| f64 | einzel | 10,000 | 24.577984 | 23.697152 |
| f64 | einzel | 100,000 | 201.912416 | 194.637760 |
| f64 | quadrupole | 10,000 | 72.957376 | 59.522912 |
| f64 | quadrupole | 100,000 | 558.632704 | 490.027552 |
| f32 Preview | einzel | 10,000 | 3.470048 | 2.334016 |
| f32 Preview | einzel | 100,000 | 19.766240 | 13.209216 |
| f32 Preview | quadrupole | 10,000 | 19.192544 | 9.940032 |
| f32 Preview | quadrupole | 100,000 | 99.749952 | 54.886016 |

Hardware occupancy/register/spill/stall counters remain unmeasured;
generated SPIR-V/source proxies are not driver register allocation.
`bench/fly-gpu.md:88-123` ("Phase replay").

### App live update evidence and auto selection

Newest **app-measured** rows: release app + production React, RTX3070,
1440x900, 300 drawn trajectories, 1e-4 V increments, five-second drags,
three-drag medians per execution, three before/after executions alternated.
Shared-machine caveat: two live-parent conhosts used about four cores each;
rAF varied 32-60 Hz, constraining the fastest einzel case.
`docs/dev/frontend.md:425-447` ("제출/수거 파이프라인과 작은 비행 패킷").

| Template/ions | Before updates/s | Latest updates/s | Round trip ms | Native/compute/host transfer ms |
|---|---:|---:|---:|---:|
| einzel 10k | 39.1 | 50.3 | 28 | 11.6 / 6.0 / 3.9 |
| einzel 100k | 13.5 | 17.7 | 144 | 70.7 / 32.9 / 18.8 |
| quadrupole 10k | 30.9 | 39.5 | 66 | 30.4 / 19.4 / 3.2 |
| quadrupole 100k | 7.6 | 8.6 | 309 | 262 / 104 / 16.1 |

Earlier approximately 8/s quad10k was a CDP rounding artifact; later
53.9/12.6/37.8/5.6 and 55.4/16.4/43.6/7.2 tables use different
pipeline/environment generations. Do not mix them.
`docs/dev/frontend.md:360-417`.
**No app remeasurement after the 54.886016 ms kernel result is documented**;
the newer kernel result does not itself establish app updates/s.

`--device auto`: calibrated electric-only hardware-f64 class selects GPU at
N>=4,096; tested CPU/GPU compute at N=1,024:13.970/43.720 ms,
2,048:27.800/44.475 ms, 4,096:58.734/51.278 ms.
This brackets a measured crossover, not a universal count.
No adapter/software/no-f64 -> CPU.
`bench/fly-gpu.md:1032-1066`.
Default **field-free HS1 only** uses RTX3070 native-f64 GPU at N>=10,000
under the exact calibrated 50 us/0.05 us profile; other HS1/SDS/magnetic
classes retain CPU. `bench/fly-gpu.md:550-574`.
PIC auto threshold N>=100,000; exact surfaces, direct Coulomb, beam,
vector potentials/material magnetic collisions are CPU contracts.
`bench/space-charge.md:330-353`;
`docs/magnetostatics.md:57-61`; `docs/geometry-pipeline.md:191-192`.

## 5. Advanced physics

### Magnetic flight and coils

Magnetic scalar PA: `B=-ng grad(phi)` in unscaled PA physical coordinates;
placement rotates but does not rescale strength. Electric/magnetic placements
may differ. Uniform B, finite straight current segments, circular hoops
(AGM elliptic K/E, analytic axis), and solenoid sums are implemented.
Infinitesimal filaments have nonfinite on-wire fields.
`docs/physics-magnetic-collision.md:110-150` ("Units and magnetic fields").

Prepared CPU solenoid map: C2 axial B and Br/r, h=0.0625 mm,
3,386,571 nodes/~488 MB f64 jets, three filament-free rectangles;
512 deterministic off-node probes/rectangle. Acceptance requires sampled
max <=2e-9 G and relative <=5e-10; exact fallback outside accepted regions.
These are sampled residuals, not global bounds.
`bench/magnetic-cpu.md:71-98` ("Implementation and profile").

| Region | CPU map maximum sampled absolute G | Relative |
|---|---:|---:|
| inner tube | 1.35616449e-9 | 2.71637035e-10 |
| entrance | 4.56778840e-12 | 9.93027565e-12 |
| exit | 2.46870272e-12 | 7.54533044e-12 |

Latest seven-case CPU acceptance: 5950X WSL release f64/16 prepared Rayon
workers; original captured starts repeated to resolve SIMION's 0.01 s clock.
SIMION Windows/one worker/no trajectories, three-run medians; Rust five-run
medians, strict unchanged magnetic tolerances `1e-12`, automatic ceilings.
Loading/reconstruction/map calibration/pool creation/output excluded.
`bench/magnetic-cpu.md:1-39` ("Actual matched-population compute").

| Case | Ions | SIMION s | CPU s | Ratio |
|---|---:|---:|---:|---:|
| magnet1 | 1,000 | 0.35 | 0.165596638 | 2.11 |
| mag90 | 1,000 | 0.25 | 0.178162641 | 1.40 |
| sector2dp | 150 | 0.24 | 0.032743208 | 7.33 |
| sector2dc | 500 | 0.23 | 0.024820506 | 9.27 |
| sector3dp | 150 | 0.17 | 0.062724102 | 2.71 |
| Helmholtz | 270 | 0.60 | 0.012323900 | 48.69 |
| mapped solenoid | 100 | 4.77 | 0.221888623 | 21.50 |

All beat fresh SIMION medians without relaxing the old API accuracy gates.
Reference is exact Biot-Savart/native PA, dt<=0.0025 us, tolerances `1e-12`,
same physical termination reasons.
`bench/magnetic-cpu.md:53-69` ("Unchanged accuracy gate").

| Case | CPU max endpoint mm | Gate mm | CPU max TOF us | Gate us |
|---|---:|---:|---:|---:|
| magnet1 | 1.27745e-8 | 5.47282e-7 | 7.35311e-9 | 4.01298e-7 |
| mag90 | 5.63213e-9 | 2.54520e-7 | 3.44915e-11 | 2.31283e-9 |
| sector2dp | 2.69811e-9 | 1.85622e-7 | 6.55120e-11 | 7.11352e-9 |
| sector2dc | 2.66234e-9 | 3.01237e-7 | 3.00506e-11 | 3.69093e-9 |
| sector3dp | 7.90770e-8 | 9.77063e-7 | 3.70319e-8 | 4.95515e-8 |
| Helmholtz | 1.46264e-13 | 1.46092e-8 | 0 | 2.69845e-11 |
| solenoid | 4.28512e-7 | 1.22619e-6 | 3.93079e-10 | 1.30382e-9 |

Mag90 comparison-only relaxed config `1e-6`, dt<=1 us:0.042293600 s,
0.00337398 mm/4.20225e-5 us max, 5.91x SIMION, but fails the stricter
old-API gate and is **not a default**.
`bench/magnetic-cpu.md:120-141`.
Mapped solenoid actual CPU16 medians at 1k/10k:1.767366474/17.261339962 s,
not extrapolations; do not pair them as an isolated OS comparison with the
older Windows controls. `bench/magnetic-cpu.md:143-156`.

GPU magnetic **accuracy passes but no magnetic crossover**: native-f64
RTX3070 three-run compute medians at 1k/10k sector3dp GPU
0.895151840/4.031644832 s vs older Windows CPU16
0.118082800/1.144121800 s; mapped solenoid GPU
52.297237440/188.427607616 s vs CPU 6.056711600/59.597406400 s.
Those CPU rows precede the latest CPU optimization above.
`bench/fly-gpu.md:446-510,550-622`.
Projected 100k/1M magnetic times must remain labelled projections;
native-f64 arithmetic/occupancy is a hypothesis from step/resource evidence,
not measured hardware counters.

Imported external B-map document `docs/bfield-import.md` **does not exist**
in main at this audit. Do not claim that separate pending work is delivered.

### Neutral gas: HS1 and SDS

HS1 follows shipped Lua's mean-relative-speed collision rate, weighted thermal
gas rejection, elastic COM scattering. Native uses exponential optical-depth
clock and 20 subdivisions/mean-free-time by default. SDS follows the empirical
five-ICDF `mbmr.dat` model, log/log mass interpolation and isotropic position
jumps, **not Gaussian diffusion**; deterministic Stokes drag uses exact
constant-E/B damped rotation plus midpoint step doubling. `mbmr.dat` is a
runtime dependency, not bundled. Nonrelativistic only.
`docs/physics-magnetic-collision.md:152-244` ("HS1", "SDS").

SIMION/native independent samples each N=4,096, native seed813/SIMION
same-process seed237, duration50 us, mass200 amu/+1. HS1 initial0.1 eV +X,
helium4 amu/0.53 Pa/273 K/sigma2.27e-18 m^2; SDS rest, air760 torr/
273.15 K, one50 us diffusion step.
`docs/physics-magnetic-collision.md:297-358` ("Statistical results").

| Model | X/Y/Z exact KS p | KE p | TOF p |
|---|---|---:|---:|
| HS1 vs SIMION | 0.515856 / 0.384564 / 0.977716 | 0.384564 | 1 |
| SDS vs SIMION | 0.699440 / 0.882848 / 0.355182 | 1 | 1 |
| HS1 CPU/GPU, seed813 each | 0.0987632 / 0.276109 / 0.264260 | 0.128702 | 1 |
| SDS CPU/GPU | 0.569700 / 0.868785 / 0.431352 | 1 | 1 |

CPU/GPU conditions: Windows native-f64 RTX3070, same input profiles, N=4,096
per implementation. `docs/physics-magnetic-collision.md:82-108`.
No common-distribution rejection at5%; not proof of identity. TOF and SDS
zero-field KE are degenerate atoms, not stochastic transport evidence.
HS1/SDS retained CPU compute0.303679/0.006252 s vs SIMION9.96/1.23 s;
thread count in this table is not stated (Rayon used).
`docs/physics-magnetic-collision.md:429-454`.
Calibrated field-free HS1 GPU1k/10k medians0.129540192/0.436820416 s vs
CPU16 0.053331800/0.546073900 s.
`bench/fly-gpu.md:512-574`.

### Linear dielectric

Positive scalar epsilon, `-div(epsilon_r grad V)=rho_free/epsilon0`.
Nodal epsilon -> harmonic link mean; SIMION cell epsilon -> arithmetic
transverse area-average conductances. Weighted symmetry/Galerkin retained.
Curved sampled interfaces generally first order; no continuous electrode
cut-cell combination in a material solve.
`docs/dielectric.md:3-15,113-147`.

Default residual1e-10, three fresh solves/device, Windows Vulkan RTX3070;
CPU thread count not stated. These are **contended phase medians/sums**,
not isolated whole-execution medians.
`docs/dielectric.md:258-296` ("Automatic selection").

| Case/device | Selected | Solve s | Hierarchy s | Material prep once s | Strict SIMION median s |
|---|---|---:|---:|---:|---:|
| plates2dp CPU | yes | 0.053610 | 0.003050 | 0.008300 | 0.13 |
| plates2dp GPU | no | 0.077542 | 0.010408 | 0.075188 | 0.13 |
| sphere2dc CPU | yes | 0.161406 | 0.013060 | 0.009803 | 0.43 |
| sphere2dc GPU | no | 0.141039 | 0.048220 | 0.386761 | 0.43 |
| sphere3dp CPU | no | 1.501676 | 0.113046 | 1.859506 | 25.78 |
| sphere3dp GPU | yes | 0.286475 | 0.174082 | 0.640721 | 25.78 |

Selected preparation+hierarchy+solve phase sums:0.064960/0.184269/
1.101278 s, all below strict SIMION; including3D transfers gives1.414065 s.
At stricter native1e-12, GPU solve medians0.164317/0.260937/0.940902 s;
small plate GPU solve loses to0.13 s SIMION. Material/hierarchy work cannot
be omitted from one-shot compute. `docs/dielectric.md:199-256,279-295`.

### Nonlinear dielectric: numerical versus physical verdict

Relaxed Picard with epsilon(|E|), CPU f64/GPU mixed inner solve;
growing nonlinear residual halves relaxation down to1/128.
Convergence requires physical nonlinear residual **and** vector-field change.
No nonlinear fast-adjust bases.
`docs/dielectric.md:310-327,359-381`.

Shipped grid201x101x101, h0.5 mm, sphere radius10 mm, plates10 MV/0 V;
CPU4, residual/field tolerances1e-10, whole coefficient+hierarchy+solve
compute, three fresh runs. Shared host contention explicitly retained.
`docs/dielectric.md:427-469` ("Nonlinear sphere results").

| Solver | Median compute s | Error vs own converged discrete model V |
|---|---:|---:|
| SIMION20 updates/0.1 V objective | 87.494 | 291.098 |
| CPU f64 | 15.189213 | 4.03011e-5 |
| Vulkan | 12.202519 | 3.15160e-5 |

The 5.76x/7.17x ratios above are **not matched physical accuracy**.
Converged models differ max424.769 V on1x because SIMION's constitutive
field uses half-grid differences and native uses containing-cell derivatives.
Grid refinement shrinks max differences1081.338/624.911/424.769/226.084 V
(h2/1/0.5/0.25 mm); extrapolated limits support a common continuum,
not a persistent424.8 V physical-model difference.
Orders are not stable enough for clean second-order Richardson.
`docs/dielectric.md:484-616` ("Physical grid-refinement audit").

**Latest matched estimated physical accuracy:** all four fixed potential
probes, center E, induced dipole and excess plate charge must be no worse
than SIMION shipped1x default. No tested native1x tolerance passes all;
native2x at residual/field1e-5 is the cheapest tested passing configuration.
Not a certified global continuum norm.
`docs/dielectric.md:618-667` ("Speed at matched estimated physical accuracy").

| Configuration | Center E error V/mm | Probe A/B/C/D errors V | Dipole error C mm | Excess-charge error nC | Median s |
|---|---:|---|---:|---:|---:|
| SIMION1x default | 834.283 | 2197.204 / 3914.693 / 3958.935 / 804.187 | 1.62070e-7 | 2.42048 | 87.494 |
| Native2x CPU4 | 415.704 | 1007.374 / 2089.326 / 2115.542 / 481.172 | 8.10736e-8 | 0.84114 | 43.636006 |
| Native2x RTX3070 | 413.942 | 1025.477 / 2059.230 / 2097.689 / 468.964 | 8.09659e-8 | 1.02216 | 48.368654 |

Thus2.01x CPU/1.81x GPU at matched estimated observables; old ratios do not
survive this stricter test. GPU triplet overlapped compilation/testing and
another Windows computation; CPU/GPU relative ordering is not a crossover
calibration. No4x/8x physical result: memory/time budgets prevented them.
`docs/dielectric.md:531-536,642-667`.

### Space charge: factor, Coulombic, PIC and beam

Synchronized ensemble uses equal-time midpoint drift/Boris/drift; factor and
Coulombic represented charge differ from physical q/m. Births and losses split
the global step; no renormalization after loss. Direct O(N^2) sum excludes
self, serial for live N<256 and target-parallel above; zero softening is exact
point Coulomb, positive is Plummer. Direct fields omit electrode images.
PIC CIC deposit/central-difference E/CIC gather solves grounded correction
Poisson and includes images appropriate to those boundaries.
`docs/physics-space-charge.md:9-150`.

Original103 captured starts, zero external PA, bounds[-10,-4,-4]..[10,4,4] mm,
nonrelativistic. Coulombic total absolute charge3.47e-13 C; independent
DOP853 reference resolves births and outward exits. CPU4 WSL release.
Current automatic-default compute/errors:
`bench/space-charge.md:3-32,72-132`;
`bench/default-flight.md:64-72`.

| Mode | Current default endpoint RMS mm | SIMION endpoint RMS mm | CPU compute s | SIMION compute s |
|---|---:|---:|---:|---:|
| factor1 | 3.81460407e-7 | 0.010609200 | 0.279076 | 0.91 |
| Coulombic | 1.01389188e-4 | 0.590059409 | 0.248803 | 0.93 |

Compute/current errors: `bench/space-charge.md:8-13`;
SIMION errors: `bench/space-charge.md:90-100`.
Configured Coulombic dt1e-4/5e-5/2.5e-5/1.25e-5 us gives native endpoint RMS
4.863959e-5/1.284997e-5/3.335221e-6/8.456660e-7 mm and
compute0.267262/0.369717/0.468491/0.759510 s; second-order error ratios
3.785/3.853/3.944. Better point-charge accuracy and faster than SIMION,
**not private-cloud equivalence**. `bench/space-charge.md:90-132,142-169`.

Algorithm probe10k cold sphere/10 global steps atdt0.001 us, Q1e-14 C,
mass100 amu/+1, PIC33^3/h0.25 mm, tolerance1e-8:
direct5.964276306 s vs CPU PIC0.246605100 s (24.2x).
Not a SIMION comparison. `bench/space-charge.md:191-204`.

GPU PIC: native-f64 RTX3070,100k identical launches,33^3, Q1e-15 C,
dt0.0002 us/duration0.0006 us, CPU4, Poisson1e-11. Three-run medians
CPU0.4073832/GPU0.2895435 s (1.407x); max endpoint disagreement
3.33067e-16 mm. Includes every in-flight transfer/synchronization/deposit/
solve/gather/push/event, excludes prepared hierarchy/shader allocation.
CPU rebuilds hierarchy per force evaluation; GPU reuses it.
`bench/space-charge.md:314-353`.

Beam is a separate **SIMION-documented semantics** contract: first particle
defines leader-normal plane, followers have unequal times; explicit infinite
parallel-line or azimuthal-ring kernel, static external fields, no private
cloud fit or iterative steady-state solution.
`docs/physics-space-charge.md:181-298`.
Latest final CPU timing is serialized with heavy jobs paused, three fresh
medians, CPU thread count not restated in final section; compute only.
Same103/201 recorded trajectories, I1e-5 A, zero PA, nonrelativistic,
SIMION T.Qual=-9. `bench/space-charge.md:368-392,538-576`.

| Case/control | Endpoint RMS/max mm | TOF RMS/max us | Exit disagreements | Compute s |
|---|---|---|---:|---:|
| repulsion1 SIMION | 0.304992 / 1.104812 | 4.30555e-4 / 1.77517e-3 | 4 | 0.86 |
| repulsion1 native defaults | 0.00569009 / 0.0170960 | 8.90735e-6 / 2.60387e-5 | 0 | 0.269405 |
| mixed SIMION | 0.908870 / 3.637525 | 1.19347e-4 / 1.53363e-3 | 2 | 4.21 |
| mixed native defaults | 0.408970 / 2.402977 | 6.01615e-6 / 8.52936e-5 | 0 | 2.116568 |

Verdict3.19x/1.99x faster at equal-or-better errors against converged stated
line model. Mixed production scattering is **not individually converged**:
tighterdt1e-6/fraction0.05 gives0.0627212 mm RMS at44.478372 s.
Private-cloud discrepancy remains separate; cannot subtract RMS errors to
attribute it. `bench/space-charge.md:433-460,506-576`.

### Magnetostatics

Scalar p/magnetization/permeability, planar Az/current, axisymmetric
psi=r*Aphi, and Cartesian vector A are implemented. Cartesian A requires
spatially uniform mu; no nonlinear B-H. B comes from curl of reconstructed
C2 potential; regular-axis f=psi/r^2 avoids division/cancellation.
Magnetic material sidecars mark solid cells. Vector/material flight is CPU.
`docs/magnetostatics.md:3-61` ("Equations and flight contract").

Analytic tests at physical residual1e-12 use cell-volume-averaged sources:
finest potential orders1.35332 cylinder M,1.31904 sphere M,
2.13212 wire Az,2.16354 coax Az,0.96620 mu20 curved shield,
2.00186 smooth psi. Curved materials remain first order.
`docs/magnetostatics.md:177-206`.

Installed audit:20 PDE stems,25 component/scalar solves,23 combined-field
flights. All25 candidate potential errors no worse than SIMION **default**;
not every strict control. Candidate1e-10 (uniform precision1e-15);
CPU1e-12 reference, special f64-floor agreement control. Three phase medians
summed (hierarchy+iteration), source/material prep/I/O/reconstruction excluded.
Host thread count is not stated; **contended**, not quiet-machine acceptance.
`docs/magnetostatics.md:208-284`.

| Case/field | CPU setup+solve s | SIMION default s | Strict s | Default/CPU |
|---|---:|---:|---:|---:|
| c_magnet2dp/Az | 0.310451 | 4.206 | 11.726 | 13.55 |
| current_cylinder2dp/Az | 0.007747 | 0.180 | 0.591 | 23.23 |
| current_sphere2dc/psi | 0.015866 | 0.091 | 0.188 | 5.74 |
| current_sphere3dp/Axyz | 0.421483 | 2.026 | 5.479 | 4.81 |
| emag_quad2dp/Az | 0.064575 | 0.431 | 0.867 | 6.67 |
| mag_2cylinder2dc/p | 0.130068 | 0.822 | 7.713 | 6.32 |
| mag_2ring2dc/p | 0.115160 | 0.697 | 7.695 | 6.05 |
| mag_cylinder2dc/p | 0.061099 | 0.370 | 1.267 | 6.06 |
| Halbach2dp/p | 0.006907 | 0.155 | 0.225 | 22.44 |
| mag_quad2dp/p | 0.460006 | 1.155 | 20.671 | 2.51 |
| mag_ring2dc/p | 0.108884 | 0.654 | 7.056 | 6.01 |
| mag_sphere2dc/p | 0.032802 | 0.228 | 0.641 | 6.95 |
| mag_sphere3dp/p | 1.759477 | 7.619 | 51.867 | 4.33 |
| mu_cylinder2dp/p | 0.048512 | 0.173 | 3.685 | 3.57 |
| mu_cylinder2dp/Az | 0.041684 | 0.188 | 2.186 | 4.51 |
| mu_sphere2dc/p | 0.072604 | 0.139 | 1.954 | 1.91 |
| mu_sphere2dc/Az | 0.131499 | 0.237 | 1.261 | 1.80 |
| mu_sphere3dp/p | 0.490606 | 0.824 | 11.213 | 1.68 |
| mu_torus3dp/p | 0.312333 | 0.981 | 5.663 | 3.14 |
| mucurrent_cylinder2dp/Az | 0.096310 | 1.006 | 12.292 | 10.45 |
| solenoid2dc/psi | 0.049384 | 0.131 | 0.201 | 2.65 |
| **uniform2dp/Az** | **0.131731** | **0.024** | 1.654 | **0.18** |
| uniform2dp/p | 0.102812 | load only | load only | not timed |

`docs/magnetostatics.md:286-315` ("Audit results"). Nonuniform timed
controls win1.68-23.23x; uniform Az loses~5.49x, so no universal claim.
Uniform Az candidate error1.42e-14 vs default3.39e-10, strict3.42e-7
stored-potential units; all retain original ng.
Candidate/converged flights max endpoint5.999e-11 mm, velocity1.203e-9 mm/us,
relative KE6.44e-15 (duration0.1 us/dt0.001 us).
SIMION strict B-probe differences include17.80 G emag_quad,14.74 G
c_magnet,1.0035 G solenoid: interpolation/interface effects, not residual.
Conservative M is not the literal native Lua-derived source; max p
differences up to3.43282e-2 for Halbach. `docs/magnetostatics.md:317-342`.

## 6. Compatibility and scientific import/export

- PA/PA#/PA0: x-fastest, electrode flags and symmetry preserved;
  adjustable integer labels1..1000 create10,000 V bases; scalable PA_
  normalization and PA0 reference/trailer retained. Fast adjust adds
  `(requested-current)/reference * basis`, preserving unspecified voltages/
  background. Existing fast-adjusted PA0 cannot be compared directly to a
  zero-adjustment fresh refine. `bench/refine-real.md:20-34,95-101`;
  `bench/fly-real.md:27-38`.
  SIMION itself loaded generated PA0 and adjusted both electrode500 and
  scalable slot0; observed123/42 V at asserted nodes.
  `bench/refine-real.md:232-238`.
- GEM: traditional/modern syntax, transformations/CSG, includes, ordered
  overwrites, Lua5.1 statements/functions/loops/callbacks; bounded vendored
  Lua5.1.5 with256 MiB/five-billion instructions. No workbench Lua runtime.
  Fractional `.pa-surf` SIMEXP/FHDR v1 edge records are separately checked.
  `docs/gem.md:17-108`.
  **Latest corpus:**261 installed GEM files ->254 standalone matches
  (including two normalized-oracle PA declarations), seven include fragments,
  zero unsupported. Earlier211 passes/50 unsupported is superseded.
  Fractional surfaces show zero grid/record mismatches for18 listed cases.
  Performance rerun contended; native compute versus SIMION process wall
  is not matched compute. `docs/gem-validation.md:3-27,328-392`.
- FLY2: executable particle-source Lua in bounded shared sandbox; sequences,
  distributions, custom functions/coroutines, read-only whitelisted includes,
  placement/field queries with explicit context. No binary FLY/native
  extensions/workbench callbacks/PA mutation. ION retained as explicit source;
  PA-coordinate output is transformed to physical workbench units.
  `docs/fly2.md:26-140`.
  **Latest154 files:**97 ordered matches,57 statistical matches,
  zero mismatches/unsupported. Random seeds1..8, KS Bonferroni family
  alpha0.01 over154x20 tests; deterministic tolerance2e-11 relative/
  1e-13 absolute, CWF absolute1e-24. No flights in this source-format oracle.
  `docs/fly2-validation.md:1-48`.
- Mesh import: STL ASCII/binary, OBJ, PLY1.0 ASCII/little/big endian,
  3MF core, glTF2/GLB; mm internal, glTF source meters,3MF declared units;
  parts/transforms retained. Safe repair handles duplicate/degenerate faces,
  welding/winding, not invented hole filling.
  `docs/mesh-io.md:8-79`.
  Shipped two-cylinder STL is defective; strict importer rejects it.
  Explicit defective-surface oracle31x21x21/h1 mm, translation(0,10,10):
  13,671 nodes/2,640 metal nodes, **zero mask mismatches**. This is
  voxelization, not fractional refine/flight. `docs/mesh-io.md:245-266`.
- Imported sphere/cylinder STL at radii0.31/0.83 mm,1/0 V,
  CPU16 release,65^3/65^2: cut max0.00479243/0.00081280 V vs
  staircase0.10487824/0.07394388 V;21.9x/91.0x improvement;
  cut compute47.096/2.330 ms vs39.357/2.358 ms.
  `docs/geometry-pipeline.md:641-653`.
- STEP/IGES: optional dynamic OCCT7.6/7.7; STEP AP203/AP214/geometric
  AP242 subset and IGES solid/closed-shell B-rep. XCAF names/assemblies/
  units/located bodies retained. Exact distance mode, tessellated mode, or
  auto(valid exact/invalid closed-mesh fallback). Default chord tolerance
  h_min/100 mm, angular0.35 rad. Complete restart on query failure,
  not pointwise mixed surfaces. `docs/step-import.md:3-27,70-106`.
  Same-grid CPU16 release/single serial shared-host runs, residual1e-11,
  mesh deflection0.02 mm:

| Shape/grid | Exact RMS/max V | Tessellated RMS/max V | Exact/mesh prep ms | Exact/mesh compute ms |
|---|---|---|---|---|
| sphere33^3 | 0.001348309 / 0.012769231 | 0.005193689 / 0.032344199 | 3047.130 / 31.004 | 47.677 / 49.763 |
| cylinder65^2 | 0.000221849 / 0.000797456 | 0.001330479 / 0.003756581 | 464.119 / 9.886 | 4.005 / 4.825 |
| ring25^3 | 0.003825231 / 0.040355959 | 0.005792104 / 0.032102006 | 6384.288 / 68.335 | 34.857 / 30.863 |

`docs/step-import.md:190-223` ("Measured accuracy and timing").
Exact RMS wins3.85/6.00/1.51x; ring mesh max happens to be smaller.
Exact imported fields match same-grid analytic SDF within
8.88e-16/2.88e-12/5.87e-12 V. B-rep prep is much slower.
- Surface exports: STL(binary f32), OBJ groups, embedded glTF/GLB;
  PA-derived surfaces retain staircase geometry, do not recover analytic CAD.
  Grid VTI or revolved cylindrical VTS; trajectories CSV/VTP; isosurfaces
  marching cubes/squares. VTK visualization E is finite differences,
  **not flyer reconstruction**. `docs/mesh-io.md:101-196`.
  STEP export is a PLAN goal but **not in the implemented export formats
  documented here**; no STEP/IGES round-trip accuracy claim is established.
- V3 exported quadrupole STL106,800 triangles: same-triangle direct-SDF
  versus CLI endpoint RMS1.442479e-8/max1.516489e-7 mm, zero class changes.
  But PA voxel export/reimport differs from physical analytic8x by
  1.74105488 mm RMS/43 exits. Not analytic-geometry preservation.
  `bench/fly-real.md:706-713`.
- IOB: GUI reads binary workbench bounds and PA placements validated against
  148 installed examples. Same einzel10k with correct workbench:
  meanTOF15.300 us versus5.105 us stopping at PA boundary.
  `docs/dev/frontend.md:196-200`.
  GUI exported19 format files across cylindrical/planar/3D arrays,
  structurally inspected; reopened PA/PA0 meanTOF15.2998855 us matched
  original. `docs/dev/frontend.md:202-216`.

## 7. GUI

- Start screen and four working templates: einzel, electrostatic quadrupole,
  reflectron TOF, ion trap; create -> refine -> voltage/source -> fly.
  Native file drag/drop, import wizard parts/numbers/voltages/units/repair,
  spacing recommendations and cut-cell versus aligned choice; exact-surface
  toggle/badges and explicit CPU fallback; progressive refine with residual
  history/cancel; CAD orbit/pan/zoom/slices; live fast-adjust sliders.
  `docs/dev/frontend.md:170-200,218-296,326-346,491-508`.
- Latest source-only GUI GEM path uses exact `electrode_sdfs`, not fourfold
  sampled CSG. Quadrupole app/CLI exact routes:750 +x,230 electrode3,
  20 electrode2, meanTOF4.1145034797 us; app packet endpoint RMS2.3e-7 mm
  versus CLI (f32 packet). Against physical8x, app-created field flown at
  reference settings gives0.003435341 mm RMS/max0.02018961 mm/zero changed
  exits. `docs/dev/frontend.md:326-346` ("정확한 전극 표면").
- Undo/redo and drag coalescing; JSON `.simrep` with asset paths, camera,
  voltages/source/settings; recovery autosave1.5 s after changes;
  Korean/English and light/dark; linked trajectory/map/table/ion selection.
  Results: TOF/KE/position histograms, detector map, per-ion traces, virtual
  table and retained-result comparison; CSV, PNG, SVG, VTP exports.
  `docs/dev/frontend.md:130-168,449-473,505-519`.
- Five-step in-app tour is actual template -> voltage -> flight -> TOF
  histogram -> CSV. Real CDP clicks completed the path;40-ion CSV41 lines.
  `docs/dev/frontend.md:475-489` ("5분 따라하기").
- Installer: NSIS current-user, no administrator, English/Korean,
  `%LOCALAPPDATA%\\SimRep`; optional app-data deletion on uninstall.
  CAD-enabled release OCCT7.7.0 bundle:39 OCCT/third-party DLLs plus
  three MSVC runtime DLLs,70.4 MB dependency bundle.
  Setup executable29,873,560 B(28.49 MiB), no build warnings.
  Silent install exit0/5.0 s,54 files/95.5 MB, HKCU only; no HKLM/
  ProgramData writes. Installed STEP and IGES two-sphere workflows:
  24x12x12 cut mesh, CPU27 iterations/residual7.5e-11,200 ions,
  meanTOF0.08756 us. Uninstall exit0/1.3 s removed installation/HKCU/
  shortcuts; user data remains unless selected. Live app processes can
  prevent deletion despite exit0: close the app first.
  `docs/dev/frontend.md:84-120` ("Windows 설치 패키지").
- Live update measurements belong in chapter4 above; neither Preview f32
  nor interactive refresh is the precise CPU scientific-acceptance mode.

## 8. Equation ledger: literal implementation semantics, units and provenance

Notation here is LaTeX transcription of stated docs/code, not a new model.
Raw SIMION files below are all under
`/mnt/c/Users/<user>/simion_docs_raw/`. A relevant page does not imply that
SIMION uses our numerical discretization.

### Electrostatics, multigrid and cut cells

1. Cartesian stencil:
   \[
   L_h V_i=\sum_{a\ {\rm active}}
   \frac{V_{i-e_a}-2V_i+V_{i+e_a}}{h_a^2},
   \qquad -L_hV=s,\quad s=\rho/\epsilon_0.
   \]
   Mirror inward coefficient doubles. Cylindrical:
   \[
   L_hV=\frac{(1-h_r/(2r))V_{r-1}-2V_r+
   (1+h_r/(2r))V_{r+1}}{h_r^2}+\delta_{zz}V,\qquad
   L_rV(0)=4[V(h_r)-V(0)]/h_r^2.
   \]
   Units V/mm^2 for mm grids. `docs/cpu-solver.md:15-52`
   ("Discrete operator"); code `crates/simrep-solve/src/mg.rs:109-137`
   (`solve`, `solve_poisson`); raw `laplace_equation.html`,
   `poisson_equation.html` ("Definition").
2. Weighted hierarchy/residual:
   \[
   A=-WL,\quad R=P^T,\quad A_c=P^TAP,\qquad
   \|r\|_\infty\le \tau_{\rm abs}+\tau_{\rm rel}\|b_{\rm reduced}\|_\infty.
   \]
   `docs/cpu-solver.md:47-76,97-113`;
   code `crates/simrep-solve/src/mg.rs:182-257`
   (`solve_in_pool`, `reached`).
   Raw SIMION `refine.html` concerns its own stopping objective, **not**
   a source for this Galerkin or residual-norm algorithm.
3. Ghost-fluid cut contribution and snapping:
   \[
   c(V_j-V_i)\ \mapsto\ (c/\theta)(V_\Gamma-V_i),\qquad
   d_{\rm snap}=2h_{\min}^2/L.
   \]
   `docs/cutcell.md:39-102`; code
   `crates/simrep-solve/src/cutcell.rs:1-45`
   (`CutRow::{diagonal,source,residual}`), `CutCells::new_with_root_tolerance`
   at74. Paper equation21 is the authority; no SIMION claim.
4. Field and fast adjust:
   \[
   \mathbf E=-\nabla V,\qquad
   V_{\rm new}=V_{\rm PA0}+
   \sum_i\frac{U_i-U_{i,0}}{U_{\rm ref}}V_{{\rm basis},i}.
   \]
   `bench/fly-real.md:27-38,276-300`;
   `crates/simrep-fly/src/field/boundary.rs:112-187`
   (`cell_powers`, `sample`);
   `crates/simrep-io/src/fast_adjust.rs:26`
   (`read_fast_adjust`).
   V and V/mm. Raw `writing_pa0_files.html`,
   `time_dependent_field.html`, `lua_simion.pas.html`.

### Motion and integration

5. Actual Lorentz/proper-velocity ODE:
   \[
   \dot{\mathbf x}=\mathbf v=\mathbf u/\gamma,\quad
   \gamma=\sqrt{1+|\mathbf u|^2/c^2},\quad
   \dot{\mathbf u}=\frac{q_e}{m_{\rm amu}}
   [a_E\mathbf E+a_B(\mathbf v\times\mathbf B)].
   \]
   Nonrelativistic mode sets gamma=1, u=v. Constants:
   \[
   a_E=\frac{1.602176634\,10^{-19}}{1.66053906892\,10^{-27}}10^{-6},
   \quad a_B=10^{-4}a_E,\quad c=299792.458\ {\rm mm/us}.
   \]
   x mm, t us, u/v mm/us, E V/mm, B G. Code
   `crates/simrep-fly/src/lib.rs:25-30`;
   `src/flight.rs:450-480` (`velocity`, `derivative`).
   `crates/simrep-fly/README.md:70-96`; raw
   `lorentz_force_law.html:45-53,94-103`,
   `omit_relativistic_effects.html`.
6. RK45 accepted state/error:
   \[
   \mathbf k_s=f(\mathbf y+h\sum_{j<s}a_{sj}\mathbf k_j,t+c_sh),
   \quad\mathbf y_5=\mathbf y+h\sum_jb_{5j}\mathbf k_j,\quad
   \mathbf y_4=\mathbf y+h\sum_jb_{4j}\mathbf k_j,
   \quad
   e=\max_a\frac{|y_{5a}-y_{4a}|}
   {{\rm atol}_a+{\rm rtol}\max(|y_a|,|y_{5a}|)}.
   \]
   Velocity components use **physical** high/low velocities.
   Accepted step growth `clamp(0.9 e^(-1/5),0.2,5)`, zero error factor5;
   rejected shrink `clamp(0.9 e^(-1/5),0.1,0.5)`.
   Code `crates/simrep-fly/src/flight.rs:483-598,807-810,948-960`
   (`rk45`, `fly`). Raw `particle_trajectory_calculation.html`/
   `time_step_size.html` describe SIMION, not our Dormand-Prince method.
7. Boris midpoint:
   \[
   \mathbf x_m=\mathbf x_n+\tfrac h2\mathbf v_n,\quad
   \mathbf u^-=\mathbf u_n+\tfrac h2(q/m)a_E\mathbf E_m,\quad
   \mathbf t=\tfrac h2(q/m)a_B\mathbf B_m/\gamma^-,
   \quad\mathbf s=2\mathbf t/(1+|\mathbf t|^2),
   \]
   \[
   \mathbf u'=\mathbf u^-+\mathbf u^-\times\mathbf t,\quad
   \mathbf u_{n+1}=\mathbf u^-+\mathbf u'\times\mathbf s+
   \tfrac h2(q/m)a_E\mathbf E_m,\quad
   \mathbf x_{n+1}=\mathbf x_m+\tfrac h2\mathbf v_{n+1}.
   \]
   `crates/simrep-fly/src/flight.rs:601-636` (`boris`);
   ensemble velocity push `physics/ensemble.rs:40-75`.
   Method named but no paper is cited in repository.
8. Automatic launch budget:
   \[
   \tau=\operatorname{clamp}
   \left(10^{-3}\frac{\|\mathbf E_0-4\mathbf E_1+6\mathbf E_2-
   4\mathbf E_3+\mathbf E_4\|}{\max_i\|\mathbf E_i\|},
   10^{-12},10^{-8}\right).
   \]
   Sample spacing one physical cell; velocity absolute budget at least
   tau times launch speed. `bench/default-flight.md:124-140`;
   `crates/simrep-fly/src/flight.rs:200-248`
   (`with_field_tolerance`). A bounded interpolation estimate, not a
   physical continuum error theorem.

### Magnetic sources and neutral gas

9. Magnetic PA/coil:
   \[
   \mathbf B=-n_g\nabla p,\qquad
   \mathbf B(\mathbf x)=\frac{\mu_0}{4\pi}
   \int I\,d\boldsymbol\ell\times\mathbf r/|\mathbf r|^3,\qquad
   \mu_0/(4\pi)=1\ {\rm G\,mm/A}.
   \]
   Hoop axis: `B_axis=2*pi*I*R^2/(R^2+z^2)^(3/2)` in G/mm/A
   conventions. General hoop uses K/E with
   `m=4*R*rho/((R+rho)^2+z^2)`.
   `docs/physics-magnetic-collision.md:110-145`;
   code `physics/magnetic.rs:133-160` (`sample_local`),
   `245-334` (`Coil::field_gauss`, `elliptic_ke`).
   Raw `magnetic_potential.html`, `biot_savart_law.html:45-60`,
   `lua_simionx.MField.html`, `solenoid_magnetic_field.html`.
10. HS1:
    \[
    n=P/(k_BT),\quad \nu=n\sigma\langle|\mathbf v-\mathbf v_g|\rangle,
    \quad \lambda=|\mathbf v-\mathbf u_g|/\nu,\quad
    \tau=-\ln U,\quad P_{\rm coll}=1-e^{-\nu\Delta t}.
    \]
    Low-field hard-sphere mobility:
    \[
    K=\frac{3|q|}{16n\sigma}\sqrt{\frac{2\pi}{\mu k_BT}},\qquad v_d=KE.
    \]
    K m^2/(V s), reduced mass kg, cross section m^2;
    native velocity mm/us and nu per us. The code explicitly converts
    thermal speed from m/s to mm/us.
    `docs/physics-magnetic-collision.md:152-193`;
    `physics/collision.rs:141-185`
    (`mean_relative_speed`, `collision_rate_per_us`, `low_field_mobility`).
    Raw `collision_model_hs1.html`, `ion_gas_collisions.html`,
    `mobility_equation.html:45-69`; actual source laws also follow installed
    `examples/collision_hs1/collision_hs1.lua`, not a paper citation.
11. SDS:
    \[
    r=L\,{\rm ICDF}(\log_{10}(m_i/m_g),U)
       \sqrt{(V_{\rm thermal}/L)\Delta t/100000},
    \quad
    \dot{\mathbf v}=(q/m)[a_E\mathbf E+a_B\mathbf v\times\mathbf B]
       -\nu_d(\mathbf v-\mathbf u_g).
    \]
    Exact damped constant-field integrals use
    `z=-nu_d+i*omega`, `F1=(exp(z*h)-1)/z`,
    `F2=(F1-h)/z`; diffusion is an accepted-step isotropic position jump.
    `docs/physics-magnetic-collision.md:195-237`;
    `physics/collision.rs:527-599` (`integrals`, `damped_step`).
    Raw `mobility_equation.html` ("SIMION Specific"), `ion_gas_collisions.html`;
    installed `examples/collision_sds/collision_sds.lua`/`mbmr.dat` are the
    actual empirical model. No standalone SDS paper is cited in repo.

### Dielectrics and charge

12. Material electrostatics:
    \[
    -\nabla\cdot(\epsilon_r\nabla V)=\rho_f/\epsilon_0,\quad
    \epsilon_{ij}^{\rm nodal}=2/(1/\epsilon_i+1/\epsilon_j),\quad
    \epsilon_r(E)=\max[\epsilon_{r,0}(1-\lambda|E|^2),\epsilon_{\min}].
    \]
    E V/mm for PA; shipped lambda8e-17 with E V/m becomes8e-11
    with E V/mm. `docs/dielectric.md:3-15,113-147,359-376`;
    code `dielectric.rs:130-155` (`harmonic`, `Permittivity::from_nodes/
    from_cells`), `dielectric/nonlinear.rs:13-66`
    (`FieldLaw::epsilon`), `NonlinearPermittivity::solve`.
    Raw `dielectric.html:52-87`, `poisson_equation.html:47-59`.
    **Raw-help typo:** `poisson_equation.html:50` displays
    `div(epsilon_r E)=-rho/epsilon0`, inconsistent with its own
    potential form at56 and Gauss law at63; implementation uses the
    potential form above, not that E-sign typo.
13. Represented sources/direct field:
    \[
    Q_i^{\rm factor}=f q_i e\,w_i,\quad
    Q_i^{\rm Coulombic}=Q\,q_iw_i/\sum_j|q_j|w_j,\quad
    \mathbf E_i=k\sum_{j\ne i}
    Q_j\mathbf r_{ij}/(|\mathbf r_{ij}|^2+\varepsilon^2)^{3/2},
    \quad k=(4\pi\epsilon_0)^{-1}.
    \]
    Q coulombs, w=CWF, eps mm, epsilon0=8.8541878128e-15 F/mm,
    E V/mm. `docs/physics-space-charge.md:9-31,75-90`;
    `physics/space_charge.rs:7-107`
    (`ChargeMode::charges`, `DirectCharge::field`).
    Raw `charge_repulsion.html`, `coulombs_law.html`.
14. PIC:
    \[
    \rho_n=V_n^{-1}\sum_iQ_iS_n(\mathbf x_i),\quad
    -L_h\phi_{\rm sc}=\rho/\epsilon_0,\quad
    \mathbf E_i=\sum_n S_n(\mathbf x_i)(-\nabla_h\phi_{\rm sc})_n.
    \]
    CIC uses identical deposit/gather shape, dual-cell volume, zero correction
    at fixed conductors. `docs/physics-space-charge.md:92-150`;
    `physics/space_charge.rs:152-391` (`PicMesh`),
    `physics/gpu_pic.rs` (`GpuPic`).
    Raw `space_charge.html`, `poisson_solver.html`; these do not document
    our linked-list f64 GPU deposition implementation.
15. Beam:
    \[
    I_i=I q_iw_i/\sum_j|q_j|w_j,\quad
    \lambda_i=10^{-6}I_i/(\mathbf v_i\cdot\mathbf n),\quad
    \mathbf E_{ij}=2k\lambda_j\mathbf r_\perp/
       (|\mathbf r_\perp|^2+\varepsilon^2),\quad
    R''=C/R,\quad C=(q/m)a_E\,2kI10^{-6}/v.
    \]
    I A, lambda C/mm, v mm/us, R mm and derivatives in time.
    `docs/physics-space-charge.md:181-233`;
    `physics/beam.rs` (`fly_beam`, line-field/encounter limiter).
    Raw `charge_repulsion.html` ("Types of Charge Repulsion Methods",
    beam/CWF discussion), `space_charge.html`. Private cloud overlap,
    longitudinal self-force, self-B and electrode images are omitted.

### Magnetostatic PDEs and field reconstruction

16. Scalar, planar current, axisymmetric current and Cartesian current:
    \[
    -\nabla\cdot(\mu_r\nabla p)=(4\pi/n_g)(-\nabla\cdot\mathbf M),
    \quad\mathbf B=-n_g\mu_r\nabla p+4\pi\mathbf M,
    \]
    \[
    -\nabla\cdot(\mu_r^{-1}\nabla A_z)=(4\pi/n_g)J_z,\quad
    -\nabla_{r,z}\cdot[(\mu_r r)^{-1}\nabla_{r,z}\psi]
       =(4\pi/n_g)J_\phi,\quad \psi=rA_\phi,
    \]
    \[
    -\mu_r^{-1}\Delta A_i=(4\pi/n_g)J_i,\qquad
    \mathbf B=n_g\nabla\times\mathbf A.
    \]
    Last PDE only uniform mu. M A/mm, J A/mm^2, B G, lengths mm.
    The psi divergence is **planar**, not axisymmetric scalar Laplace.
    `docs/magnetostatics.md:3-37`; code
    `crates/simrep-solve/src/magnetostatics.rs:152-325`
    (`MagneticProblem::{cartesian_current_component,scalar,currents}`).
    Raw `magnetic_potential.html` ("Magnetic Vector and Scalar Potential -
    Theory"), `permeability.html`, `poisson_solver.html`.
17. Regular axis/curl:
    \[
    f=\psi/r^2,\quad f(0)=[4f(h)-f(2h)]/3,\quad
    B_x=n_g(2f+r\partial_rf),\quad B_r=-n_gr\partial_xf,\quad
    B_x(0)=2n_gf(0).
    \]
    Curl transforms under reflections as `det(R) R B`, preserving axial-vector
    parity. `docs/magnetostatics.md:39-55`;
    `crates/simrep-fly/src/physics/magnetic/vector_potential.rs:32-96,145-186`
    (`VectorPotential::placed`, `sample`).
    No claim that the help pages prescribe this specific C2 regularization.

## 9. Limitations, open items and contradictions to preserve

- PLAN design prose is not shipped-state evidence: planned tricubic and
  texture interpolation versus current boundary-aware quintics; planned
  df64/f32 GPU versus current f64/native and explicit Preview; planned
  octree/BEM, broad export formats and web port remain goals.
  `docs/PLAN.md:35-61,152-180`.
- Older `docs/cpu-solver.md:21-23`/`docs/gpu-solver.md:7-9` say GPU physical
  Poisson/PIC is unavailable; **superseded** by
  `docs/physics-space-charge.md:126-150` and hardware results in
  `bench/space-charge.md:314-353`. Do not copy the stale limitation.
- `crates/simrep-fly/README.md` later "Real SIMION arrays" prose still calls
  FLY2 a literal-only source and says no Lua/IOB; its earlier particle-input
  section and `docs/fly2.md` describe executable source Lua, while GUI IOB
  is documented separately. Source Lua is **not** workbench-program Lua.
  `README.md:58-68,225-249`; `docs/fly2.md:53-132`;
  `docs/dev/frontend.md:196-200`.
- **Actual Python/CLI default discrepancy:** `docs/python.md:344-348`
  states one cell, and `crates/simrep-py/src/flight.rs:115-131`
  (`Settings::default`) really assigns `max_grid_cells:1.0`; at574-576 it
  passes that into `StepLimits`. This is not merely stale Python prose.
  Rust `FlightOptions::default`/`StepLimits` and the main CLI policy
  (`bench/default-flight.md:3-10`;
  `crates/simrep-fly/src/flight.rs:289-302`) use four cells. Do not claim
  identical Python/CLI step defaults without addressing that override.
- `bench/fly-real.md:8-23` intro's0.225707623 mm/0.223796 s is older
  than `bench/default-flight.md:3-44` selected main0.226123540 mm/
  0.184954 s. Likewise historical nearest-node GPU/quad comparisons are
  not all-corner acceptance. Historical whole-command gates were replaced
  by compute-only; do not mix clocks.
- Residual tolerance is not continuum geometry accuracy; cut-cell near-surface
  E is roughly first order; curved dielectric/magnetic interfaces remain
  first-order/phase-sensitive. Numerical references are not certified global
  bounds. `docs/cpu-solver.md:104-108`; `docs/cutcell.md:153-190`;
  `docs/dielectric.md:538-616`; `docs/magnetostatics.md:177-206`.
- V1/V2 CSG approximations round corners; v3 triangle accuracy does not
  remove CAD tessellation error. Exact-surface GPU flight not implemented;
  material+continuous-cut electrostatics not combined.
  `docs/geometry-pipeline.md:142-192`; `docs/dielectric.md:139-147`.
- GEM excludes workbench WGEM/multiple pa_define and host Lua APIs;
  analytic SDF export excludes Lua shape/voltage callbacks and hyperbola/
  parabola SDFs although rasterization works. FLY2 excludes binary FLY,
  native Lua extensions, arbitrary host access, negative coordinate modes
  and filled deterministic circle/cone sequences.
  `docs/gem.md:110-159`; `docs/fly2.md:109-132`.
- Mesh safe repair does not heal open solids; glTF compressed geometry,
  animation/skinning/materials and network assets are outside scope.
  CAD corpus is bounded, not all industrial STEP/AP242 PMI.
  `docs/mesh-io.md:3-6,27-36,245-252`; `docs/step-import.md:24-27`.
- Direct/beam omit image charges/private cloud kernel; PIC grounded correction
  has different boundary physics. Beam static-only, single pass, no
  self-consistent trajectory-Poisson iteration; mixed default endpoints
  are not individually converged. `docs/physics-space-charge.md:75-90,
  118-124,196-223,259-298`; `bench/space-charge.md:433-455`.
- No nonlinear B-H or spatially varying-mu Cartesian vector solver;
  vector/material magnetic flights CPU-only. Uniform Az is a measured
  speed loss. No automatic GPU magnetic crossover is established.
  `docs/magnetostatics.md:3-61,312-324`;
  `bench/fly-gpu.md:550-622`.
- Precise/Preview tolerances differ; GPU large fields must fit binding limits,
  not streamed. Hardware occupancy, register spills, FP64 utilization and
  stalls are unmeasured; Nsight install did not yield Vulkan counters.
  `bench/fly-gpu.md:88-123`; `docs/gpu-solver.md:120-130`.
- Latest app quad100k still8.6 updates/s in recorded app evidence; newer
  isolated kernel54.886016 ms is not a GUI remeasurement.
  `docs/dev/frontend.md:425-447`; `bench/fly-gpu.md:7-52`.
- `docs/step-import.md:3-7` says app untouched in that increment; later
  `docs/dev/frontend.md:170-194` documents actual CAD integration.
  Do not interpret the earlier increment boundary as current absence.
- Many physical timing tables explicitly retain shared-machine contention.
  Do not describe dielectric/magnetostatics/app tables as isolated-machine
  throughput. Raw artifacts are often under user-profile Windows paths,
  not committed reproducibility bundles. LSP requests in this audit all
  timed out; no new code/test validation is claimed.

## 10. External literature actually cited, verified bibliography

Inventory scope: `docs/` excluding `docs/report/`, `bench/`, `crates/`
including README/code comments. A separate read-only bibliography audit
checked URL/author/title/DOI-like strings. Verification date2026-10-24:
publisher/Crossref, official project APIs and primary documentation.
Only one actual research-paper bibliographic citation was found; method
names below are not retroactively converted into citations.

1. **Frederic Gibou, Ronald P. Fedkiw, Li-Tien Cheng, Myungjoo Kang.**
   "A Second-Order-Accurate Symmetric Discretization of the Poisson Equation
   on Irregular Domains." *Journal of Computational Physics* **176**(1),
   205-227, February2002. DOI **10.1006/jcph.2001.6977**.
   Cited `docs/cutcell.md:41-45`, specifically linear equation21 versus
   nonsymmetric quadratic equation20. DOI/title/authors/volume/pages verified
   through Crossref metadata; cited author manuscript
   `https://physbam.stanford.edu/papers/cam2000-37.pdf` returns PDF.
2. **Scientific Instrument Services / SIMION documentation.**
   "FLY2 File" and "simion.wb" API pages, undated:
   `https://simion.com/info/fly2_file.html`,
   `https://simion.com/info/lua_simion.wb.html`.
   Cited `docs/fly2.md:142-144`; both live primary pages verified.
3. **Open CASCADE Technology documentation.** "STEP User Guide" /
   STEP-XDE translation and assembly/name coverage, undated:
   `https://dev.opencascade.org/doc/overview/html/occt_user_guides__step.html`
   (now redirects to official `occt3d.com` documentation).
   Cited `docs/step-import.md:39-42`; final page verified.
4. **Free Software Foundation; Open CASCADE SAS.** *GNU Lesser General
   Public License, version2.1* and *Open CASCADE Technology LGPL Exception*,
   distributed at OCCT tagV7_6_3:
   `https://github.com/Open-Cascade-SAS/OCCT/blob/V7_6_3/LICENSE_LGPL_21.txt`,
   `https://github.com/Open-Cascade-SAS/OCCT/blob/V7_6_3/OCCT_LGPL_EXCEPTION.txt`.
   Cited `docs/step-import.md:43-45`; tagged files verified.
5. **bschwind/opencascade-rs project contributors.** *Rust bindings to the
   OpenCascade CAD Kernel*, repository, undated:
   `https://github.com/bschwind/opencascade-rs`.
   Cited `docs/step-import.md:46`; official repository verified.
6. **ricosjp/truck project contributors.** *truck-stepio*, STEP I/O package,
   repository, undated:
   `https://github.com/ricosjp/truck/tree/master/truck-stepio`.
   Cited `docs/step-import.md:47`; upstream path verified.
7. **Open CASCADE SAS.** *OCCT7.7.0 release / Windows SDK*:
   `https://github.com/Open-Cascade-SAS/OCCT/releases/tag/V7_7_0`.
   Cited `docs/dev/windows-occt.md:12`; release and
   `opencascade-7.7.0-vc14-64.exe` asset verified.
8. **Daniel Scharrer / innoextract contributors.** *innoextract1.9*,
   Windows extraction utility/release:
   `https://github.com/dscharrer/innoextract/releases`,
   `https://github.com/dscharrer/innoextract/releases/download/1.9/innoextract-1.9-windows.zip`.
   Cited `docs/dev/windows-occt.md:36,46`; release/asset verified.
9. **NVIDIA Corporation.** *Nsight Compute Profiling Guide* and *Nsight
   Systems User Guide*, live manuals:
   `https://docs.nvidia.com/nsight-compute/ProfilingGuide/index.html`,
   `https://docs.nvidia.com/nsight-systems/UserGuide/index.html`.
   Cited `bench/fly-gpu.md:116-123`; official pages verified.
10. **NVIDIA Corporation.** *CUDA GPU Compute Capability* table,
    `https://developer.nvidia.com/cuda/gpus`;
    *CUDA C++ Programming Guide, version12.6*, Table4 arithmetic instructions,
    `https://docs.nvidia.com/cuda/archive/12.6.0/cuda-c-programming-guide/index.html#arithmetic-instructions`;
    *Ampere Tuning Guide*, section1.4.1.1 occupancy,
    `https://docs.nvidia.com/cuda/ampere-tuning-guide/index.html#occupancy`.
    Cited `bench/fly-gpu.md:506-510`; official pages verified.

**Named but no bibliographic citation found:** Dormand-Prince5(4), Boris,
Galerkin multigrid, Chebyshev/Jacobi, quintic Hermite/B-splines, Horner,
Biot-Savart/AGM, Mason-Schamp/Chapman-Enskog, CIC PIC, Plummer, SplitMix64,
Philox, DOP853, SciPy/NumPy and CODATA constants. No book/ISBN/arXiv citation
was found. Do not insert a plausible paper/DOI as if the repository cited it.
Shortley-Weller is named in the Gibou comparison, not separately referenced.
SIMION CHM/manual appendices are local references, not public bibliographic
URLs; exact raw filenames for equation provenance are listed in chapter8.
Localhost development URLs and XML namespace identifiers in format fixtures
are not literature.
