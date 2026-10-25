//! CPU f64 geometric multigrid for masked, node-centred Laplace grids.
//!
//! Storage is x-fastest: `x + nx * (y + ny * z)`. Set `nz = 1` for 2D.
//! Unmasked outer faces use homogeneous Neumann (mirror) conditions. Electrode
//! values in the input slice are Dirichlet data and are never changed.
//!
//! Correction transfer is multilinear interpolation, zeroed at fine electrodes.
//! Restriction is its transpose and coarse matrices are Galerkin `P^T A P`,
//! not rediscretizations of a sampled electrode mask. Thin electrodes therefore
//! cannot disappear. Nested endpoint-preserving grids support arbitrary sizes.
//! See `docs/cpu-solver.md` for the stencil, norm, and coarse-grid details.

use std::error::Error;
use std::fmt;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use rayon::prelude::*;

/// Coordinates are x-y-z for planar grids, and r-z for cylindrical grids.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Geometry {
    Planar,
    /// Axisymmetric geometry; the radial origin must be nonnegative.
    Cylindrical {
        radial_origin: f64,
    },
}

#[derive(Clone, Copy, Debug)]
pub struct Grid {
    pub dims: [usize; 3],
    pub spacing: [f64; 3],
    pub geometry: Geometry,
}

#[derive(Clone, Copy, Debug)]
pub struct Options {
    pub relative_tolerance: f64,
    /// Absolute infinity norm of the unweighted Laplace residual (V/length^2).
    pub absolute_tolerance: f64,
    pub max_cycles: usize,
    pub pre_smooth: usize,
    pub post_smooth: usize,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            relative_tolerance: 1e-10,
            absolute_tolerance: 0.0,
            max_cycles: 100,
            pre_smooth: 3,
            post_smooth: 3,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Residual {
    pub absolute: f64,
    pub relative: f64,
}

#[derive(Debug)]
pub struct Report {
    pub converged: bool,
    /// Number of V-cycles; history also contains the initial residual.
    pub iterations: usize,
    pub residual_history: Vec<Residual>,
    /// Includes validation, hierarchy construction, and the final copy.
    pub elapsed: Duration,
    pub levels: usize,
    pub worker_threads: usize,
    pub timings: Timings,
}

/// Wall-clock phase times; solve includes smoothing, transfers and residuals.
#[derive(Debug, Default)]
pub struct Timings {
    pub setup: Duration,
    pub solve: Duration,
    pub smoothing: Duration,
    pub restriction: Duration,
    pub prolongation: Duration,
    pub residual: Duration,
}

#[derive(Debug)]
pub struct SolveError(String);

impl fmt::Display for SolveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl Error for SolveError {}

pub(crate) fn invalid(message: &str) -> SolveError {
    SolveError(message.into())
}

/// Solve Laplace's equation in place, retaining every fixed value exactly.
///
/// At least one electrode is required to fix the Neumann nullspace. Free entries
/// are the initial guess. An exhausted iteration budget returns a report with
/// `converged = false`, not a silently accepted solution.
pub fn solve(
    potential: &mut [f64],
    electrodes: &[bool],
    grid: Grid,
    options: Options,
) -> Result<Report, SolveError> {
    solve_prepared(potential, electrodes, grid, options, None, None)
}

/// Solve `-L potential = source` with the same boundary conditions as [`solve`].
///
/// Source is the physical, unweighted `rho / epsilon_0`, in V/length².
/// For coordinates in mm and density in C/mm³, divide density by
/// `8.8541878128e-15` F/mm. Sources at fixed nodes are ignored.
pub fn solve_poisson(
    potential: &mut [f64],
    electrodes: &[bool],
    source: &[f64],
    grid: Grid,
    options: Options,
) -> Result<Report, SolveError> {
    if source.len() != potential.len() || source.iter().any(|v| !v.is_finite()) {
        return Err(invalid(
            "Poisson source must be finite and match the potential length",
        ));
    }
    solve_prepared(potential, electrodes, grid, options, None, Some(source))
}

pub(crate) fn solve_prepared(
    potential: &mut [f64],
    electrodes: &[bool],
    grid: Grid,
    options: Options,
    cuts: Option<&crate::cutcell::CutCells>,
    source: Option<&[f64]>,
) -> Result<Report, SolveError> {
    let start = Instant::now();
    // Enter the pool once, not once per sweep. A V-cycle contains hundreds of
    // short parallel regions interleaved with serial coarse work.
    if potential.len() <= 1_048_576 && rayon::current_num_threads() > 4 {
        // Extra sleeping workers make these sub-millisecond regions slower,
        // even when slab task counts are bounded. Reuse a small pool rather
        // than waking a machine-sized pool for a cache-resident problem.
        static SMALL_POOL: OnceLock<rayon::ThreadPool> = OnceLock::new();
        let pool = SMALL_POOL.get_or_init(|| {
            rayon::ThreadPoolBuilder::new()
                .num_threads(4)
                .build()
                .expect("CPU multigrid worker pool")
        });
        pool.install(|| solve_in_pool(potential, electrodes, grid, options, start, (cuts, source)))
    } else {
        rayon::scope(|_| solve_in_pool(potential, electrodes, grid, options, start, (cuts, source)))
    }
}

pub(crate) fn solve_cut(
    potential: &mut [f64],
    cuts: &crate::cutcell::CutCells,
    options: Options,
) -> Result<Report, SolveError> {
    if potential.len() != cuts.fixed.len() {
        return Err(invalid("SDF potential length mismatch"));
    }
    for (i, value) in potential.iter_mut().enumerate() {
        if cuts.fixed[i] {
            *value = cuts.values[i];
        }
    }
    solve_prepared(potential, &cuts.fixed, cuts.grid, options, Some(cuts), None)
}

fn solve_in_pool(
    potential: &mut [f64],
    electrodes: &[bool],
    grid: Grid,
    options: Options,
    start: Instant,
    data: (Option<&crate::cutcell::CutCells>, Option<&[f64]>),
) -> Result<Report, SolveError> {
    let (cuts, source) = data;
    let mut timings = Timings::default();
    let count = validate(potential, electrodes, grid, options)?;
    let origin = potential[electrodes.iter().position(|&fixed| fixed).unwrap()];
    let mut fine = Fine::new(grid, electrodes.to_vec());
    if let Some(cuts) = cuts {
        fine.cuts = Some((cuts.slots.clone(), cuts.rows.clone(), origin));
    }
    let operator = Operator::Fine(Box::new(fine));
    let x: Vec<_> = potential.iter().map(|v| v - origin).collect();
    let scale = operator.boundary_scale(&x, source);
    let mut root = Level {
        operator,
        x,
        rhs: source.map_or_else(Vec::new, <[f64]>::to_vec),
        work: vec![0.0; count],
        coarse: None,
    };
    let absolute = root.residual(&mut timings);
    let mut history = vec![Residual {
        absolute,
        relative: absolute / scale,
    }];
    let mut converged = reached(absolute, scale, options);
    if !converged && options.max_cycles > 0 {
        root.build_hierarchy();
    }
    timings.setup = start.elapsed();
    timings.residual = Duration::ZERO;
    let solve_start = Instant::now();
    let mut iterations = 0;
    while !converged && iterations < options.max_cycles {
        root.cycle(options, &mut timings);
        let absolute = root.residual(&mut timings);
        if !absolute.is_finite() {
            return Err(invalid("multigrid produced a non-finite residual"));
        }
        history.push(Residual {
            absolute,
            relative: absolute / scale,
        });
        iterations += 1;
        converged = reached(absolute, scale, options);
    }
    timings.solve = solve_start.elapsed();
    potential
        .par_iter_mut()
        .zip(&root.x)
        .zip(electrodes)
        .for_each(|((out, &value), &fixed)| {
            if !fixed {
                *out = value + origin;
            }
        });
    Ok(Report {
        converged,
        iterations,
        residual_history: history,
        elapsed: start.elapsed(),
        levels: root.depth(),
        worker_threads: rayon::current_num_threads(),
        timings,
    })
}

fn reached(absolute: f64, scale: f64, options: Options) -> bool {
    absolute <= options.absolute_tolerance + options.relative_tolerance * scale
}

pub(crate) fn validate(
    potential: &[f64],
    electrodes: &[bool],
    grid: Grid,
    options: Options,
) -> Result<usize, SolveError> {
    let [nx, ny, nz] = grid.dims;
    if nx < 2 || ny < 2 || nz == 0 {
        return Err(invalid("grid requires nx >= 2, ny >= 2, nz >= 1"));
    }
    let count = nx
        .checked_mul(ny)
        .and_then(|n| n.checked_mul(nz))
        .ok_or_else(|| invalid("grid size overflows usize"))?;
    if potential.len() != count || electrodes.len() != count {
        return Err(invalid(
            "potential and electrode lengths must match grid dimensions",
        ));
    }
    if !grid
        .spacing
        .iter()
        .all(|h| h.is_finite() && *h > 0.0 && (1.0 / (h * h)).is_finite() && 1.0 / (h * h) > 0.0)
    {
        return Err(invalid(
            "spacing must be finite, positive, and numerically representable",
        ));
    }
    if let Geometry::Cylindrical { radial_origin } = grid.geometry
        && (nz != 1 || !radial_origin.is_finite() || radial_origin < 0.0)
    {
        return Err(invalid(
            "cylindrical grids require nz = 1 and finite radial_origin >= 0",
        ));
    }
    if !potential.iter().all(|v| v.is_finite()) {
        return Err(invalid("potential values must be finite"));
    }
    if !electrodes.iter().any(|&fixed| fixed) {
        return Err(invalid(
            "at least one electrode is required to fix the potential gauge",
        ));
    }
    if !options.relative_tolerance.is_finite()
        || options.relative_tolerance < 0.0
        || !options.absolute_tolerance.is_finite()
        || options.absolute_tolerance < 0.0
        || options.relative_tolerance + options.absolute_tolerance <= 0.0
        || options.pre_smooth == 0
        || options.post_smooth == 0
    {
        return Err(invalid(
            "tolerances must be finite and nonnegative (one positive), smoothing counts positive",
        ));
    }
    Ok(count)
}

pub(crate) struct Axis {
    pub(crate) minus: Vec<f64>,
    pub(crate) plus: Vec<f64>,
    weight: Vec<f64>,
}

impl Axis {
    pub(crate) fn new(n: usize, h: f64, radial: Option<f64>) -> Self {
        let mut axis = Self {
            minus: vec![0.0; n],
            plus: vec![0.0; n],
            weight: vec![1.0; n],
        };
        if n == 1 {
            return axis;
        }
        let inv_h2 = 1.0 / (h * h);
        for i in 0..n {
            let (minus, plus, weight) = if let Some(origin) = radial {
                let r = origin / h + i as f64;
                if i == 0 && origin == 0.0 {
                    (0.0, 4.0, 0.125)
                } else if i == 0 {
                    (0.0, 2.0, (r + 0.5) * 0.5)
                } else if i == n - 1 {
                    (2.0, 0.0, (r - 0.5) * 0.5)
                } else {
                    (1.0 - 0.5 / r, 1.0 + 0.5 / r, r)
                }
            } else if i == 0 {
                (0.0, 2.0, 0.5)
            } else if i == n - 1 {
                (2.0, 0.0, 0.5)
            } else {
                (1.0, 1.0, 1.0)
            };
            axis.minus[i] = minus * inv_h2;
            axis.plus[i] = plus * inv_h2;
            axis.weight[i] = weight;
        }
        axis
    }
}

struct Fine {
    dims: [usize; 3],
    axes: [Axis; 3],
    fixed: Vec<bool>,
    planar: bool,
    cuts: Option<(Vec<u32>, Vec<crate::cutcell::CutRow>, f64)>,
}

impl Fine {
    fn new(grid: Grid, fixed: Vec<bool>) -> Self {
        Self {
            dims: grid.dims,
            axes: std::array::from_fn(|a| {
                let radial = match grid.geometry {
                    Geometry::Cylindrical { radial_origin } if a == 0 => Some(radial_origin),
                    _ => None,
                };
                Axis::new(grid.dims[a], grid.spacing[a], radial)
            }),
            fixed,
            planar: matches!(grid.geometry, Geometry::Planar),
            cuts: None,
        }
    }

    fn weight(&self, c: [usize; 3]) -> f64 {
        (0..3).map(|a| self.axes[a].weight[c[a]]).product()
    }

    fn diagonal(&self, c: [usize; 3]) -> f64 {
        if let Some(row) = self.cut(index(c, self.dims)) {
            return row.diagonal();
        }
        (0..3)
            .map(|a| self.axes[a].minus[c[a]] + self.axes[a].plus[c[a]])
            .sum()
    }

    fn neighbors(&self, i: usize, c: [usize; 3], mut visit: impl FnMut(usize, f64)) {
        let strides = [1, self.dims[0], self.dims[0] * self.dims[1]];
        if let Some(row) = self.cut(i) {
            for s in 0..6 {
                if row.neighbors[s] != 0.0 {
                    visit(
                        i.wrapping_add_signed(
                            strides[s / 2] as isize * if s % 2 == 0 { -1 } else { 1 },
                        ),
                        row.neighbors[s],
                    );
                }
            }
            return;
        }
        for a in 0..3 {
            if c[a] > 0 {
                visit(i - strides[a], self.axes[a].minus[c[a]]);
            }
            if c[a] + 1 < self.dims[a] {
                visit(i + strides[a], self.axes[a].plus[c[a]]);
            }
        }
    }

    fn cut(&self, i: usize) -> Option<&crate::cutcell::CutRow> {
        self.cuts.as_ref().and_then(|(slots, rows, _)| {
            let slot = slots[i];
            (slot > 0).then(|| &rows[slot as usize - 1])
        })
    }

    fn sweep<const RESIDUAL: bool>(
        &self,
        x: &[f64],
        rhs: &[f64],
        out: &mut [f64],
        omega: f64,
    ) -> f64 {
        let [nx, ny, nz] = self.dims;
        let plane = nx * ny;
        update_slabs(out, self.dims, 1, |start, out| {
            let mut norm = 0.0_f64;
            for (r, row) in out.chunks_mut(nx).enumerate() {
                let i = start + r * nx;
                let [_, y, z] = coords(i, self.dims);
                let center = &x[i..i + nx];
                let ym = if y > 0 { &x[i - nx..i] } else { center };
                let yp = if y + 1 < ny {
                    &x[i + nx..i + 2 * nx]
                } else {
                    center
                };
                let zm = if z > 0 {
                    &x[i - plane..i - plane + nx]
                } else {
                    center
                };
                let zp = if z + 1 < nz {
                    &x[i + plane..i + plane + nx]
                } else {
                    center
                };
                let my = self.axes[1].minus[y];
                let py = self.axes[1].plus[y];
                let mz = self.axes[2].minus[z];
                let pz = self.axes[2].plus[z];
                let wy = self.axes[1].weight[y];
                let wz = self.axes[2].weight[z];
                let planar_interior = self.planar && nx > 2;
                if planar_interior {
                    let cx = self.axes[0].minus[1];
                    let inverse = 1.0 / ((cx + cx) + (my + py) + (mz + pz));
                    let points = row[1..nx - 1]
                        .iter_mut()
                        .zip(center.windows(3))
                        .zip(ym[1..nx - 1].iter().zip(&yp[1..nx - 1]))
                        .zip(zm[1..nx - 1].iter().zip(&zp[1..nx - 1]))
                        .zip(&self.fixed[i + 1..i + nx - 1]);
                    for (offset, ((((out, w), (&ym, &yp)), (&zm, &zp)), &fixed)) in
                        points.enumerate()
                    {
                        if fixed {
                            *out = if RESIDUAL { 0.0 } else { w[1] };
                            continue;
                        }
                        let base = if RESIDUAL { w[1] } else { 0.0 };
                        let mut sum = rhs.get(i + 1 + offset).copied().unwrap_or(0.0);
                        sum += cx * ((w[0] - base) + (w[2] - base));
                        sum += my * (ym - base);
                        sum += py * (yp - base);
                        sum += mz * (zm - base);
                        sum += pz * (zp - base);
                        if RESIDUAL {
                            *out = wy * wz * sum;
                            if self.cut(i + 1 + offset).is_none() {
                                norm = norm.max(sum.abs());
                            }
                        } else {
                            *out = w[1] + omega * (sum * inverse - w[1]);
                        }
                    }
                }
                let ranges = if planar_interior {
                    [0..1, nx - 1..nx]
                } else {
                    [0..nx, nx..nx]
                };
                for cx in ranges.into_iter().flatten() {
                    let out = &mut row[cx];
                    if self.fixed[i + cx] {
                        *out = if RESIDUAL { 0.0 } else { center[cx] };
                        continue;
                    }
                    let mx = self.axes[0].minus[cx];
                    let px = self.axes[0].plus[cx];
                    let base = if RESIDUAL { center[cx] } else { 0.0 };
                    let mut sum = rhs.get(i + cx).copied().unwrap_or(0.0);
                    sum += mx * (center[cx.saturating_sub(1)] - base);
                    sum += px * (center[(cx + 1).min(nx - 1)] - base);
                    sum += my * (ym[cx] - base);
                    sum += py * (yp[cx] - base);
                    sum += mz * (zm[cx] - base);
                    sum += pz * (zp[cx] - base);
                    if RESIDUAL {
                        *out = self.axes[0].weight[cx] * wy * wz * sum;
                        if self.cut(i + cx).is_none() {
                            norm = norm.max(sum.abs());
                        }
                    } else {
                        let diagonal = (mx + px) + (my + py) + (mz + pz);
                        *out = center[cx] + omega * (sum / diagonal - center[cx]);
                    }
                }
            }
            if let Some((_, rows, gauge)) = &self.cuts {
                let strides = [1, nx, plane];
                let first = rows.partition_point(|row| row.node < start);
                for row in &rows[first..] {
                    if row.node >= start + out.len() {
                        break;
                    }
                    let r = row.residual(x, strides, *gauge)
                        + rhs.get(row.node).copied().unwrap_or(0.0);
                    out[row.node - start] = if RESIDUAL {
                        self.weight(coords(row.node, self.dims)) * r
                    } else {
                        x[row.node] + omega * r / row.diagonal()
                    };
                    if RESIDUAL {
                        norm = norm.max(r.abs());
                    }
                }
            }
            norm
        })
    }
}

enum Operator {
    Fine(Box<Fine>),
    Coarse {
        dims: [usize; 3],
        rows: Stencils,
        inverse_row_sums: Vec<f64>,
    },
}

// Regular Galerkin interiors share an identical stencil. Keep exceptional
// rows exactly, but avoid streaming 216 bytes of identical coefficients per
// point on every sweep. Irregular masks/geometry can still use dense storage.
enum Stencils {
    Dense(Vec<[f64; 27]>),
    Shared {
        common: Box<[f64; 27]>,
        symmetric: bool,
        indices: Vec<usize>,
        exceptions: Vec<[f64; 27]>,
    },
}

impl Stencils {
    fn new(rows: Vec<[f64; 27]>, dims: [usize; 3]) -> Self {
        let common = rows[index(dims.map(|n| n / 2), dims)];
        let shared = rows.iter().filter(|&&row| row == common).count();
        if shared < rows.len() / 2 {
            return Self::Dense(rows);
        }
        let mut indices = Vec::with_capacity(rows.len());
        let mut exceptions = Vec::with_capacity(rows.len() - shared);
        for row in rows {
            if row == common {
                indices.push(usize::MAX);
            } else {
                indices.push(exceptions.len());
                exceptions.push(row);
            }
        }
        Self::Shared {
            symmetric: reflection_symmetric(&common),
            common: Box::new(common),
            indices,
            exceptions,
        }
    }

    #[inline]
    fn get(&self, i: usize) -> &[f64; 27] {
        match self {
            Self::Dense(rows) => &rows[i],
            Self::Shared {
                common,
                indices,
                exceptions,
                ..
            } => {
                let slot = indices[i];
                if slot == usize::MAX {
                    common
                } else {
                    &exceptions[slot]
                }
            }
        }
    }

    fn product(&self, offsets: &[isize; 27], i: usize, x: &[f64]) -> f64 {
        if let Self::Shared {
            common,
            symmetric,
            indices,
            ..
        } = self
            && indices[i] == usize::MAX
            && i >= offsets[26] as usize
            && i + (offsets[26] as usize) < x.len()
        {
            let line = |s: usize| {
                let begin = i.wrapping_add_signed(offsets[s]);
                &x[begin..begin + 3]
            };
            let a = line(0);
            let b = line(3);
            let c = line(6);
            let d = line(9);
            let e = line(12);
            let f = line(15);
            let g = line(18);
            let h = line(21);
            let j = line(24);
            if !symmetric {
                let dot = |s: usize, line: &[f64]| {
                    common[s] * line[0] + common[s + 1] * line[1] + common[s + 2] * line[2]
                };
                return dot(0, a)
                    + dot(3, b)
                    + dot(6, c)
                    + dot(9, d)
                    + dot(12, e)
                    + dot(15, f)
                    + dot(18, g)
                    + dot(21, h)
                    + dot(24, j);
            }
            return common[13] * e[1]
                + common[14] * (e[0] + e[2])
                + common[16] * (d[1] + f[1])
                + common[22] * (b[1] + h[1])
                + common[17] * (d[0] + d[2] + f[0] + f[2])
                + common[23] * (b[0] + b[2] + h[0] + h[2])
                + common[25] * (a[1] + c[1] + g[1] + j[1])
                + common[26] * (a[0] + a[2] + c[0] + c[2] + g[0] + g[2] + j[0] + j[2]);
        }
        stencil_product(self.get(i), offsets, i, x)
    }
}

fn reflection_symmetric(row: &[f64; 27]) -> bool {
    (0..27).all(|s| {
        let x = if s % 3 == 1 { 1 } else { 2 };
        let y = if (s / 3) % 3 == 1 { 1 } else { 2 };
        let z = if s / 9 == 1 { 1 } else { 2 };
        row[s] == row[x + 3 * y + 9 * z]
    })
}

pub(crate) fn coords(i: usize, dims: [usize; 3]) -> [usize; 3] {
    [
        i % dims[0],
        (i / dims[0]) % dims[1],
        i / (dims[0] * dims[1]),
    ]
}

fn index(c: [usize; 3], dims: [usize; 3]) -> usize {
    c[0] + dims[0] * (c[1] + dims[1] * c[2])
}

// A task owns whole contiguous z slabs (x rows in 2D). In particular the
// coarsest 80-sweep solve must not launch a Rayon job for each tiny sweep.
fn update_slabs(
    values: &mut [f64],
    dims: [usize; 3],
    cost: usize,
    update: impl Fn(usize, &mut [f64]) -> f64 + Sync,
) -> f64 {
    let slab = dims[0] * dims[1];
    let chunk = if dims[2] > 1 { slab } else { dims[0] };
    let work = values.len() * cost;
    if work < 131_072 || rayon::current_num_threads() == 1 {
        return update(0, values);
    }
    let tasks = (work / 65_536).max(1).min(rayon::current_num_threads());
    let min_slabs = (values.len() / chunk / tasks).max(1);
    values
        .par_chunks_mut(chunk)
        .enumerate()
        .with_min_len(min_slabs)
        .map(|(i, values)| update(i * chunk, values))
        .reduce(|| 0.0, f64::max)
}

fn stencil_offsets(dims: [usize; 3]) -> [isize; 27] {
    std::array::from_fn(|s| {
        (s % 3) as isize - 1
            + dims[0] as isize * (((s / 3) % 3) as isize - 1)
            + (dims[0] * dims[1]) as isize * ((s / 9) as isize - 1)
    })
}

fn stencil_product(row: &[f64; 27], offsets: &[isize; 27], i: usize, x: &[f64]) -> f64 {
    let mut ax = 0.0;
    let halo = offsets[26] as usize;
    if i >= halo && i + halo < x.len() {
        for (r, coefficients) in row.chunks_exact(3).enumerate() {
            let begin = i.wrapping_add_signed(offsets[3 * r]);
            let neighbors = &x[begin..begin + 3];
            ax += coefficients[0] * neighbors[0];
            ax += coefficients[1] * neighbors[1];
            ax += coefficients[2] * neighbors[2];
        }
        return ax;
    }
    for (&coefficient, &offset) in row.iter().zip(offsets) {
        // Zero stencil entries include every neighbor outside the domain.
        if coefficient != 0.0 {
            ax += coefficient * x[i.wrapping_add_signed(offset)];
        }
    }
    ax
}

impl Operator {
    fn dims(&self) -> [usize; 3] {
        match self {
            Self::Fine(fine) => fine.dims,
            Self::Coarse { dims, .. } => *dims,
        }
    }

    fn active(&self, i: usize) -> bool {
        match self {
            Self::Fine(fine) => !fine.fixed[i],
            Self::Coarse { rows, .. } => rows.get(i)[13] > 0.0,
        }
    }

    /// Row of the symmetric correction operator, with fixed columns eliminated.
    #[cfg(test)]
    fn row(&self, i: usize, mut visit: impl FnMut(usize, f64)) {
        self.row_at(i, coords(i, self.dims()), |j, _, coefficient| {
            visit(j, coefficient)
        });
    }

    fn row_at(&self, i: usize, c: [usize; 3], mut visit: impl FnMut(usize, [usize; 3], f64)) {
        match self {
            Self::Fine(fine) => {
                if fine.fixed[i] {
                    return;
                }
                let weight = fine.weight(c);
                visit(i, c, weight * fine.diagonal(c));
                let strides = [1, fine.dims[0], fine.dims[0] * fine.dims[1]];
                if fine.cut(i).is_some() {
                    fine.neighbors(i, c, |j, coefficient| {
                        if !fine.fixed[j] {
                            visit(j, coords(j, fine.dims), -weight * coefficient);
                        }
                    });
                    return;
                }
                for a in 0..3 {
                    for (exists, offset, coefficient) in [
                        (c[a] > 0, -1, fine.axes[a].minus[c[a]]),
                        (c[a] + 1 < fine.dims[a], 1, fine.axes[a].plus[c[a]]),
                    ] {
                        if exists {
                            let j = i.wrapping_add_signed(offset * strides[a] as isize);
                            if !fine.fixed[j] {
                                let mut neighbor = c;
                                neighbor[a] = neighbor[a].wrapping_add_signed(offset);
                                visit(j, neighbor, -weight * coefficient);
                            }
                        }
                    }
                }
            }
            Self::Coarse { dims, rows, .. } => {
                for (s, &coefficient) in rows.get(i).iter().enumerate() {
                    if coefficient != 0.0 {
                        let neighbor = [
                            (c[0] as isize + (s % 3) as isize - 1) as usize,
                            (c[1] as isize + ((s / 3) % 3) as isize - 1) as usize,
                            (c[2] as isize + (s / 9) as isize - 1) as usize,
                        ];
                        visit(index(neighbor, *dims), neighbor, coefficient);
                    }
                }
            }
        }
    }

    fn boundary_scale(&self, x: &[f64], source: Option<&[f64]>) -> f64 {
        let Self::Fine(fine) = self else {
            unreachable!()
        };
        // Gauge-shifted reduced RHS. A constant boundary has zero RHS; use a
        // unit-voltage stencil scale then, so convergence remains meaningful.
        let rhs_scale = (0..x.len())
            .into_par_iter()
            .filter(|&i| !fine.fixed[i])
            .map(|i| {
                let mut rhs = source.map_or(0.0, |s| s[i]);
                if let Some(row) = fine.cut(i) {
                    rhs += row.source(fine.cuts.as_ref().unwrap().2);
                }
                fine.neighbors(i, coords(i, fine.dims), |j, coefficient| {
                    if fine.fixed[j] {
                        rhs += coefficient * x[j];
                    }
                });
                rhs.abs()
            })
            .reduce(|| 0.0, f64::max);
        if rhs_scale > 0.0 {
            rhs_scale
        } else {
            fine.axes.iter().map(|a| a.plus[0]).sum::<f64>().max(1.0)
        }
    }

    fn smooth(&self, x: &[f64], rhs: &[f64], out: &mut [f64], damping: f64) {
        match self {
            Self::Fine(fine) => {
                fine.sweep::<false>(x, rhs, out, damping);
            }
            Self::Coarse {
                dims,
                rows,
                inverse_row_sums,
            } => {
                let offsets = stencil_offsets(*dims);
                update_slabs(out, *dims, 8, |start, out| {
                    for (j, out) in out.iter_mut().enumerate() {
                        let i = start + j;
                        let row = rows.get(i);
                        let diagonal = row[13];
                        if diagonal == 0.0 {
                            *out = 0.0;
                            continue;
                        }
                        let ax = rows.product(&offsets, i, x);
                        *out = x[i] + damping * (rhs[i] - ax) * inverse_row_sums[i];
                    }
                    0.0
                });
            }
        }
    }

    /// Writes the weighted residual used by transpose restriction. Returns its
    /// unweighted infinity norm on the finest grid.
    fn residual(&self, x: &[f64], rhs: &[f64], out: &mut [f64]) -> f64 {
        match self {
            Self::Fine(fine) => fine.sweep::<true>(x, rhs, out, 0.0),
            Self::Coarse { dims, rows, .. } => {
                let offsets = stencil_offsets(*dims);
                update_slabs(out, *dims, 8, |start, out| {
                    let mut norm = 0.0_f64;
                    for (j, out) in out.iter_mut().enumerate() {
                        let i = start + j;
                        let row = rows.get(i);
                        if row[13] == 0.0 {
                            *out = 0.0;
                            continue;
                        }
                        let ax = rows.product(&offsets, i, x);
                        *out = rhs[i] - ax;
                        norm = norm.max(out.abs());
                    }
                    norm
                })
            }
        }
    }
}

#[derive(Clone, Copy)]
struct Interpolation {
    nodes: [(usize, f64); 2],
}

struct Transfer {
    fine_dims: [usize; 3],
    coarse_dims: [usize; 3],
    interpolation: [Vec<Interpolation>; 3],
    support: [Vec<Vec<(usize, f64)>>; 3],
}

impl Transfer {
    fn new(fine_dims: [usize; 3]) -> Self {
        let selected: [Vec<usize>; 3] = std::array::from_fn(|a| {
            let n = fine_dims[a];
            let mut nodes: Vec<_> = (0..n).step_by(2).collect();
            if *nodes.last().unwrap() != n - 1 {
                nodes.push(n - 1);
            }
            nodes
        });
        let coarse_dims = selected.each_ref().map(|v| v.len());
        let interpolation = std::array::from_fn(|a| {
            (0..fine_dims[a])
                .map(|i| {
                    let hi = selected[a].partition_point(|&node| node < i);
                    if selected[a][hi] == i {
                        Interpolation {
                            nodes: [(hi, 1.0), (hi, 0.0)],
                        }
                    } else {
                        let lo = hi - 1;
                        let t = (i - selected[a][lo]) as f64
                            / (selected[a][hi] - selected[a][lo]) as f64;
                        Interpolation {
                            nodes: [(lo, 1.0 - t), (hi, t)],
                        }
                    }
                })
                .collect::<Vec<_>>()
        });
        let support = std::array::from_fn(|a| {
            let mut support = vec![Vec::new(); coarse_dims[a]];
            for (i, entry) in interpolation[a].iter().enumerate() {
                for (node, weight) in entry.nodes {
                    if weight != 0.0 {
                        support[node].push((i, weight));
                    }
                }
            }
            support
        });
        Self {
            fine_dims,
            coarse_dims,
            interpolation,
            support,
        }
    }

    #[cfg(test)]
    fn interpolate_row(&self, i: usize, mut visit: impl FnMut(usize, f64)) {
        self.interpolate_at(coords(i, self.fine_dims), |c, weight| {
            visit(index(c, self.coarse_dims), weight)
        });
    }

    fn interpolate_at(&self, c: [usize; 3], mut visit: impl FnMut([usize; 3], f64)) {
        for (z, wz) in self.interpolation[2][c[2]].nodes {
            if wz == 0.0 {
                continue;
            }
            for (y, wy) in self.interpolation[1][c[1]].nodes {
                if wy == 0.0 {
                    continue;
                }
                for (x, wx) in self.interpolation[0][c[0]].nodes {
                    if wx != 0.0 {
                        visit([x, y, z], wx * wy * wz);
                    }
                }
            }
        }
    }

    fn support_row(&self, i: usize, mut visit: impl FnMut(usize, f64)) {
        self.support_at(coords(i, self.coarse_dims), |c, weight| {
            visit(index(c, self.fine_dims), weight)
        });
    }

    fn support_at(&self, c: [usize; 3], mut visit: impl FnMut([usize; 3], f64)) {
        for &(z, wz) in &self.support[2][c[2]] {
            for &(y, wy) in &self.support[1][c[1]] {
                for &(x, wx) in &self.support[0][c[0]] {
                    visit([x, y, z], wx * wy * wz);
                }
            }
        }
    }

    fn galerkin(&self, fine: &Operator) -> Operator {
        let count = self.coarse_dims.iter().product();
        let mut regular: Vec<_> = (0..count)
            .map(|i| {
                let c = coords(i, self.coarse_dims);
                (0..3).all(|a| {
                    self.coarse_dims[a] == 1 || (c[a] >= 2 && c[a] + 2 < self.coarse_dims[a])
                })
            })
            .collect();
        match fine {
            Operator::Fine(fine) if fine.planar => {
                // A fixed fine node changes rows whose interpolation support,
                // or a stencil neighbor of that support, reaches the node.
                // Mark a conservative box; thin electrodes are never sampled away.
                for (i, &fixed) in fine.fixed.iter().enumerate() {
                    if !fixed && fine.cut(i).is_none() {
                        continue;
                    }
                    let c = coords(i, self.fine_dims);
                    let lo = c.map(|v| v.saturating_sub(2) / 2);
                    let hi = std::array::from_fn::<_, 3, _>(|a| {
                        (c[a] + 2).div_ceil(2).min(self.coarse_dims[a] - 1)
                    });
                    for z in lo[2]..=hi[2] {
                        for y in lo[1]..=hi[1] {
                            for x in lo[0]..=hi[0] {
                                regular[index([x, y, z], self.coarse_dims)] = false;
                            }
                        }
                    }
                }
            }
            Operator::Coarse {
                rows: Stencils::Shared { indices, .. },
                ..
            } => {
                for (i, regular) in regular.iter_mut().enumerate() {
                    if *regular {
                        self.support_row(i, |j, _| *regular &= indices[j] == usize::MAX);
                    }
                }
            }
            _ => regular.fill(false),
        }
        let assemble = |i: usize, row: &mut [f64; 27]| {
            let c = coords(i, self.coarse_dims);
            self.support_at(c, |jc, left_weight| {
                let j = index(jc, self.fine_dims);
                if !fine.active(j) {
                    return;
                }
                fine.row_at(j, jc, |_, kc, coefficient| {
                    self.interpolate_at(kc, |lc, right_weight| {
                        let offset = [
                            (lc[0] as isize - c[0] as isize + 1) as usize,
                            (lc[1] as isize - c[1] as isize + 1) as usize,
                            (lc[2] as isize - c[2] as isize + 1) as usize,
                        ];
                        debug_assert!(offset.iter().all(|&v| v < 3));
                        row[offset[0] + 3 * offset[1] + 9 * offset[2]] +=
                            left_weight * coefficient * right_weight;
                    });
                });
            });
        };
        let common = regular.iter().position(|&value| value).map(|i| {
            let mut row = [0.0; 27];
            assemble(i, &mut row);
            row
        });
        let rows = if let Some(common) = common {
            let mut exceptional_nodes = Vec::new();
            let indices = regular
                .iter()
                .enumerate()
                .map(|(i, &regular)| {
                    if regular {
                        usize::MAX
                    } else {
                        let slot = exceptional_nodes.len();
                        exceptional_nodes.push(i);
                        slot
                    }
                })
                .collect();
            let mut exceptions = vec![[0.0; 27]; exceptional_nodes.len()];
            exceptions
                .par_iter_mut()
                .zip(&exceptional_nodes)
                .with_min_len(128)
                .for_each(|(row, &i)| assemble(i, row));
            Stencils::Shared {
                symmetric: reflection_symmetric(&common),
                common: Box::new(common),
                indices,
                exceptions,
            }
        } else {
            let mut rows = vec![[0.0; 27]; count];
            rows.par_iter_mut()
                .enumerate()
                .with_min_len(128)
                .for_each(|(i, row)| assemble(i, row));
            Stencils::new(rows, self.coarse_dims)
        };
        // A nearly empty interpolation column can have a tiny diagonal but
        // sizeable couplings. A global max(sum(abs(row))/diagonal) then freezes
        // every other row. Local absolute-row-sum scaling, also used on GPU,
        // bounds the symmetric preconditioned spectral radius by one.
        let inverse_row_sums = (0..count)
            .map(|i| {
                let sum: f64 = rows.get(i).iter().map(|v| v.abs()).sum();
                if sum > 0.0 { 1.0 / sum } else { 0.0 }
            })
            .collect();
        Operator::Coarse {
            dims: self.coarse_dims,
            rows,
            inverse_row_sums,
        }
    }
}

struct Level {
    operator: Operator,
    x: Vec<f64>,
    rhs: Vec<f64>,
    work: Vec<f64>,
    coarse: Option<(Transfer, Box<Level>)>,
}

impl Level {
    fn depth(&self) -> usize {
        1 + self.coarse.as_ref().map_or(0, |(_, next)| next.depth())
    }

    fn build_hierarchy(&mut self) {
        if self.operator.dims().iter().all(|&n| n <= 4) {
            return;
        }
        let transfer = Transfer::new(self.operator.dims());
        let operator = transfer.galerkin(&self.operator);
        let count = transfer.coarse_dims.iter().product();
        let mut coarse = Self {
            operator,
            x: vec![0.0; count],
            rhs: vec![0.0; count],
            work: vec![0.0; count],
            coarse: None,
        };
        coarse.build_hierarchy();
        self.coarse = Some((transfer, Box::new(coarse)));
    }

    fn smooth(&mut self, sweeps: usize, timings: &mut Timings) {
        let start = Instant::now();
        let (bound, jacobi) = match &self.operator {
            Operator::Fine(_) => (2.0, 2.0 / 3.0),
            Operator::Coarse { .. } => (1.0, 4.0 / 3.0),
        };
        // An axis stuck at two nodes keeps contributing to the diagonal while
        // the other axes coarsen. Include their weaker modes in the band.
        let (center, radius) = if self.operator.dims().contains(&2) {
            (0.525, 0.475)
        } else {
            (0.575, 0.425)
        };
        let mut remaining = sweeps;
        while remaining > 0 {
            // A degree <=3 Chebyshev-Jacobi polynomial damps the upper
            // spectral band with the same sweeps and two existing buffers.
            // The tiny bottom solve retains its stationary Jacobi iteration.
            let batch = remaining.min(3);
            for k in 0..batch {
                let damping = if self.coarse.is_some() {
                    let angle = std::f64::consts::PI * (2 * k + 1) as f64 / (2 * batch) as f64;
                    1.0 / (bound * (center + radius * angle.cos()))
                } else {
                    jacobi
                };
                self.operator
                    .smooth(&self.x, &self.rhs, &mut self.work, damping);
                std::mem::swap(&mut self.x, &mut self.work);
            }
            remaining -= batch;
        }
        let elapsed = start.elapsed();
        timings.smoothing += elapsed;
    }

    fn residual(&mut self, timings: &mut Timings) -> f64 {
        let start = Instant::now();
        let norm = self.operator.residual(&self.x, &self.rhs, &mut self.work);
        timings.residual += start.elapsed();
        norm
    }

    fn cycle(&mut self, options: Options, timings: &mut Timings) {
        if self.coarse.is_none() {
            self.smooth(80, timings);
            return;
        }
        self.smooth(options.pre_smooth, timings);
        self.residual(timings);
        let start = Instant::now();
        let (transfer, coarse) = self.coarse.as_mut().unwrap();
        update_slabs(&mut coarse.rhs, transfer.coarse_dims, 4, |start, rhs| {
            for (r, row) in rhs.chunks_mut(transfer.coarse_dims[0]).enumerate() {
                let i = start + r * transfer.coarse_dims[0];
                let [_, y, z] = coords(i, transfer.coarse_dims);
                for (x, rhs) in row.iter_mut().enumerate() {
                    let mut sum = 0.0;
                    transfer.support_at([x, y, z], |c, weight| {
                        sum += weight * self.work[index(c, transfer.fine_dims)];
                    });
                    *rhs = sum;
                }
            }
            0.0
        });
        coarse.x.fill(0.0);
        timings.restriction += start.elapsed();
        coarse.cycle(options, timings);
        let start = Instant::now();
        update_slabs(&mut self.x, transfer.fine_dims, 2, |start, x| {
            for (r, row) in x.chunks_mut(transfer.fine_dims[0]).enumerate() {
                let i = start + r * transfer.fine_dims[0];
                let [_, y, z] = coords(i, transfer.fine_dims);
                for (cx, x) in row.iter_mut().enumerate() {
                    if self.operator.active(i + cx) {
                        transfer.interpolate_at([cx, y, z], |c, weight| {
                            *x += weight * coarse.x[index(c, transfer.coarse_dims)];
                        });
                    }
                }
            }
            0.0
        });
        timings.prolongation += start.elapsed();
        self.smooth(options.post_smooth, timings);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cut_galerkin_preserves_symmetry_and_positive_energy() {
        use simrep_geom::{Electrode, Sdf};
        for geometry in [
            Geometry::Planar,
            Geometry::Cylindrical { radial_origin: 0.0 },
        ] {
            let grid = Grid {
                dims: [25, 21, 1],
                spacing: [0.05, 0.06, 1.0],
                geometry,
            };
            let cuts = crate::cutcell::CutCells::new(
                grid,
                [0.0; 3],
                &[
                    Electrode::new(
                        0.0,
                        Sdf::Sphere {
                            center: [0.53, 0.49, 0.0],
                            radius: 0.23,
                        },
                    ),
                    Electrode::new(
                        1.0,
                        Sdf::Plane {
                            normal: [1.0, 0.0, 0.0],
                            offset: 0.0,
                        },
                    ),
                ],
            )
            .unwrap();
            let mut fine = Fine::new(grid, cuts.fixed.clone());
            fine.cuts = Some((cuts.slots.clone(), cuts.rows.clone(), 0.0));
            let mut operator = Operator::Fine(Box::new(fine));
            for _ in 0..3 {
                let count = operator.dims().iter().product();
                let x: Vec<f64> = (0..count)
                    .map(|i| {
                        if operator.active(i) {
                            (i as f64 * 0.13).sin()
                        } else {
                            0.0
                        }
                    })
                    .collect();
                let y: Vec<f64> = (0..count)
                    .map(|i| {
                        if operator.active(i) {
                            (i as f64 * 0.23).cos()
                        } else {
                            0.0
                        }
                    })
                    .collect();
                let (mut xy, mut yx, mut energy) = (0.0, 0.0, 0.0);
                for i in 0..count {
                    operator.row(i, |j, a| {
                        xy += x[i] * a * y[j];
                        yx += y[i] * a * x[j];
                        energy += x[i] * a * x[j];
                    });
                }
                assert!(energy > 0.0);
                assert!((xy - yx).abs() < 1e-12 * energy);
                let transfer = Transfer::new(operator.dims());
                let coarse = transfer.galerkin(&operator);
                let cn = transfer.coarse_dims.iter().product();
                let v: Vec<f64> = (0..cn).map(|i| (i as f64 * 0.31).sin()).collect();
                let mut pv = vec![0.0; count];
                for (i, p) in pv.iter_mut().enumerate() {
                    if operator.active(i) {
                        transfer.interpolate_row(i, |j, w| *p += w * v[j]);
                    }
                }
                let (mut fe, mut ce) = (0.0, 0.0);
                for i in 0..count {
                    operator.row(i, |j, a| fe += pv[i] * a * pv[j]);
                }
                for i in 0..cn {
                    coarse.row(i, |j, a| ce += v[i] * a * v[j]);
                }
                assert!((fe - ce).abs() < 1e-12 * fe.abs());
                operator = coarse;
            }
        }
    }

    #[test]
    fn mirrored_octant_shells_converge_with_large_fixed_regions() {
        let dims = [65; 3];
        let grid = Grid {
            dims,
            spacing: [1.0; 3],
            geometry: Geometry::Planar,
        };
        let mut potential = vec![0.0; dims.iter().product()];
        let fixed: Vec<_> = potential
            .iter_mut()
            .enumerate()
            .map(|(i, value)| {
                let radius_squared: usize = coords(i, dims).iter().map(|v| v * v).sum();
                *value = f64::from(radius_squared >= 60 * 60);
                radius_squared <= 40 * 40 || radius_squared >= 60 * 60
            })
            .collect();
        let initial = potential.clone();
        let report = solve(
            &mut potential,
            &fixed,
            grid,
            Options {
                max_cycles: 32,
                ..Options::default()
            },
        )
        .unwrap();
        eprintln!(
            "octant shells: {} cycles, residual {:?}",
            report.iterations,
            report.residual_history.last().unwrap()
        );
        assert!(report.converged, "{:?}", report.residual_history);
        for ((value, &fixed), original) in potential.iter().zip(&fixed).zip(initial) {
            if fixed {
                assert_eq!(*value, original);
            }
        }
    }

    #[test]
    fn shared_and_dense_stencils_retain_every_coefficient() {
        let dims = [9, 7, 5];
        let count = dims.iter().product();
        for regular in [false, true] {
            let rows: Vec<[f64; 27]> = (0..count)
                .map(|i| {
                    std::array::from_fn(|s| {
                        let scale = if regular && i % 7 != 0 { 1 } else { i + 2 };
                        scale as f64 * (s + 1) as f64
                    })
                })
                .collect();
            let stencils = Stencils::new(rows.clone(), dims);
            assert_eq!(matches!(stencils, Stencils::Shared { .. }), regular);
            for (i, row) in rows.iter().enumerate() {
                assert_eq!(stencils.get(i).map(f64::to_bits), row.map(f64::to_bits));
            }
        }
    }

    #[test]
    fn regular_region_reuse_matches_full_masked_galerkin_assembly() {
        let dims = [22, 19, 16];
        let count = dims.iter().product();
        let fixed: Vec<_> = (0..count)
            .map(|i| {
                let [x, y, z] = coords(i, dims);
                x == 0 || (x == 11 && y == 8 && (5..9).contains(&z))
            })
            .collect();
        let fine = Operator::Fine(Box::new(Fine::new(
            Grid {
                dims,
                spacing: [0.3, 0.4, 0.5],
                geometry: Geometry::Planar,
            },
            fixed,
        )));
        let mut transfer = Transfer::new(dims);
        let mut operator = fine;
        for _ in 0..2 {
            let coarse = transfer.galerkin(&operator);
            let Operator::Coarse { rows, .. } = &coarse else {
                unreachable!()
            };
            for i in 0..transfer.coarse_dims.iter().product() {
                let c = coords(i, transfer.coarse_dims);
                let mut expected = [0.0; 27];
                transfer.support_row(i, |j, left| {
                    if operator.active(j) {
                        operator.row(j, |k, coefficient| {
                            transfer.interpolate_row(k, |l, right| {
                                let lc = coords(l, transfer.coarse_dims);
                                let offset = std::array::from_fn::<_, 3, _>(|a| lc[a] + 1 - c[a]);
                                expected[index(offset, [3; 3])] += left * coefficient * right;
                            });
                        });
                    }
                });
                assert_eq!(rows.get(i).map(f64::to_bits), expected.map(f64::to_bits));
            }
            transfer = Transfer::new(transfer.coarse_dims);
            operator = coarse;
        }
    }

    #[test]
    fn masked_galerkin_preserves_energy_and_symmetry() {
        for (dims, geometry) in [
            ([10, 9, 7], Geometry::Planar),
            ([10, 9, 1], Geometry::Cylindrical { radial_origin: 0.0 }),
        ] {
            let count = dims.iter().product();
            let fixed: Vec<_> = (0..count).map(|i| i % 13 == 0 || i % 17 == 3).collect();
            let fine = Operator::Fine(Box::new(Fine::new(
                Grid {
                    dims,
                    spacing: [0.3, 0.4, 0.5],
                    geometry,
                },
                fixed,
            )));
            let transfer = Transfer::new(dims);
            let coarse = transfer.galerkin(&fine);
            let n = transfer.coarse_dims.iter().product();
            let x: Vec<_> = (0..n).map(|i| (i as f64 * 0.17).sin()).collect();
            let y: Vec<_> = (0..n).map(|i| (i as f64 * 0.23).cos()).collect();
            let mut px = vec![0.0; count];
            for (i, value) in px.iter_mut().enumerate() {
                if fine.active(i) {
                    transfer.interpolate_row(i, |j, weight| *value += weight * x[j]);
                }
            }
            let mut fine_energy = 0.0;
            for i in 0..count {
                fine.row(i, |j, coefficient| {
                    fine_energy += px[i] * coefficient * px[j]
                });
            }
            let (mut energy, mut xy, mut yx) = (0.0, 0.0, 0.0);
            for i in 0..n {
                coarse.row(i, |j, coefficient| {
                    energy += x[i] * coefficient * x[j];
                    xy += x[i] * coefficient * y[j];
                    yx += y[i] * coefficient * x[j];
                });
            }
            assert!((energy - fine_energy).abs() < 1e-11 * fine_energy.abs());
            assert!((xy - yx).abs() < 1e-11 * fine_energy.abs());
            assert!(energy > 0.0);
        }
    }
}
