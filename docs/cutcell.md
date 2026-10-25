# Continuous electrodes: symmetric linear ghost-fluid refinement

`simrep-geom` is the geometry interchange crate. It has no solver dependency.
`SignedDistance::distance([x,y,z])` is negative inside an electrode and positive
outside. Implementations must return finite, continuous, conservative distance
bounds with Lipschitz constant at most one, in the grid's length unit.
`Sdf` provides spheres, infinite axis-aligned cylinders, boxes, unit-normal
planes, union/min, intersection/max, difference/max(a,-b), and sampled grids.
CSG preserves signs and conservative bounds, not exact Euclidean distances.
`SampledSdf::new` validates finite x-fastest samples and spacings; evaluation
is trilinear, clamped outside its box. Cover the complete solve domain with
samples. Sample interpolation's own geometry error is part of the input error.

```rust
use simrep_geom::{Electrode, Sdf};
use simrep_solve::{cutcell::CutCells, mg::{Geometry, Grid, Options}};

let electrodes = [
    Electrode::new(1.0, Sdf::Sphere { center: [0.0; 3], radius: 0.31 }),
    Electrode::new(0.0,
        Sdf::Box { center: [0.0; 3], half_extent: [4.0; 3] }
        .difference(Sdf::Sphere { center: [0.0; 3], radius: 0.83 })),
];
let grid = Grid {
    dims: [65; 3], spacing: [2.0 / 64.0; 3], geometry: Geometry::Planar,
};
let boundary = CutCells::new(grid, [-1.0; 3], &electrodes)?;
let mut potential = boundary.values.clone();
let report = boundary.solve(&mut potential, Options::default())?;
assert!(report.converged);
# Ok::<(), Box<dyn std::error::Error>>(())
```

An importer may instead implement `SignedDistance`, or pass a thread-safe
closure to `Electrode::new`. `Electrode::shape` is an `Arc<dyn SignedDistance>`;
electrodes retain their own voltage. Different-voltage overlapping conductors
are rejected. At least one grid node must anchor the conductor potential.

## Operator and symmetry invariant

The method is the **linear**, symmetric ghost-fluid discretization, equation
(21) in Gibou, Fedkiw, Cheng and Kang, *A Second-Order-Accurate Symmetric
Discretization of the Poisson Equation on Irregular Domains*, JCP 176 (2002).
The reference manuscript is `https://physbam.stanford.edu/papers/cam2000-37.pdf`.
It is not the nonsymmetric quadratic/Shortley-Weller formula (20).

For a surface voltage V at fraction theta along a link of length h, replace
the ordinary contribution `c*(u_neighbor-u_i)` by

```text
(c/theta)*(V-u_i).
```

Free/free links retain exactly the original coefficients. Each cut contributes
a positive diagonal and a known Dirichlet source, never a one-sided change to
a free/free coefficient. Thus `A=-W L` stays symmetric positive definite on
each anchored connected component. Restriction remains `P^T`, coarse operators
remain `P^T A P`, and the existing CPU Chebyshev and GPU row-sum Jacobi bounds
remain valid. Tests check symmetry, positive energy and Galerkin energy equality
through three hierarchy levels, including cylindrical grids.

Planar 3D and planar 2D (`nz=1`) use the existing mirror-face weights.
Cylindrical 2D uses `(r,z,0)` storage coordinates, with origin[0] equal to
`Geometry::Cylindrical::radial_origin`. Its unweighted radial coefficients
remain `(1 +/- h/(2r))/h^2`, with coefficient `4/h^2` at r=0.
Multiplying by the existing radial weight makes each free/free edge
proportional to `r_{i+1/2}` in **both** directions. Axial and mirror weights
multiply as in `docs/cpu-solver.md`. Applying theta only to cut diagonal/source
terms therefore preserves the r-weighted symmetry, including the axis.

### Roots and small cuts

Conservative-distance sphere tracing locates the first conductor along each
axis segment, including a conductor between two free endpoints. A sign bracket
is refined by 48 bisections. Near tangency, tracing stops within
`snap_distance/64` in distance, avoiding an unbounded approach to a tangent root.

Let `L=max((dims[a]-1)*spacing[a])` and let h_min be the smallest active spacing.
Nodes within

```text
snap_distance = 2 h_min^2 / L
```

of an electrode are set to its Dirichlet voltage. This moves the surface by
**O(h^2/L)**, including under refinement and a change of length unit.
A constant fractional theta clamp would move it by O(h) and is forbidden.
The snap test is on the conservative SDF; a custom bound must accurately
represent distance near its zero set for this displacement guarantee.
The root tolerance is smaller than the same quadratic displacement bound.

Only boundary rows are explicit. CPU keeps the vectorized implicit interior
and overrides sparse boundary rows in their owning parallel slab. GPU encodes
cut-row slots in its existing fine mask and appends sparse rows to the matrix;
coarse storage is unchanged. Geometry preparation, hierarchy construction,
upload and final output copying are outside reported compute time.
Sparse cut-row and slot storage is additional to the same nodal potential grid.

CPU solves in f64. `Gpu::solve_sdf` uses the same f32 Galerkin V-cycles and f64
outer residual as `Gpu::solve`; the explicit precision variant also exercises
the CPU-f64 fallback. Fixed values are retained, the gauge-shifted Dirichlet
RHS infinity norm defines the relative residual, and the default is `1e-10`.
Iteration-budget exhaustion remains nonconvergence.

`CutCells::electric_field` uses unequal-distance quadratic differences at
free nodes, inserting the actual cut-surface voltage. Outer mirror normals
are zero; conductor nodes return `None`. Potential error, iterative residual,
and near-surface field error are different quantities.

## Analytic CLI and acceptance

```sh
cargo build --release -p simrep-cli
RAYON_NUM_THREADS=16 target/release/simrep refine-sdf --geometry sphere --n 128 --compare-staircase
RAYON_NUM_THREADS=16 target/release/simrep refine-sdf --geometry sphere --n 256 --compare-staircase
target/release/simrep refine-sdf --geometry cylinder --n 65
target/release/simrep refine-sdf --geometry cylindrical --n 65
simrep.exe refine-sdf --geometry sphere --n 256 --device gpu --backend vulkan --compare-staircase
```

The inner/outer radii are 0.31/0.83, at 1/0 V. Spheres occupy `[-1,1]^3`;
planar coaxial cylinders occupy `[-1,1]^2`. The axisymmetric test has
`h=(b-a)/(n-2)` and radial origin `a-h/2`, placing both surfaces half a cell
from the end nodes under refinement. Its z faces are mirrors.
Exact free-space solutions are `(1/r-1/b)/(1/a-1/b)` and
`ln(b/r)/ln(b/a)`, respectively. Max potential errors include snapped nodes
in the physical annulus. Field errors are maximum component errors over free
nodes within two grid spacings of either surface.

The staircase comparison uses the same dimensions and nodal conductor mask,
including quadratically snapped nodes, but ordinary stair-stepped stencils.
It reports both compute times and the potential error ratio. Preparation,
GPU initialization, upload and final readback are printed separately.

### Measured acceptance (2026-10-23)

Ryzen 9 5950X, Linux release CPU with 16 Rayon threads; native Windows
Vulkan RTX 3070, f32 V-cycles and GPU-native f64 residuals. Each timing is
one serial run, not a timing median. Relative tolerance is `1e-10`.

| Device | Grid | SDF compute (ms) | Staircase compute (ms) | Overhead | SDF max error | Staircase max error |
|---|---|---:|---:|---:|---:|---:|
| CPU | 128^3 | 238.368 | 219.064 | 1.088x | 6.711957e-4 | 3.991813e-2 |
| CPU | 256^3 | 2228.011 | 2000.849 | 1.114x | 2.678142e-4 | 2.573109e-2 |
| RTX 3070 | 128^3 | 89.484 | 64.665 | 1.384x | 6.711960e-4 | 3.991813e-2 |
| RTX 3070 | 256^3 | 285.495 | 218.014 | 1.310x | 2.678142e-4 | 2.573109e-2 |

All four meet the 1.5x compute-time limit. GPU SDF final relative residuals
were `4.217045045e-11` and `7.577449241e-11`, in 14 and 16 cycles.
CPU used nine and ten cycles. SDF potential errors are 59.47x and 96.08x
smaller than staircase errors at the same dimensions.

#### Grid convergence

The f64 CPU convergence test uses `1e-12` residual tolerance. Max potential
error includes snapped nodes in the physical annulus; max field component
error uses free nodes within two spacings of either surface.

| Geometry | n | Max potential error | Near-surface max field error |
|---|---:|---:|---:|
| Sphere | 65 | 4.792432e-3 | 5.189737e-1 |
| Sphere | 129 | 7.759763e-4 | 1.990090e-1 |
| Sphere | 257 | 2.226561e-4 | 7.853889e-2 |
| Planar cylinders, worst phase | 65 | 1.031903e-3 | 1.552507e-1 |
| Planar cylinders, worst phase | 129 | 5.631917e-4 | 6.122100e-2 |
| Planar cylinders, worst phase | 257 | 7.229136e-5 | 3.776763e-2 |
| Axisymmetric cylinders | 65 | 8.849054e-5 | 1.454852e-2 |
| Axisymmetric cylinders | 129 | 2.195969e-5 | 7.214950e-3 |
| Axisymmetric cylinders | 257 | 5.469603e-6 | 3.592335e-3 |

Measured orders between the first and last grids are **2.214, 1.918, 1.991**
for potential, and **1.362, 1.020, 1.000** for near-surface field.
The axisymmetric order uses the actual spacing ratio, not exactly four.
The planar-cylinder test takes the worst error over center translations of
0, 1/4, 1/2 and 3/4 of a cell in x at every resolution. This avoids accepting
a favorable grid alignment. Even a fixed-center sequence has phase-dependent
maxima (its overall potential order here is 1.73); no claim is made that
every adjacent-grid ratio equals four. Quadratic snapping itself changes which
nodes attain the maximum. Tests also enforce a uniform `6 h^2` potential
envelope and `20 h` field envelope, the full-range order, and at least 10x
improvement over staircase at the finest grid for every tested phase.

`cargo test` in a debug build runs the same test at n=33, 65, 129 instead of
65, 129, 257; release keeps the full ladder. Both ladders span two
refinements (endpoint spacing ratio of four), so the potential order
assertion, both envelopes and the staircase ratio are identical in both
profiles. The near-surface sphere field maximum does not enter its O(h)
regime until about n=97 (0.52 at n=33 and n=65, 0.20 at n=129), so the debug
run requires a 2x field-error improvement across the two refinements where
release requires 4x.

#### Equal memory budget

Sparse geometry storage is counted rather than equating memory with node
count. The logical GPU solve-buffer requirement for a cube with N fine nodes
and S cut rows is

```text
24 N + 108 n + 96 S + 120 sum(coarse_node_counts)
     + 48 number_of_levels + 8 sum(norm_reduction_counts) + 4 bytes.
```

This includes f32/f64 sparse rows, the existing mask, outer iterate, correction
vectors, explicit coarse matrices, axis arrays, uniforms, norm reductions and
the shared four-byte dummy binding.
Add `8 N` bytes for the final f64 readback staging buffer. Pipeline/driver
caches and host allocations are not GPU solver buffers.

| Solve | Compute buffers (MiB) | Including final staging (MiB) | Max potential error |
|---|---:|---:|---:|
| SDF 128^3 | 87.35 | 103.35 | 6.711960e-4 |
| Staircase 129^3 | 85.48 | 101.86 | 5.231624e-2 |
| SDF 256^3 | 679.06 | 807.06 | 2.678142e-4 |
| Staircase 257^3 | 671.44 | 800.94 | 2.241964e-2 |

The larger staircase cubes fit **within** the corresponding SDF memory
budgets, yet their potential errors remain **77.94x and 83.71x larger**.
Staircase errors in this table are the stricter `1e-12` CPU f64 discrete
solutions; the GPU solves the same operator, as checked independently below.
This is an explicit-buffer accounting comparison, not a process-RSS claim.

### Verification and Windows hygiene

`tests/cutcell.rs` also checks an electrode thinner than a grid interval
between two free endpoints, and verifies the small-cut distance shrinks
by exactly four when spacing halves. The multigrid unit test checks fine
and coarse weighted symmetry and energy equality. Existing staircase tests
are retained.

The standalone GPU test executes the three new SDF cases in CPU-f64-fallback
mode on DX12, and in both native/fallback modes on Vulkan. On the RTX 3070,
the largest SDF CPU/GPU difference was `1.614e-11 V` with a 1000 V gauge.
The actual annular cylindrical CLI on Windows Vulkan reproduced CPU errors
`8.849053604e-5` in potential and `1.454852025e-2` in field at n=65.
Linux Vulkan/llvmpipe is a software correctness check, not the speed device.

Every Windows invocation ran in the foreground, with a timeout, `/mnt/c`
working directory and closed stdin. The final command was:

```sh
s="$(wslpath -w tools/win/winproc.ps1)"
(cd /mnt/c && timeout 30 powershell.exe -NoProfile -NonInteractive \
  -ExecutionPolicy Bypass -File "$s" cleanup < /dev/null)
```

It exited successfully and printed:

```text
== process counts ==

Name           Count
----           -----
cmd.exe            4
conhost.exe       34
node.exe           4
powershell.exe     1
simrep-app.exe     1

== conhost by parent ==

Name                              Count
----                              -----
wsl.exe                              12
cmd.exe                               4
wslhost.exe                           4
lmgrd.exe                             2
node_repl.exe                         2
node.exe                              1
powershell.exe                        1
simrep-app.exe                        1
rlm.exe                               1
GoogleDriveFS.exe                     1
sunshine.exe                          1
dispatcher.exe                        1
codex.exe                             1
wlanext.exe                           1
matlab-mcp-server-windows-x64.exe     1

orphan conhost: 0
stopped orphan conhost: 0
```

No task-owned `cargo.exe`, `rustc.exe` or `simrep.exe` remained. The existing
GUI and its console were not task-owned and were retained.

The final Linux commands all exited zero:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
RAYON_NUM_THREADS=16 cargo test --workspace -- --nocapture
```

All 80 standard tests passed, plus 16 actual multigrid GPU comparisons and
two smoke kernels on Vulkan/llvmpipe. WSL DX12 was explicitly unavailable.
The debug cut-cell convergence test (n=33/65/129) took 35.3 seconds; its
release run at the full n=65/129/257 ladder took 10.61 seconds. The new symmetry fixture, geometry primitives,
sampled interpolation and existing staircase regressions all passed.
The Rust LSP daemon timed out after 30 seconds; compiler and strict Clippy
diagnostics were clean.
