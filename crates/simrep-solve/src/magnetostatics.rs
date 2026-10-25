//! Linear magnetostatics in mm, A, and gauss, sharing dielectric coefficients.

use std::{f64::consts::PI, path::PathBuf};

use serde::{Deserialize, Serialize};

use crate::{
    dielectric::Permittivity,
    gpu::{Gpu, GpuResult, Poisson, SolveReport},
    mg::{self, Axis, Geometry, Grid, Options, Report, SolveError},
};

mod input;
pub use input::{load, load_cells};

/// Meaning of the values in a magnetic potential PA.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Formulation {
    Scalar,
    PlanarAz,
    AxisymmetricPsi,
    CartesianA,
}

/// Version 1 magnetic field sidecar. `potential` is relative to the sidecar.
/// Cells use PA storage order, not the solver's transposed cylindrical order.
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FieldFile {
    pub version: u32,
    pub formulation: Formulation,
    pub potential: PathBuf,
    pub ng: f64,
    pub solid_cells: Vec<bool>,
    /// Required only for Cartesian A; paths for Ax, Ay, Az.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub components: Option<[PathBuf; 3]>,
}

/// Prepared operator, physical source, and ion-opaque material cells.
/// Scalar p solves -div(mu_r grad p) = (4*pi/ng)*(-div M).
/// Planar Az solves -div(mu_r^-1 grad Az) = (4*pi/ng)*Jz.
/// Axisymmetric psi=r*Aphi uses the PLANAR operator with coefficient
/// 1/(mu_r*r), with psi=0 on the axis.
pub struct MagneticProblem {
    pub formulation: Formulation,
    pub solid_cells: Vec<bool>,
    material: Permittivity,
    source: Vec<f64>,
    axis: Vec<usize>,
}

/// RTX 3070 / Ryzen 5950X compute crossover for prepared magnetic Poisson.
/// Native f64 wins at 101*101*51 nodes; the CPU-residual backend first wins
/// reliably at 101^3. Two-dimensional controls remain on the CPU.
pub fn prefer_gpu(grid: Grid, native_f64: bool) -> bool {
    grid.dims[2] > 1
        && grid
            .dims
            .iter()
            .try_fold(1usize, |n, d| n.checked_mul(*d))
            .is_some_and(|nodes| nodes >= 101 * 101 * if native_f64 { 51 } else { 101 })
}

fn validate_cells(grid: Grid, cells: usize, ng: f64) -> Result<(), SolveError> {
    let dims = grid.dims.map(|n| n.saturating_sub(1).max(1));
    if dims.iter().try_fold(1usize, |n, d| n.checked_mul(*d)) != Some(cells)
        || !ng.is_finite()
        || ng <= 0.0
    {
        return Err(mg::invalid(
            "magnetic cell dimensions must match the grid and ng must be positive",
        ));
    }
    Ok(())
}

fn adjacent(
    grid: Grid,
    node: usize,
    link_axis: Option<usize>,
    mut visit: impl FnMut(usize, [usize; 3]),
) {
    let c = mg::coords(node, grid.dims);
    let dims = grid.dims.map(|n| n.saturating_sub(1).max(1));
    for corner in 0..8 {
        if (0..3).any(|a| (grid.dims[a] == 1 || link_axis == Some(a)) && corner & (1 << a) != 0) {
            continue;
        }
        let mut cell = c;
        let mut inside = true;
        for a in 0..3 {
            if grid.dims[a] == 1 || link_axis == Some(a) {
                continue;
            }
            if corner & (1 << a) == 0 {
                if cell[a] == 0 {
                    inside = false;
                    break;
                }
                cell[a] -= 1;
            } else if cell[a] == dims[a] {
                inside = false;
                break;
            }
        }
        if inside {
            visit(cell[0] + dims[0] * (cell[1] + dims[1] * cell[2]), cell);
        }
    }
}

fn cell_volume_weight(grid: Grid, node: usize, cell: [usize; 3]) -> f64 {
    match grid.geometry {
        Geometry::Planar => 1.0,
        Geometry::Cylindrical { radial_origin } => {
            let radial_node = node % grid.dims[0];
            let r = radial_origin + radial_node as f64 * grid.spacing[0];
            // Only half of a primal cell lies inside the nodal dual volume.
            // Its radial centroid is r +/- h/4, not the primal center r +/- h/2.
            r + if cell[0] < radial_node { -0.25 } else { 0.25 } * grid.spacing[0]
        }
    }
}

/// Volume average primal-cell sources onto potential nodes. Cylindrical
/// scalar sources use radial volume weights; psi sources use a planar grid.
pub fn cell_source(grid: Grid, values: &[f64]) -> Result<Vec<f64>, SolveError> {
    validate_cells(grid, values.len(), 1.0)?;
    if values.iter().any(|v| !v.is_finite()) {
        return Err(mg::invalid("magnetic source must be finite"));
    }
    Ok((0..grid.dims.iter().product())
        .map(|node| {
            let mut sum = 0.0;
            let mut weight = 0.0;
            adjacent(grid, node, None, |i, cell| {
                let w = cell_volume_weight(grid, node, cell);
                sum += w * values[i];
                weight += w;
            });
            sum / weight
        })
        .collect())
}

impl MagneticProblem {
    /// One component of Cartesian A in a spatially uniform permeability.
    /// Variable-mu 3D vector potentials are coupled and are not represented
    /// by three independent scalar Poisson equations.
    pub fn cartesian_current_component(
        grid: Grid,
        fixed: &[bool],
        permeability: f64,
        current_density: &[f64],
        ng: f64,
    ) -> Result<Self, SolveError> {
        if grid.geometry != Geometry::Planar || !permeability.is_finite() || permeability <= 0.0 {
            return Err(mg::invalid(
                "Cartesian A requires planar geometry and positive uniform permeability",
            ));
        }
        validate_cells(grid, current_density.len(), ng)?;
        let material = Permittivity::from_cells(
            grid,
            fixed,
            &vec![1.0 / permeability; current_density.len()],
        )?;
        let mut source = cell_source(grid, current_density)?;
        source.iter_mut().for_each(|v| *v *= 4.0 * PI / ng);
        Ok(Self {
            formulation: Formulation::CartesianA,
            solid_cells: vec![permeability != 1.0; current_density.len()],
            material,
            source,
            axis: Vec::new(),
        })
    }

    /// Cell magnetization is (Mx,My,Mz), or (Mr,Mz,0) for solver (r,z).
    /// An explicit cell -div M may replace its discrete divergence, for
    /// reproducing source arrays supplied by SIMION examples.
    pub fn scalar(
        grid: Grid,
        fixed: &[bool],
        permeability: &[f64],
        magnetization: &[[f64; 3]],
        negative_divergence: Option<&[f64]>,
        ng: f64,
    ) -> Result<Self, SolveError> {
        validate_cells(grid, permeability.len(), ng)?;
        if magnetization.len() != permeability.len()
            || magnetization.iter().flatten().any(|v| !v.is_finite())
        {
            return Err(mg::invalid(
                "cell magnetization must be finite and match permeability",
            ));
        }
        let material = Permittivity::from_cells(grid, fixed, permeability)?;
        let mut source = if let Some(values) = negative_divergence {
            cell_source(grid, values)?
        } else {
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
            (0..fixed.len())
                .map(|node| {
                    let c = mg::coords(node, grid.dims);
                    let mut divergence = 0.0;
                    for a in 0..3 {
                        for side in 0..2 {
                            if grid.dims[a] == 1
                                || side == 0 && c[a] == 0
                                || side == 1 && c[a] + 1 == grid.dims[a]
                            {
                                continue;
                            }
                            let first = if side == 0 { node - strides[a] } else { node };
                            let mut sum = 0.0;
                            let mut weight = 0.0;
                            adjacent(grid, first, Some(a), |i, cell| {
                                let w = if a == 0 {
                                    1.0
                                } else {
                                    cell_volume_weight(grid, first, cell)
                                };
                                sum += w * magnetization[i][a];
                                weight += w;
                            });
                            let coefficient = if side == 0 {
                                axes[a].minus[c[a]]
                            } else {
                                axes[a].plus[c[a]]
                            };
                            divergence += if side == 0 { 1.0 } else { -1.0 }
                                * coefficient
                                * grid.spacing[a]
                                * sum
                                / weight;
                        }
                    }
                    divergence
                })
                .collect()
        };
        source.iter_mut().for_each(|v| *v *= 4.0 * PI / ng);
        Ok(Self {
            formulation: Formulation::Scalar,
            solid_cells: permeability
                .iter()
                .zip(magnetization)
                .map(|(&mu, m)| mu != 1.0 || *m != [0.0; 3])
                .collect(),
            material,
            source,
            axis: Vec::new(),
        })
    }

    /// Jz in planar 2D, or Jphi in cylindrical (r,z). The current density
    /// alone does not make a cell solid; callers may add conductor occupancy.
    pub fn currents(
        mut grid: Grid,
        fixed: &[bool],
        permeability: &[f64],
        current_density: &[f64],
        ng: f64,
    ) -> Result<Self, SolveError> {
        validate_cells(grid, permeability.len(), ng)?;
        if grid.dims[0] < 2
            || grid.dims[1] < 2
            || grid.dims[2] != 1
            || current_density.len() != permeability.len()
        {
            return Err(mg::invalid(
                "current potentials require planar 2D or axisymmetric cell arrays",
            ));
        }
        let mut mask = fixed.to_vec();
        let mut axis = Vec::new();
        let mut coefficient: Vec<_> = permeability.iter().map(|v| 1.0 / v).collect();
        let formulation = match grid.geometry {
            Geometry::Planar => Formulation::PlanarAz,
            Geometry::Cylindrical { radial_origin } => {
                let nr = grid.dims[0].saturating_sub(1).max(1);
                for (i, value) in coefficient.iter_mut().enumerate() {
                    *value /=
                        radial_origin + (i % nr) as f64 * grid.spacing[0] + 0.5 * grid.spacing[0];
                }
                if radial_origin == 0.0 {
                    for node in (0..mask.len()).step_by(grid.dims[0]) {
                        mask[node] = true;
                        axis.push(node);
                    }
                }
                grid.geometry = Geometry::Planar;
                Formulation::AxisymmetricPsi
            }
        };
        // Validate mu itself as well as its reciprocal: negative or infinite
        // permeability must not turn into an admissible coefficient.
        if permeability.iter().any(|v| !v.is_finite() || *v <= 0.0) {
            return Err(mg::invalid(
                "relative permeability must be finite and positive",
            ));
        }
        let material = Permittivity::from_cells(grid, &mask, &coefficient)?;
        let mut source = cell_source(grid, current_density)?;
        source.iter_mut().for_each(|v| *v *= 4.0 * PI / ng);
        Ok(Self {
            formulation,
            solid_cells: permeability.iter().map(|&mu| mu != 1.0).collect(),
            material,
            source,
            axis,
        })
    }

    pub fn prepared_stencil(&self) -> &crate::cutcell::CutCells {
        self.material.prepared_stencil()
    }

    pub fn source(&self) -> &[f64] {
        &self.source
    }

    fn regular_axis(&self, potential: &[f64]) -> Result<(), SolveError> {
        if self.axis.iter().any(|&i| potential.get(i) != Some(&0.0)) {
            return Err(mg::invalid(
                "axisymmetric psi must be zero on the regular axis",
            ));
        }
        Ok(())
    }

    pub fn solve(&self, potential: &mut [f64], options: Options) -> Result<Report, SolveError> {
        self.regular_axis(potential)?;
        let started = std::time::Instant::now();
        let stencil = self.prepared_stencil();
        // Two constant Dirichlet planes in vacuum have an affine solution.
        // Starting from zero otherwise spends many V-cycles removing smooth
        // error, even though the boundary data already determine that field.
        // Leave residual-only queries and nonuniform operators untouched.
        if options.max_cycles > 0
            && stencil.grid.geometry == Geometry::Planar
            && stencil.rows.is_empty()
            && self.source.iter().all(|&v| v == 0.0)
        {
            mg::validate(potential, &stencil.fixed, stencil.grid, options)?;
            let dims = stencil.grid.dims;
            let strides = [1, dims[0], dims[0] * dims[1]];
            for a in 0..3 {
                if dims[a] == 1 {
                    continue;
                }
                let lower = potential[0];
                let upper = potential[(dims[a] - 1) * strides[a]];
                if stencil.fixed.iter().enumerate().all(|(i, &fixed)| {
                    let c = i / strides[a] % dims[a];
                    let boundary = c == 0 || c + 1 == dims[a];
                    fixed == boundary
                        && (!fixed || potential[i] == if c == 0 { lower } else { upper })
                }) {
                    for (i, value) in potential.iter_mut().enumerate() {
                        if !stencil.fixed[i] {
                            *value = lower
                                + (upper - lower) * (i / strides[a] % dims[a]) as f64
                                    / (dims[a] - 1) as f64;
                        }
                    }
                    break;
                }
            }
        }
        let initialization = started.elapsed();
        // The ordinary solver still certifies the requested physical residual.
        let mut report = self
            .material
            .solve_poisson(potential, &self.source, options)?;
        report.timings.solve += initialization;
        report.elapsed += initialization;
        Ok(report)
    }

    pub fn solve_gpu(
        &self,
        gpu: &Gpu,
        potential: &mut [f64],
        options: Options,
    ) -> GpuResult<SolveReport> {
        self.regular_axis(potential)?;
        let started = std::time::Instant::now();
        let mut solver = Poisson::new_prepared(
            gpu,
            potential,
            self.prepared_stencil(),
            options,
            gpu.has_f64(),
        )?;
        let setup = started.elapsed();
        let mut report = solver.solve(gpu, potential, &self.source)?;
        report.timings.setup += setup;
        report.elapsed = started.elapsed();
        Ok(report)
    }
}
