# CPU reference potential solver

The additive `dielectric::Permittivity` API solves scalar variable-permittivity
electrostatics, including CPU free charge, through the same Galerkin hierarchy.
See `docs/dielectric.md` for nodal harmonic coefficients, SIMION cell arrays,
interfaces, and validation. The ordinary `mg::solve` operator is unchanged.

`simrep_solve::mg::solve` takes an x-fastest `&mut [f64]`, a `&[bool]`
electrode mask, `Grid { dims, spacing, geometry }`, and `Options`. It does not
depend on `simrep-core::PotentialArray`. Masked values are the prescribed
Dirichlet potentials; unmasked values are the initial guess. Fixed values are
retained bit-for-bit. A fixed point is required to remove the pure-Neumann
constant nullspace.

`mg::solve_poisson(potential, electrodes, source, grid, options)` additively
solves `-L phi = source` with those same boundaries. Supply the physical
unweighted `rho/epsilon_0` in V/length², not a row-weighted source; with mm
coordinates divide C/mm³ density by `8.8541878128e-15 F/mm`. The fine smoother
and residual both include this source, and restriction receives `W*(source+L phi)`.
Relative scaling includes the source and reduced Dirichlet RHS. Fixed-node
sources are ignored. This CPU API supports planar and cylindrical meshes;
GPU Poisson source support is not yet implemented. See
`docs/physics-space-charge.md` and `tests/poisson.rs`.

`nz = 1` selects a 2D grid; otherwise the planar operator is 3D. Every unmasked
outer face has zero normal derivative, independently for each active axis.
Dirichlet faces are expressed by masking their nodes. Spacing can differ
between axes. Cylindrical geometry requires `nz = 1`; storage axes are r, z,
with an optional nonnegative radial origin.

## Discrete operator

The planar operator uses the usual second differences. At a mirror face the
inward neighbor coefficient is `2/h^2`, from reflection of the ghost value.
The axisymmetric operator is

```text
L phi = d²phi/dr² + (1/r) dphi/dr + d²phi/dz²
      = [(1 - h/(2r)) phi[r-1] - 2 phi[r]
         + (1 + h/(2r)) phi[r+1]] / h² + d²phi/dz².
```

At `r = 0`, regularity gives `L_r phi = 4*(phi[1] - phi[0])/h_r²`,
not the planar mirror coefficient 2. A positive-origin annular grid instead
uses an ordinary mirror face when that face is not masked.

For multigrid the physical operator is multiplied row-wise by positive nodal
weights, giving a symmetric correction matrix `A = -W L`. Planar weights are
1 internally and 1/2 on mirror faces. Radial interior weights are `r/h_r`,
the axis weight is 1/8, and the outer weight is `(r/h_r - 1/2)/2`.
For an annular inner mirror face its weight is `(r/h_r + 1/2)/2`.
Weights multiply across axes. This changes no solution or physical residual.
Known electrode columns are eliminated only in the correction operator.

## Coarsening electrodes without losing constraints

Each axis selects indices 0, 2, 4, ... and always includes the last endpoint.
Axes of length 1 or 2 stop shrinking. Thus even, odd, and non-cubic dimensions
work without padding or moving the fine-grid boundaries.

Prolongation `P` is tensor-product linear interpolation in the parent grid's
index coordinates, with **zero correction at every fine electrode**.
Restriction is exactly `P^T`. The coarse matrix is `P^T A P`, assembled row-wise
in parallel and stored as a 27-point stencil (9 points in 2D).
This is a geometric hierarchy with Galerkin operators, not a coarse sampled
mask: a one-node electrode, a thin wire, and partially filled aggregates
contribute to coarse diagonals/couplings even when no selected coarse node
lands on the electrode. No electrode dilation or erosion is introduced.
Nested, endpoint-preserving interpolation keeps the coarse stencil local.

V-cycles use parallel Chebyshev-Jacobi smoothing (three pre- and three post-sweeps
by default). Fine-grid scaling uses its diagonal; Galerkin levels use their
individual absolute row sums, as on the GPU. This bounds the symmetric
preconditioned spectral radius by one without allowing a nearly empty coarse
basis column to reduce every row's damping. A three-sweep Chebyshev polynomial
damps the upper spectral band; the bottom solve uses stationary Jacobi.
Jacobi avoids unsafe
concurrent in-place updates and remains valid when coarse couplings are not
red-black. Coarsening continues until every axis is at most four nodes, so
skinny grids cannot stop with a long unresolved axis merely because their
total node count is small. This coarsest grid (at most 64 nodes) gets 80 sweeps. Empty basis
columns have zero diagonal and are skipped. If only a few free nodes remain,
coarse bases can be dependent; a consistent coarse Jacobi solve avoids needing
a nonsingular dense factorization. The fine anchored system remains the
convergence authority.

`mirrored_octant_shells_converge_with_large_fixed_regions` covers the large
masked regions in real spherical-shell PA files. With one global
diagonal-scaled Gershgorin bound, its 65-cubed hierarchy reached a bound of
2372.86 on the bottom level and needed 67 cycles. Local row-sum scaling
converges in eight cycles without changing the f64 residual criterion.

Hierarchy setup and all solution/residual operations use f64. Rayon parallelizes
setup, smoothing, restriction, prolongation, residual norms, and output copying.
No GPU code or GPU context is involved.

## Convergence and reporting

Each cycle computes the **f64 infinity norm of the physical unweighted Laplace
residual over free nodes**, in V/length². Residuals use neighbor differences to
reduce cancellation. The internal potentials are shifted by the first fixed
voltage to reduce gauge-related cancellation.

The relative norm divides by the infinity norm of the gauge-shifted reduced
Dirichlet RHS: `b_i = sum(fixed neighbors j) c_ij*(V_j - V_first)`.
When this RHS is zero, a unit-voltage stencil scale is used. The criterion is
`absolute <= absolute_tolerance + relative_tolerance * scale`. A residual
tolerance is not a bound on the continuous-geometry error.

The report includes the initial residual and each cycle's absolute/relative
residual, V-cycle count, number of levels, convergence flag, and wall time
including setup and copying. Reaching `max_cycles` is explicitly reported as
nonconvergence; the CLI returns failure in that case.

## Validation

`tests/mg_reference.rs` independently implements serial SOR and checks MG
against its tightly converged discrete solution. Cases include anisotropic
parallel plates with mirror faces, a planar quadrupole, single-node interior
electrodes on arbitrary dimensions, stair-stepped coaxial cylinders in x-y,
concentric spheres in 3D, the cylindrical harmonic polynomial `r² - 2z²`
including `r=0`, and the annular cylindrical `log(r)` solution.

The `log(r)` and smooth planar sine/sinh cases check second-order error
reduction on nested grids. Curved stair-stepped electrodes are compared with
SOR separately from their analytic-geometry errors: those errors must not be
mistaken for iterative error or claimed to converge at second order.

Run the reproducible 3D unit-cube benchmark with

```sh
cargo build --release -p simrep-cli
RAYON_NUM_THREADS=16 target/release/simrep refine-bench --n 128
RAYON_NUM_THREADS=16 target/release/simrep refine-bench --n 256
```

All six cube faces have exact Dirichlet data for
`sin(pi*x) sin(pi*y) sinh(sqrt(2)*pi*z)/sinh(sqrt(2)*pi)`, and the initial
interior is zero. The command reports setup-plus-solve time, cycles, f64
relative residual, and maximum analytic discretization error separately.
It does not time CLI grid generation or analytic error evaluation. This CPU
benchmark is not the phase 1 GPU/SIMION speed acceptance measurement.

### Measured baseline (2026-10-23)

Ryzen 9 5950X, WSL Linux, release profile, `RAYON_NUM_THREADS=16`, default
relative tolerance `1e-10`, one run per size, setup included:

| Grid | Time | V-cycles | Levels | Final relative residual | Max analytic error |
|---|---:|---:|---:|---:|---:|
| 128³ | 2.298412 s | 17 | 7 | 7.158869225e-11 | 2.794214453e-5 |
| 256³ | 10.519190 s | 17 | 8 | 7.328593709e-11 | 6.932721520e-6 |

Analytic error decreases by 4.03 while the spacing approximately halves.
This is discretization error, distinct from the reported iterative residual.
The test suite's aligned annular log(r) errors at 17, 33, and 65 radial nodes
were `2.966296e-5`, `7.423450e-6`, and `1.858360e-6`, respectively.
The smooth planar sine/sinh errors were `1.108842e-3`, `2.779615e-4`, and
`6.962716e-5`. Both sequences have second-order convergence.

The stair-stepped 37² coaxial case had maximum analytic-geometry error
`1.383692e-1` but differed from tightly converged SOR by only `2.094e-12`.
The 33³ sphere case had analytic-geometry error `1.410777e-1` and discrete
SOR difference `1.288e-12`. These coarse staircase geometries do not satisfy
the eventual continuous-geometry accuracy target; the tests intentionally
separate that error from solver convergence.

Validation commands `cargo fmt --all --check`, `cargo build`,
`RAYON_NUM_THREADS=16 cargo test -- --nocapture`, and
`cargo clippy --workspace --all-targets -- -D warnings` all exited 0.
The workspace had 17 passing unit/integration tests, plus the existing GPU
smoke's two successful Vulkan/llvmpipe kernels; DX12 was unavailable on WSL.
No Windows executable was launched.
