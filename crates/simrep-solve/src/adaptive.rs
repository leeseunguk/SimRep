//! Conforming variational nodal refinement of Cartesian cut-cell grids.
//!
//! Cells partition the domain, and their edge energies partition the existing
//! volume-weighted nodal operator. Hanging nodes are affine coarse interpolants:
//! assembling each energy after substitution is exactly P^T A_f P. Interface
//! conservation is weak (against every coarse trace basis), not facewise.

use std::collections::BTreeMap;
use std::time::Instant;

use simrep_geom::Electrode;

use crate::cutcell::CutCells;
use crate::mg::{self, Geometry, Grid, Options, Report, Residual, SolveError, Timings};

#[derive(Clone, Copy, Debug)]
pub struct Region {
    pub bounds_mm: [[f64; 3]; 2],
    pub factor: usize,
}

#[derive(Clone)]
pub struct Patch {
    pub lo: [usize; 3],
    pub hi: [usize; 3],
    pub factor: usize,
    pub cuts: CutCells,
}

#[derive(Clone)]
pub struct AdaptiveGrid {
    pub base: CutCells,
    pub patches: Vec<Patch>,
}

pub struct Solution {
    pub base: Vec<f64>,
    pub patches: Vec<Vec<f64>>,
    pub report: Report,
    pub unknowns: usize,
    /// CSR matrix and load only, not total process RSS.
    pub matrix_bytes: usize,
    #[cfg(test)]
    matrix: Matrix,
    #[cfg(test)]
    locations: Vec<[f64; 3]>,
}

#[derive(Clone, Default)]
struct Node {
    terms: Vec<(usize, f64)>,
    value: f64,
}

impl Node {
    fn sample(&self, x: &[f64]) -> f64 {
        self.value + self.terms.iter().map(|&(i, w)| w * x[i]).sum::<f64>()
    }
}

fn index(c: [usize; 3], dims: [usize; 3]) -> usize {
    c[0] + dims[0] * (c[1] + dims[1] * c[2])
}

fn inside(c: [usize; 3], patch: &Patch, strict: bool, dims: [usize; 3]) -> bool {
    (0..3).all(|a| {
        dims[a] == 1
            || if strict {
                (c[a] > patch.lo[a] || patch.lo[a] == 0)
                    && (c[a] < patch.hi[a] || patch.hi[a] + 1 == dims[a])
            } else {
                c[a] >= patch.lo[a] && c[a] <= patch.hi[a]
            }
    })
}

/// Tensor-linear interpolation, also used to fill exported patch ghost layers.
pub fn interpolate(values: &[f64], grid: Grid, origin: [f64; 3], p: [f64; 3]) -> f64 {
    interpolation(grid, origin, p)
        .into_iter()
        .map(|(i, w)| values[i] * w)
        .sum()
}

fn interpolation(grid: Grid, origin: [f64; 3], p: [f64; 3]) -> Vec<(usize, f64)> {
    let lo: [usize; 3] = std::array::from_fn(|a| {
        if grid.dims[a] == 1 {
            0
        } else {
            (((p[a] - origin[a]) / grid.spacing[a]).floor().max(0.0) as usize).min(grid.dims[a] - 2)
        }
    });
    let t: [f64; 3] = std::array::from_fn(|a| {
        if grid.dims[a] == 1 {
            0.0
        } else {
            ((p[a] - origin[a]) / grid.spacing[a] - lo[a] as f64).clamp(0.0, 1.0)
        }
    });
    let mut result = Vec::new();
    for z in 0..=usize::from(grid.dims[2] > 1) {
        for y in 0..=usize::from(grid.dims[1] > 1) {
            for x in 0..=usize::from(grid.dims[0] > 1) {
                let d = [x, y, z];
                let w: f64 = (0..3)
                    .map(|a| if d[a] == 0 { 1.0 - t[a] } else { t[a] })
                    .product();
                if w > 0.0 {
                    result.push((index(std::array::from_fn(|a| lo[a] + d[a]), grid.dims), w));
                }
            }
        }
    }
    result
}

// Interpolating stored metal voltages across a cut would move the surface
// to the metal node and introduce O(h) trace error. Continue the vacuum
// linear polynomial through that node using the actual cut distance.
// This remains an affine interpolation of coarse unknowns, so elimination
// still assembles P^T A P and cannot introduce nonsymmetric interface rows.
fn trace_node(i: usize, support: &[(usize, f64)], cuts: &CutCells, coarse: &[Node]) -> Node {
    if !cuts.fixed[i] {
        return coarse[i].clone();
    }
    let c = mg::coords(i, cuts.grid.dims);
    let mut selected: Option<(usize, f64, f64)> = None;
    for &(j, _) in support {
        if cuts.fixed[j] || cuts.slots[j] == 0 {
            continue;
        }
        let d = mg::coords(j, cuts.grid.dims);
        let axes: Vec<_> = (0..3).filter(|&a| c[a] != d[a]).collect();
        if axes.len() != 1 || c[axes[0]].abs_diff(d[axes[0]]) != 1 {
            continue;
        }
        let a = axes[0];
        let side = 2 * a + usize::from(c[a] > d[a]);
        let row = &cuts.rows[cuts.slots[j] as usize - 1];
        if row.boundary[side] == 0.0 {
            continue;
        }
        let mirror = if d[a] == 0 || d[a] + 1 == cuts.grid.dims[a] {
            2.0
        } else {
            1.0
        };
        let theta = mirror / (cuts.grid.spacing[a].powi(2) * row.boundary[side]);
        if selected.is_none_or(|(_, t, _)| theta > t) {
            selected = Some((j, theta, row.voltage[side]));
        }
    }
    selected.map_or_else(
        || coarse[i].clone(),
        |(j, theta, voltage)| Node {
            terms: coarse[j]
                .terms
                .iter()
                .map(|&(k, w)| (k, w * (1.0 - 1.0 / theta)))
                .collect(),
            value: coarse[j].value * (1.0 - 1.0 / theta) + voltage / theta,
        },
    )
}

impl AdaptiveGrid {
    pub fn new(
        base: CutCells,
        regions: &[Region],
        electrodes: &[Electrode],
    ) -> Result<Self, SolveError> {
        if base.grid.geometry != Geometry::Planar || regions.is_empty() {
            return Err(mg::invalid(
                "adaptive refinement requires Cartesian geometry and a region",
            ));
        }
        let mut patches: Vec<Patch> = Vec::new();
        for region in regions {
            if ![2, 4].contains(&region.factor) {
                return Err(mg::invalid("refinement factor must be 2 or 4"));
            }
            let mut bounds = [[0; 3]; 2];
            for (side, indices) in bounds.iter_mut().enumerate() {
                for (a, coordinate) in indices.iter_mut().enumerate() {
                    let q = (region.bounds_mm[side][a] - base.origin[a]) / base.grid.spacing[a];
                    if !q.is_finite()
                        || (q - q.round()).abs() > 1e-8
                        || q < 0.0
                        || q > (base.grid.dims[a] - 1) as f64
                    {
                        return Err(mg::invalid(
                            "patch bounds must be base nodes within the domain",
                        ));
                    }
                    *coordinate = q.round() as usize;
                }
            }
            if (0..3).any(|a| base.grid.dims[a] > 1 && bounds[0][a] >= bounds[1][a]) {
                return Err(mg::invalid("patch bounds must increase on active axes"));
            }
            if patches.iter().any(|p| {
                (0..3).all(|a| {
                    base.grid.dims[a] == 1 || (bounds[0][a] <= p.hi[a] && bounds[1][a] >= p.lo[a])
                })
            }) {
                return Err(mg::invalid("adaptive patches must not overlap or touch"));
            }
            let dims = std::array::from_fn(|a| (bounds[1][a] - bounds[0][a]) * region.factor + 1);
            let spacing = std::array::from_fn(|a| {
                base.grid.spacing[a]
                    / if dims[a] > 1 {
                        region.factor as f64
                    } else {
                        1.0
                    }
            });
            let origin = std::array::from_fn(|a| {
                base.origin[a] + bounds[0][a] as f64 * base.grid.spacing[a]
            });
            let cuts = CutCells::new_patch(
                Grid {
                    dims,
                    spacing,
                    geometry: Geometry::Planar,
                },
                origin,
                electrodes,
            )?;
            patches.push(Patch {
                lo: bounds[0],
                hi: bounds[1],
                factor: region.factor,
                cuts,
            });
        }
        Ok(Self { base, patches })
    }

    pub fn with_voltages(&self, voltage: impl Fn(f64) -> f64 + Copy) -> Result<Self, SolveError> {
        Ok(Self {
            base: self.base.with_voltages(voltage)?,
            patches: self
                .patches
                .iter()
                .map(|p| {
                    Ok(Patch {
                        cuts: p.cuts.with_voltages(voltage)?,
                        ..p.clone()
                    })
                })
                .collect::<Result<_, SolveError>>()?,
        })
    }

    pub fn solve(&self, options: Options) -> Result<Solution, SolveError> {
        self.solve_poisson(options, |_| 0.0)
    }

    pub fn solve_poisson(
        &self,
        options: Options,
        source: impl Fn([f64; 3]) -> f64,
    ) -> Result<Solution, SolveError> {
        let start = Instant::now();
        if !options.relative_tolerance.is_finite()
            || options.relative_tolerance <= 0.0
            || !options.absolute_tolerance.is_finite()
            || options.absolute_tolerance < 0.0
        {
            return Err(mg::invalid(
                "adaptive relative tolerance must be finite and positive",
            ));
        }
        let mut count = 0;
        let mut coarse = vec![Node::default(); self.base.values.len()];
        let mut locations = Vec::new();
        for (i, node) in coarse.iter_mut().enumerate() {
            let c = mg::coords(i, self.base.grid.dims);
            if self.base.fixed[i] {
                node.value = self.base.values[i];
            } else if !self
                .patches
                .iter()
                .any(|p| inside(c, p, true, self.base.grid.dims))
            {
                node.terms.push((count, 1.0));
                locations.push(point(&self.base, i));
                count += 1;
            }
        }
        let mut fine = Vec::new();
        for patch in &self.patches {
            let mut nodes = vec![Node::default(); patch.cuts.values.len()];
            for (i, node) in nodes.iter_mut().enumerate() {
                let c = mg::coords(i, patch.cuts.grid.dims);
                if patch.cuts.fixed[i] {
                    node.value = patch.cuts.values[i];
                } else if (0..3).any(|a| {
                    patch.cuts.grid.dims[a] > 1
                        && ((c[a] == 0 && patch.lo[a] > 0)
                            || (c[a] + 1 == patch.cuts.grid.dims[a]
                                && patch.hi[a] + 1 < self.base.grid.dims[a]))
                }) {
                    let support =
                        interpolation(self.base.grid, self.base.origin, point(&patch.cuts, i));
                    for &(j, w) in &support {
                        let trace = trace_node(j, &support, &self.base, &coarse);
                        node.value += w * trace.value;
                        node.terms
                            .extend(trace.terms.iter().map(|&(k, v)| (k, w * v)));
                    }
                } else {
                    node.terms.push((count, 1.0));
                    locations.push(point(&patch.cuts, i));
                    count += 1;
                }
            }
            fine.push(nodes);
        }
        let mut matrix = MatrixBuilder {
            rows: vec![BTreeMap::new(); count],
            rhs: vec![0.0; count],
        };
        assemble(&self.base, &coarse, &mut matrix, &source, |c| {
            !self.patches.iter().any(|p| {
                (0..3).all(|a| self.base.grid.dims[a] == 1 || (c[a] >= p.lo[a] && c[a] < p.hi[a]))
            })
        })?;
        for (patch, nodes) in self.patches.iter().zip(&fine) {
            assemble(&patch.cuts, nodes, &mut matrix, &source, |_| true)?;
        }
        let matrix = matrix.finish();
        if matrix.diagonal.iter().any(|&d| d <= 0.0 || !d.is_finite()) {
            return Err(mg::invalid(
                "adaptive operator has an unconstrained degree of freedom",
            ));
        }
        let transfer: Vec<_> = locations
            .iter()
            .map(|&p| {
                interpolation(self.base.grid, self.base.origin, p)
                    .into_iter()
                    .filter(|&(i, _)| !self.base.fixed[i])
                    .collect::<Vec<_>>()
            })
            .collect();
        let mut cycle = mg::CorrectionCycle::new(&self.base);
        let setup = start.elapsed();
        let mut x = vec![0.0; count];
        let mut r = matrix.rhs.clone();
        let initial = norm(&r);
        let mut history = vec![Residual {
            absolute: initial,
            relative: if initial > 0.0 { 1.0 } else { 0.0 },
        }];
        let mut z = vec![0.0; count];
        let mut coarse_rhs = vec![0.0; coarse.len()];
        let volume: f64 = (0..3)
            .filter(|&a| self.base.grid.dims[a] > 1)
            .map(|a| self.base.grid.spacing[a])
            .product();
        let mut precondition = |r: &[f64], z: &mut [f64]| {
            coarse_rhs.fill(0.0);
            for (row, &v) in transfer.iter().zip(r) {
                for &(j, w) in row {
                    coarse_rhs[j] += w * v;
                }
            }
            let correction = cycle.apply(&coarse_rhs);
            for i in 0..count {
                z[i] = r[i] / matrix.diagonal[i]
                    + transfer[i]
                        .iter()
                        .map(|&(j, w)| w * correction[j] / volume)
                        .sum::<f64>();
            }
        };
        precondition(&r, &mut z);
        let mut direction = z.clone();
        let mut rz = dot(&r, &z);
        let mut product = vec![0.0; count];
        let threshold = options
            .absolute_tolerance
            .max(initial * options.relative_tolerance);
        let mut iterations = 0;
        while norm(&r) > threshold && iterations < options.max_cycles {
            matrix.apply(&direction, &mut product);
            let curvature = dot(&direction, &product);
            if curvature <= 0.0 || !curvature.is_finite() || rz <= 0.0 {
                return Err(mg::invalid("adaptive CG lost positive definiteness"));
            }
            let alpha = rz / curvature;
            for i in 0..count {
                x[i] += alpha * direction[i];
            }
            // Recompute the true residual; recurrence drift must not certify convergence.
            matrix.apply(&x, &mut product);
            for i in 0..count {
                r[i] = matrix.rhs[i] - product[i];
            }
            iterations += 1;
            history.push(Residual {
                absolute: norm(&r),
                relative: norm(&r) / initial,
            });
            if norm(&r) <= threshold {
                break;
            }
            precondition(&r, &mut z);
            let next = dot(&r, &z);
            let beta = next / rz;
            for i in 0..count {
                direction[i] = z[i] + beta * direction[i];
            }
            rz = next;
        }
        let patch_values: Vec<Vec<f64>> = fine
            .iter()
            .map(|nodes| nodes.iter().map(|n| n.sample(&x)).collect())
            .collect();
        let mut base: Vec<_> = coarse.iter().map(|n| n.sample(&x)).collect();
        for (i, value) in base.iter_mut().enumerate() {
            let c = mg::coords(i, self.base.grid.dims);
            if let Some((p, values)) = self
                .patches
                .iter()
                .zip(&patch_values)
                .find(|(p, _)| inside(c, p, true, self.base.grid.dims))
            {
                *value = interpolate(values, p.cuts.grid, p.cuts.origin, point(&self.base, i));
            }
        }
        Ok(Solution {
            base,
            patches: patch_values,
            unknowns: count,
            matrix_bytes: matrix.entries.len() * std::mem::size_of::<(usize, f64)>()
                + (matrix.offsets.len() + count) * std::mem::size_of::<usize>()
                + count * 8,
            report: Report {
                converged: norm(&r) <= threshold,
                iterations,
                residual_history: history,
                elapsed: start.elapsed(),
                levels: 1,
                worker_threads: 1,
                timings: Timings {
                    setup,
                    solve: start.elapsed() - setup,
                    ..Default::default()
                },
            },
            #[cfg(test)]
            matrix,
            #[cfg(test)]
            locations,
        })
    }
}

fn point(cuts: &CutCells, i: usize) -> [f64; 3] {
    let c = mg::coords(i, cuts.grid.dims);
    std::array::from_fn(|a| cuts.origin[a] + c[a] as f64 * cuts.grid.spacing[a])
}

struct MatrixBuilder {
    rows: Vec<BTreeMap<usize, f64>>,
    rhs: Vec<f64>,
}

impl MatrixBuilder {
    fn energy(&mut self, left: &Node, right: &Node, coefficient: f64) {
        let mut terms = left.terms.clone();
        terms.extend(right.terms.iter().map(|&(i, w)| (i, -w)));
        let offset = left.value - right.value;
        for &(i, wi) in &terms {
            self.rhs[i] -= coefficient * wi * offset;
            for &(j, wj) in &terms {
                *self.rows[i].entry(j).or_default() += coefficient * wi * wj;
            }
        }
    }

    fn finish(self) -> Matrix {
        let mut offsets = vec![0];
        let mut entries = Vec::new();
        let mut diagonal = Vec::new();
        for (i, row) in self.rows.into_iter().enumerate() {
            diagonal.push(*row.get(&i).unwrap_or(&0.0));
            entries.extend(row.into_iter().filter(|&(_, v)| v != 0.0));
            offsets.push(entries.len());
        }
        Matrix {
            offsets,
            entries,
            diagonal,
            rhs: self.rhs,
        }
    }
}

struct Matrix {
    offsets: Vec<usize>,
    entries: Vec<(usize, f64)>,
    diagonal: Vec<f64>,
    rhs: Vec<f64>,
}

impl Matrix {
    fn apply(&self, x: &[f64], out: &mut [f64]) {
        for (i, value) in out.iter_mut().enumerate() {
            *value = self.entries[self.offsets[i]..self.offsets[i + 1]]
                .iter()
                .map(|&(j, a)| a * x[j])
                .sum();
        }
    }
}

fn dot(x: &[f64], y: &[f64]) -> f64 {
    x.iter().zip(y).map(|(x, y)| x * y).sum()
}
fn norm(x: &[f64]) -> f64 {
    x.iter().fold(0.0_f64, |m, v| m.max(v.abs()))
}

fn assemble(
    cuts: &CutCells,
    nodes: &[Node],
    matrix: &mut MatrixBuilder,
    source: &impl Fn([f64; 3]) -> f64,
    keep: impl Fn([usize; 3]) -> bool,
) -> Result<(), SolveError> {
    let dims = cuts.grid.dims;
    let active: Vec<_> = (0..3).filter(|&a| dims[a] > 1).collect();
    let cell_dims = dims.map(|n| (n - 1).max(1));
    let volume: f64 = active.iter().map(|&a| cuts.grid.spacing[a]).product();
    let corners = 1usize << active.len();
    for cell in 0..cell_dims.iter().product() {
        let c = mg::coords(cell, cell_dims);
        if !keep(c) {
            continue;
        }
        for corner in 0..corners {
            let mut p = c;
            for (bit, &a) in active.iter().enumerate() {
                p[a] += (corner >> bit) & 1;
            }
            let i = index(p, dims);
            let load = source(point(cuts, i)) * volume / corners as f64;
            if !load.is_finite() {
                return Err(mg::invalid("adaptive Poisson source must be finite"));
            }
            for &(j, w) in &nodes[i].terms {
                matrix.rhs[j] += w * load;
            }
            for (bit, &a) in active.iter().enumerate() {
                if corner & (1 << bit) != 0 {
                    continue;
                }
                let mut q = p;
                q[a] += 1;
                let j = index(q, dims);
                let coefficient = volume / ((corners / 2) as f64 * cuts.grid.spacing[a].powi(2));
                let mut cut = false;
                for (node, side) in [(i, 2 * a + 1), (j, 2 * a)] {
                    if cuts.fixed[node] {
                        continue;
                    }
                    let slot = cuts.slots[node];
                    if slot == 0 {
                        continue;
                    }
                    let row = &cuts.rows[slot as usize - 1];
                    if row.boundary[side] > 0.0 {
                        let coordinate = mg::coords(node, dims)[a];
                        let mirror = if coordinate == 0 || coordinate + 1 == dims[a] {
                            2.0
                        } else {
                            1.0
                        };
                        let theta = mirror / (cuts.grid.spacing[a].powi(2) * row.boundary[side]);
                        let boundary = Node {
                            value: row.voltage[side],
                            ..Default::default()
                        };
                        matrix.energy(&nodes[node], &boundary, coefficient / theta);
                        cut = true;
                    }
                }
                if !cut {
                    matrix.energy(&nodes[i], &nodes[j], coefficient);
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use simrep_geom::Sdf;

    fn plates(n: usize, factor: usize) -> AdaptiveGrid {
        let electrodes = [
            Electrode::new(
                0.0,
                Sdf::Plane {
                    normal: [1.0, 0.0, 0.0],
                    offset: 0.0,
                },
            ),
            Electrode::new(
                1.0,
                Sdf::Plane {
                    normal: [-1.0, 0.0, 0.0],
                    offset: -1.0,
                },
            ),
        ];
        let base = CutCells::new(
            Grid {
                dims: [n, n, 1],
                spacing: [1.0 / (n - 1) as f64; 3],
                geometry: Geometry::Planar,
            },
            [0.0; 3],
            &electrodes,
        )
        .unwrap();
        AdaptiveGrid::new(
            base,
            &[Region {
                bounds_mm: [[0.25, 0.25, 0.0], [0.75, 0.75, 0.0]],
                factor,
            }],
            &electrodes,
        )
        .unwrap()
    }

    #[test]
    fn composite_is_symmetric_positive_definite_and_balances_weak_interface_flux() {
        for factor in [2, 4] {
            let grid = plates(9, factor);
            let solution = grid
                .solve(Options {
                    relative_tolerance: 1e-12,
                    max_cycles: 300,
                    ..Default::default()
                })
                .unwrap();
            assert!(solution.report.converged);
            let matrix = &solution.matrix;
            let n = matrix.rhs.len();
            let mut dense = vec![vec![0.0_f64; n]; n];
            for (i, row) in dense.iter_mut().enumerate() {
                for &(j, value) in &matrix.entries[matrix.offsets[i]..matrix.offsets[i + 1]] {
                    row[j] = value;
                }
            }
            let scale = matrix.diagonal.iter().copied().fold(0.0_f64, f64::max);
            let mut symmetry = 0.0_f64;
            for (i, row) in dense.iter().enumerate() {
                for (j, &value) in row.iter().enumerate() {
                    symmetry = symmetry.max((value - dense[j][i]).abs());
                }
            }
            assert!(symmetry < 2e-14 * scale, "symmetry={symmetry}");
            // Full dense Cholesky, not just positive energy of selected vectors.
            let mut smallest = f64::INFINITY;
            for i in 0..n {
                for j in 0..=i {
                    let v = dense[i][j] - (0..j).map(|k| dense[i][k] * dense[j][k]).sum::<f64>();
                    if i == j {
                        assert!(v > 0.0, "Cholesky pivot {i}: {v}");
                        smallest = smallest.min(v);
                        dense[i][j] = v.sqrt();
                    } else {
                        dense[i][j] = v / dense[j][j];
                    }
                }
            }
            // u=x is harmonic, has unit normal flux through both patch faces,
            // and zero transverse flux. P^T sums both sides into each trace row.
            let exact: Vec<_> = solution.locations.iter().map(|p| p[0]).collect();
            let mut applied = vec![0.0; n];
            matrix.apply(&exact, &mut applied);
            let flux = applied
                .iter()
                .zip(&matrix.rhs)
                .fold(0.0_f64, |m, (a, b)| m.max((a - b).abs()));
            assert!(flux < 2e-13 * scale, "weak interface imbalance={flux}");
            for (i, &v) in solution.base.iter().enumerate() {
                assert!((v - point(&grid.base, i)[0]).abs() < 2e-10);
            }
            eprintln!(
                "factor={factor} unknowns={n} symmetry={symmetry:.3e} min_cholesky_pivot={smallest:.3e} weak_flux={flux:.3e}"
            );
        }
    }

    #[test]
    fn manufactured_poisson_is_second_order_across_patch_interfaces() {
        let mut errors = Vec::new();
        for n in [17, 33, 65] {
            let mut grid = plates(n, 2).with_voltages(|_| 0.0).unwrap();
            for cuts in
                std::iter::once(&mut grid.base).chain(grid.patches.iter_mut().map(|p| &mut p.cuts))
            {
                for i in 0..cuts.values.len() {
                    let p = point(cuts, i);
                    if p[0] == 0.0 || p[0] == 1.0 || p[1] == 0.0 || p[1] == 1.0 {
                        cuts.fixed[i] = true;
                        cuts.values[i] = 0.0;
                    }
                }
            }
            let pi = std::f64::consts::PI;
            let exact = |p: [f64; 3]| (pi * p[0]).sin() * (pi * p[1]).sin();
            let solution = grid
                .solve_poisson(
                    Options {
                        relative_tolerance: 1e-12,
                        max_cycles: 300,
                        ..Default::default()
                    },
                    |p| 2.0 * pi * pi * exact(p),
                )
                .unwrap();
            assert!(
                solution.report.converged,
                "{:?}",
                solution.report.residual_history
            );
            let mut error = 0.0_f64;
            for (cuts, values) in std::iter::once((&grid.base, &solution.base)).chain(
                grid.patches
                    .iter()
                    .zip(&solution.patches)
                    .map(|(p, v)| (&p.cuts, v)),
            ) {
                for (i, &v) in values.iter().enumerate() {
                    error = error.max((v - exact(point(cuts, i))).abs());
                }
            }
            eprintln!("MMS n={n}: max_error={error:.9e}");
            errors.push(error);
        }
        for pair in errors.windows(2) {
            let order = (pair[0] / pair[1]).log2();
            assert!(order > 1.7, "interface MMS order={order}: {errors:?}");
        }
    }

    #[test]
    fn curved_cuts_crossing_the_interface_keep_symmetry_and_mms_order() {
        let radius = 0.28_f64;
        let mut errors = Vec::new();
        for n in [17, 33, 65] {
            let mut worst_phase = 0.0_f64;
            // As in the uniform cut-cell convergence tests, changing snap
            // alignment must not let one unusually favorable grid set the rate.
            for phase in [0.0, 0.25, 0.5, 0.75] {
                let h = 1.0 / (n - 1) as f64;
                let center = [0.5 + phase * h, 0.5 + 0.37 * phase * h, 0.0];
                let electrodes = [Electrode::new(
                    radius * radius,
                    Sdf::Sphere { center, radius },
                )];
                let base = CutCells::new(
                    Grid {
                        dims: [n, n, 1],
                        spacing: [1.0 / (n - 1) as f64; 3],
                        geometry: Geometry::Planar,
                    },
                    [0.0; 3],
                    &electrodes,
                )
                .unwrap();
                let mut grid = AdaptiveGrid::new(
                    base,
                    &[Region {
                        bounds_mm: [[0.25, 0.25, 0.0], [0.75, 0.75, 0.0]],
                        factor: 2,
                    }],
                    &electrodes,
                )
                .unwrap();
                let exact = |p: [f64; 3]| (p[0] - center[0]).powi(2) + (p[1] - center[1]).powi(2);
                for cuts in std::iter::once(&mut grid.base)
                    .chain(grid.patches.iter_mut().map(|p| &mut p.cuts))
                {
                    for i in 0..cuts.values.len() {
                        let p = point(cuts, i);
                        if p[0] == 0.0 || p[0] == 1.0 || p[1] == 0.0 || p[1] == 1.0 {
                            cuts.fixed[i] = true;
                            cuts.values[i] = exact(p);
                        }
                    }
                }
                let solution = grid
                    .solve_poisson(
                        Options {
                            relative_tolerance: 1e-12,
                            max_cycles: 300,
                            ..Default::default()
                        },
                        |_| -4.0,
                    )
                    .unwrap();
                assert!(solution.report.converged);
                let mut asymmetry = 0.0_f64;
                for i in 0..solution.unknowns {
                    for &(j, value) in &solution.matrix.entries
                        [solution.matrix.offsets[i]..solution.matrix.offsets[i + 1]]
                    {
                        let other = solution.matrix.entries
                            [solution.matrix.offsets[j]..solution.matrix.offsets[j + 1]]
                            .iter()
                            .find(|&&(k, _)| k == i)
                            .map_or(0.0, |&(_, v)| v);
                        asymmetry = asymmetry.max((value - other).abs());
                    }
                }
                assert!(asymmetry < 1e-13);
                let mut worst = 0.0_f64;
                for (cuts, values) in std::iter::once((&grid.base, &solution.base)).chain(
                    grid.patches
                        .iter()
                        .zip(&solution.patches)
                        .map(|(p, v)| (&p.cuts, v)),
                ) {
                    for (i, &v) in values.iter().enumerate() {
                        let p = point(cuts, i);
                        if exact(p) > radius * radius {
                            worst = worst.max((v - exact(p)).abs());
                        }
                    }
                }
                eprintln!(
                    "cut-interface MMS n={n} phase={phase} max_error={worst:.9e} symmetry={asymmetry:.3e}"
                );
                worst_phase = worst_phase.max(worst);
            }
            let h = 1.0 / (n - 1) as f64;
            assert!(
                worst_phase < 2.0 * h * h,
                "cut-interface O(h^2) envelope: {worst_phase}"
            );
            errors.push(worst_phase);
        }
        // Like the uniform curved-boundary ladder, measure over two
        // refinements: individual rates vary as nodes enter the O(h^2)
        // snapping band. The absolute quadratic envelope is checked above.
        let order = (errors[0] / errors[2]).ln() / 4.0_f64.ln();
        assert!(order > 1.5, "cut-interface MMS order={order}: {errors:?}");
    }
}
