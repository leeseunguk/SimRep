//! Scalar, positive relative permittivity for `-div(epsilon_r grad V) = source`.
//!
//! Nodal coefficients use harmonic means on links. SIMION dielectric arrays
//! instead describe primal cells: a link lies inside a cell longitudinally, so
//! its coefficient is the area average of the adjacent transverse cells.
//! In both cases a link has one shared coefficient. Multiplying by the existing
//! cylindrical/mirror quadrature weights therefore gives an SPD operator.

use rayon::prelude::*;
use simrep_geom::{Point, SignedDistance};

use crate::{
    cutcell::{CutCells, CutRow},
    gpu::{Gpu, GpuResult, SolveReport},
    mg::{self, Axis, Geometry, Grid, Options, Report, SolveError},
};

mod input;
pub use input::{LoadedMaterial, MaterialReport, load, load_material};
mod nonlinear;
pub use nonlinear::{
    FieldLaw, FieldSampler, NonlinearOptions, NonlinearPermittivity, NonlinearReport,
};

/// Measured Ryzen 5950X / RTX 3070 crossover for automatic dielectric refine.
/// Linear and nonlinear 3D solves have different measured crossover brackets.
/// Both devices use 16 Rayon workers; explicit device choices bypass this policy.
pub fn prefer_gpu(grid: Grid, nonlinear: bool) -> bool {
    grid.dims[2] > 1
        && grid
            .dims
            .iter()
            .try_fold(1usize, |n, d| n.checked_mul(*d))
            .is_some_and(|nodes| nodes >= if nonlinear { 131_072 } else { 1_048_576 })
}

/// A material region. Later regions override earlier ones in overlaps.
pub struct Region<'a> {
    pub relative_permittivity: f64,
    pub shape: &'a dyn SignedDistance,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn weight(c: [usize; 3], grid: Grid) -> f64 {
        (0..3)
            .map(|a| {
                let n = grid.dims[a];
                if n == 1 {
                    return 1.0;
                }
                if let Geometry::Cylindrical { radial_origin } = grid.geometry
                    && a == 0
                {
                    let r = radial_origin / grid.spacing[0] + c[0] as f64;
                    if c[0] == 0 && radial_origin == 0.0 {
                        return 0.125;
                    }
                    if c[0] == 0 {
                        return 0.5 * (r + 0.5);
                    }
                    if c[0] + 1 == n {
                        return 0.5 * (r - 0.5);
                    }
                    return r;
                }
                if c[a] == 0 || c[a] + 1 == n { 0.5 } else { 1.0 }
            })
            .product()
    }

    #[test]
    fn weighted_links_are_symmetric_and_positive_including_axis_and_mirrors() {
        for geometry in [
            Geometry::Planar,
            Geometry::Cylindrical { radial_origin: 0.0 },
            Geometry::Cylindrical { radial_origin: 2.0 },
        ] {
            let grid = Grid {
                dims: [9, 7, 1],
                spacing: [0.3, 0.7, 1.0],
                geometry,
            };
            let mut fixed = vec![false; 63];
            fixed[0] = true;
            let eps: Vec<_> = (0..63).map(|i| 2.0 + (i % 5) as f64).collect();
            let material = Permittivity::from_nodes(grid, &fixed, &eps).unwrap();
            for row in &material.stencil.rows {
                let c = mg::coords(row.node, grid.dims);
                for (s, &a) in row.neighbors.iter().enumerate() {
                    if a == 0.0 {
                        continue;
                    }
                    assert!(a > 0.0);
                    let stride = [1, 9, 63][s / 2];
                    let j = row
                        .node
                        .wrapping_add_signed(stride * if s % 2 == 0 { -1 } else { 1 });
                    if fixed[j] {
                        continue;
                    }
                    let reverse = &material.stencil.rows[material.stencil.slots[j] as usize - 1];
                    let wa = weight(c, grid) * a;
                    let wb = weight(mg::coords(j, grid.dims), grid) * reverse.neighbors[s ^ 1];
                    assert!((wa - wb).abs() < 2e-14 * wa, "{geometry:?}: {wa} != {wb}");
                }
            }
        }
    }
}

/// Prepared, reusable coefficients and Dirichlet mask. Voltages are supplied
/// at solve time, so this also supports linear fast-adjust basis solutions.
#[derive(Clone)]
pub struct Permittivity {
    stencil: CutCells,
}

fn positive(values: &[f64]) -> Result<(), SolveError> {
    if values.iter().any(|v| !v.is_finite() || *v <= 0.0) {
        return Err(mg::invalid(
            "relative permittivity must be finite and positive",
        ));
    }
    Ok(())
}

fn harmonic(a: f64, b: f64) -> f64 {
    let lo = a.min(b);
    lo / (0.5 + 0.5 * lo / a.max(b))
}

impl Permittivity {
    /// Borrow the prepared variable-coefficient operator and fixed-node mask.
    /// This is also the operator used by scalar magnetostatics and GPU Poisson.
    pub fn prepared_stencil(&self) -> &CutCells {
        &self.stencil
    }

    /// Relative permittivity at potential nodes (x-fastest). Interfaces lie
    /// halfway between unlike nodes; use cells for grid-aligned SIMION inputs.
    pub fn from_nodes(grid: Grid, fixed: &[bool], relative: &[f64]) -> Result<Self, SolveError> {
        positive(relative)?;
        if relative.len() != fixed.len() {
            return Err(mg::invalid("nodal permittivity length must match the grid"));
        }
        Self::prepare(grid, fixed, |i, j, _| harmonic(relative[i], relative[j]))
    }

    /// SIMION dielectric-array convention: one fewer sample than potential
    /// nodes in each active dimension, sampled at primal-cell centers.
    /// Electrode flags from the material PA are not part of this API.
    pub fn from_cells(grid: Grid, fixed: &[bool], relative: &[f64]) -> Result<Self, SolveError> {
        positive(relative)?;
        let dims = grid.dims.map(|n| n.saturating_sub(1).max(1));
        if dims.iter().try_fold(1usize, |n, d| n.checked_mul(*d)) != Some(relative.len()) {
            return Err(mg::invalid(
                "cell permittivity dimensions must be max(node dimensions - 1, 1)",
            ));
        }
        Self::prepare(grid, fixed, |i, _, axis| {
            let c = mg::coords(i, grid.dims);
            let mut sum = 0.0;
            let mut count = 0usize;
            for corner in 0..8usize {
                if corner & (1 << axis) != 0
                    || (0..3).any(|a| grid.dims[a] == 1 && corner & (1 << a) != 0)
                {
                    continue;
                }
                let mut cell = c;
                let mut inside = true;
                for a in 0..3 {
                    if a == axis || grid.dims[a] == 1 {
                        continue;
                    }
                    if corner & (1 << a) == 0 {
                        if c[a] == 0 {
                            inside = false;
                            break;
                        }
                        cell[a] -= 1;
                    } else if c[a] == dims[a] {
                        inside = false;
                        break;
                    }
                }
                if inside {
                    sum += relative[cell[0] + dims[0] * (cell[1] + dims[1] * cell[2])];
                    count += 1;
                }
            }
            sum / count as f64
        })
    }

    /// Sample piecewise-constant SDF materials at nodes and use harmonic links.
    /// Coordinates are Cartesian, or (r,z,0) in cylindrical geometry.
    pub fn from_regions(
        grid: Grid,
        fixed: &[bool],
        origin: Point,
        background: f64,
        regions: &[Region<'_>],
    ) -> Result<Self, SolveError> {
        mg::validate(&vec![0.0; fixed.len()], fixed, grid, Options::default())?;
        positive(&[background])?;
        if !origin.iter().all(|v| v.is_finite()) {
            return Err(mg::invalid("material origin must be finite"));
        }
        if let Geometry::Cylindrical { radial_origin } = grid.geometry
            && origin[0] != radial_origin
        {
            return Err(mg::invalid("material radial origin must match the grid"));
        }
        for region in regions {
            positive(&[region.relative_permittivity])?;
        }
        let relative: Result<Vec<_>, _> = (0..fixed.len())
            .into_par_iter()
            .map(|i| {
                let c = mg::coords(i, grid.dims);
                let p = std::array::from_fn(|a| origin[a] + c[a] as f64 * grid.spacing[a]);
                let mut value = background;
                for region in regions {
                    let distance = region.shape.distance(p);
                    if !distance.is_finite() {
                        return Err(mg::invalid("material SDF returned a non-finite distance"));
                    }
                    if distance <= 0.0 {
                        value = region.relative_permittivity;
                    }
                }
                Ok(value)
            })
            .collect();
        Self::from_nodes(grid, fixed, &relative?)
    }

    fn prepare(
        grid: Grid,
        fixed: &[bool],
        link: impl Fn(usize, usize, usize) -> f64 + Sync,
    ) -> Result<Self, SolveError> {
        mg::validate(&vec![0.0; fixed.len()], fixed, grid, Options::default())?;
        if fixed.len() > u32::MAX as usize {
            return Err(mg::invalid("material grid exceeds the row index range"));
        }
        let axes: [Axis; 3] = std::array::from_fn(|a| {
            Axis::new(
                grid.dims[a],
                grid.spacing[a],
                match grid.geometry {
                    Geometry::Cylindrical { radial_origin } if a == 0 => Some(radial_origin),
                    _ => None,
                },
            )
        });
        let strides = [1, grid.dims[0], grid.dims[0] * grid.dims[1]];
        let rows: Result<Vec<_>, _> = (0..fixed.len())
            .into_par_iter()
            .filter(|&i| !fixed[i])
            .map(|i| {
                let c = mg::coords(i, grid.dims);
                let mut row = CutRow {
                    node: i,
                    neighbors: [0.0; 6],
                    boundary: [0.0; 6],
                    voltage: [0.0; 6],
                };
                let mut changed = false;
                for a in 0..3 {
                    for side in 0..2 {
                        if (side == 0 && c[a] == 0) || (side == 1 && c[a] + 1 == grid.dims[a]) {
                            continue;
                        }
                        let j = i.wrapping_add_signed(
                            strides[a] as isize * if side == 0 { -1 } else { 1 },
                        );
                        // Always evaluate the positive link, so opposite rows
                        // share exactly the same material coefficient.
                        let epsilon = link(i.min(j), i.max(j), a);
                        let base = if side == 0 {
                            axes[a].minus[c[a]]
                        } else {
                            axes[a].plus[c[a]]
                        };
                        row.neighbors[2 * a + side] = epsilon * base;
                        changed |= epsilon != 1.0;
                    }
                }
                if !row.diagonal().is_finite() || row.diagonal() <= 0.0 {
                    return Err(mg::invalid(
                        "material coefficients are not numerically representable",
                    ));
                }
                Ok(changed.then_some(row))
            })
            .collect();
        let rows: Vec<_> = rows?.into_iter().flatten().collect();
        let mut slots = vec![0; fixed.len()];
        for (slot, row) in rows.iter().enumerate() {
            slots[row.node] = slot as u32 + 1;
        }
        Ok(Self {
            stencil: CutCells {
                grid,
                origin: [0.0; 3],
                fixed: fixed.to_vec(),
                values: Vec::new(),
                snap_distance: 0.0,
                slots,
                rows,
            },
        })
    }

    pub fn solve(&self, potential: &mut [f64], options: Options) -> Result<Report, SolveError> {
        mg::solve_prepared(
            potential,
            &self.stencil.fixed,
            self.stencil.grid,
            options,
            Some(&self.stencil),
            None,
        )
    }

    /// `source = rho_free / epsilon_0`, not divided by epsilon_r. Source values
    /// on fixed nodes are ignored. Units match mg::solve_poisson.
    pub fn solve_poisson(
        &self,
        potential: &mut [f64],
        source: &[f64],
        options: Options,
    ) -> Result<Report, SolveError> {
        if source.len() != potential.len() || source.iter().any(|v| !v.is_finite()) {
            return Err(mg::invalid(
                "Poisson source must be finite and match the potential length",
            ));
        }
        mg::solve_prepared(
            potential,
            &self.stencil.fixed,
            self.stencil.grid,
            options,
            Some(&self.stencil),
            Some(source),
        )
    }

    pub fn solve_gpu(
        &self,
        gpu: &Gpu,
        potential: &mut [f64],
        options: Options,
    ) -> GpuResult<SolveReport> {
        self.solve_gpu_with_precision(gpu, potential, options, gpu.has_f64())
    }

    pub fn solve_gpu_with_precision(
        &self,
        gpu: &Gpu,
        potential: &mut [f64],
        options: Options,
        native_f64: bool,
    ) -> GpuResult<SolveReport> {
        gpu.solve_inner(
            potential,
            &self.stencil.fixed,
            self.stencil.grid,
            options,
            native_f64,
            Some(&self.stencil),
        )
    }
}
