//! Self-consistent cell-centred isotropic constitutive laws.
use std::time::{Duration, Instant};

use rayon::prelude::*;
use serde::Deserialize;

use super::{Permittivity, positive};
use crate::{
    gpu::{Gpu, GpuResult},
    mg::{self, Grid, Options, Residual, SolveError},
};

/// Fields use V / grid length unit (V/mm for PA inputs), not V/m.
#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum FieldLaw {
    /// `max(epsilon0 * (1 - lambda * |E|^2), minimum)`.
    SaturatingQuadratic { lambda: f64, minimum: f64 },
    /// Piecewise-linear `[field, epsilon]` knots; constant beyond endpoints.
    Table { points: Vec<[f64; 2]> },
}

impl FieldLaw {
    fn validate(&self) -> Result<(), SolveError> {
        match self {
            Self::SaturatingQuadratic { lambda, minimum } => {
                positive(&[*minimum])?;
                if !lambda.is_finite() || *lambda < 0.0 {
                    return Err(mg::invalid(
                        "quadratic lambda must be finite and nonnegative",
                    ));
                }
            }
            Self::Table { points } => {
                if points.len() < 2
                    || points.iter().any(|p| !p[0].is_finite() || p[0] < 0.0)
                    || points.windows(2).any(|p| p[0][0] >= p[1][0])
                {
                    return Err(mg::invalid(
                        "law table needs two or more increasing nonnegative fields",
                    ));
                }
                positive(&points.iter().map(|p| p[1]).collect::<Vec<_>>())?;
            }
        }
        Ok(())
    }

    pub fn epsilon(&self, epsilon0: f64, field: f64) -> f64 {
        match self {
            Self::SaturatingQuadratic { lambda, minimum } => {
                (epsilon0 * (1.0 - lambda * field * field)).max(*minimum)
            }
            Self::Table { points } => {
                let j = points.partition_point(|p| p[0] < field);
                if j == 0 {
                    points[0][1]
                } else if j == points.len() {
                    points[j - 1][1]
                } else {
                    let [x, y] = points[j - 1];
                    y + (field - x) / (points[j][0] - x) * (points[j][1] - y)
                }
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum FieldSampler {
    #[default]
    CellCenter,
    /// SIMION example's one-sided projection, in physical grid coordinates.
    SphereOffset {
        center: [f64; 3],
        radius: f64,
        offset: f64,
    },
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct NonlinearOptions {
    pub max_iterations: usize,
    /// Relative infinity norm of the vector field change, checked separately.
    pub field_tolerance: f64,
    /// Initial potential relaxation, reduced on a growing nonlinear residual.
    pub relaxation: f64,
}

impl Default for NonlinearOptions {
    fn default() -> Self {
        Self {
            max_iterations: 100,
            field_tolerance: 1e-8,
            relaxation: 1.0,
        }
    }
}

#[derive(Debug)]
pub struct NonlinearReport {
    pub converged: bool,
    pub iterations: usize,
    pub inner_cycles: usize,
    /// Re-evaluated with epsilon(E), never the lagged linear coefficients.
    pub residual_history: Vec<Residual>,
    pub field_history: Vec<f64>,
    /// All computation, including coefficient/hierarchy rebuilds and transfers.
    pub compute: Duration,
}

/// Laws are selected by exact zero-field cell labels. Unlisted cells stay linear.
pub struct NonlinearPermittivity {
    grid: Grid,
    fixed: Vec<bool>,
    epsilon0: Vec<f64>,
    laws: Vec<(f64, FieldLaw)>,
    samples: Vec<[f64; 3]>,
}

impl NonlinearPermittivity {
    pub fn from_cells(
        grid: Grid,
        fixed: &[bool],
        epsilon0: &[f64],
        laws: Vec<(f64, FieldLaw)>,
        sampler: FieldSampler,
    ) -> Result<Self, SolveError> {
        // Use the same shape, positivity, gauge, and geometry contract as linear materials.
        Permittivity::from_cells(grid, fixed, epsilon0)?;
        for (i, (label, law)) in laws.iter().enumerate() {
            positive(&[*label])?;
            law.validate()?;
            if laws[..i].iter().any(|(other, _)| other == label) {
                return Err(mg::invalid("duplicate nonlinear material label"));
            }
        }
        if let FieldSampler::SphereOffset {
            center,
            radius,
            offset,
        } = sampler
            && (!center.iter().all(|v| v.is_finite())
                || !radius.is_finite()
                || !offset.is_finite()
                || offset <= 0.0
                || radius <= offset)
        {
            return Err(mg::invalid(
                "sphere sampler needs finite center and radius > offset > 0",
            ));
        }
        let dims = grid.dims.map(|n| (n - 1).max(1));
        let samples: Result<Vec<_>, _> = (0..epsilon0.len())
            .map(|i| {
                let c = mg::coords(i, dims);
                let mut p = std::array::from_fn(|a| {
                    if grid.dims[a] == 1 {
                        0.0
                    } else {
                        (c[a] as f64 + 0.5) * grid.spacing[a]
                    }
                });
                if let FieldSampler::SphereOffset {
                    center,
                    radius,
                    offset,
                } = sampler
                {
                    let r = (0..3)
                        .map(|a| (p[a] - center[a]).powi(2))
                        .sum::<f64>()
                        .sqrt();
                    let target = if r >= radius && r < radius + offset {
                        radius + offset
                    } else if r < radius && r > radius - offset {
                        radius - offset
                    } else {
                        r
                    };
                    if target != r {
                        p = std::array::from_fn(|a| center[a] + (p[a] - center[a]) * target / r);
                    }
                }
                if (0..3).any(|a| p[a] < 0.0 || p[a] > (grid.dims[a] - 1) as f64 * grid.spacing[a])
                {
                    return Err(mg::invalid("nonlinear field sample outside grid"));
                }
                Ok(p)
            })
            .collect();
        Ok(Self {
            grid,
            fixed: fixed.to_vec(),
            epsilon0: epsilon0.to_vec(),
            laws,
            samples: samples?,
        })
    }

    fn fields(&self, potential: &[f64]) -> Vec<[f64; 3]> {
        self.samples
            .par_iter()
            .map(|&p| self.field(potential, p))
            .collect()
    }

    /// Negative derivative of the multilinear potential, one-sided within a cell.
    fn field(&self, potential: &[f64], p: [f64; 3]) -> [f64; 3] {
        let lo: [usize; 3] = std::array::from_fn(|a| {
            if self.grid.dims[a] == 1 {
                0
            } else {
                ((p[a] / self.grid.spacing[a]).floor() as usize).min(self.grid.dims[a] - 2)
            }
        });
        let t: [f64; 3] = std::array::from_fn(|a| p[a] / self.grid.spacing[a] - lo[a] as f64);
        std::array::from_fn(|axis| {
            if self.grid.dims[axis] == 1 {
                return 0.0;
            }
            let mut value = 0.0;
            for corner in 0..8 {
                if (0..3).any(|a| self.grid.dims[a] == 1 && corner & (1 << a) != 0) {
                    continue;
                }
                let mut c = lo;
                let mut weight = 1.0;
                for a in 0..3 {
                    let high = corner & (1 << a) != 0;
                    c[a] += usize::from(high);
                    weight *= if a == axis {
                        if high { -1.0 } else { 1.0 }
                    } else if self.grid.dims[a] == 1 {
                        1.0
                    } else if high {
                        t[a]
                    } else {
                        1.0 - t[a]
                    };
                }
                value += weight
                    * potential[c[0] + self.grid.dims[0] * (c[1] + self.grid.dims[1] * c[2])];
            }
            value / self.grid.spacing[axis]
        })
    }

    fn material(&self, fields: &[[f64; 3]]) -> Result<Permittivity, SolveError> {
        let eps: Vec<_> = fields
            .par_iter()
            .zip(&self.epsilon0)
            .map(|(e, &k0)| {
                self.laws
                    .iter()
                    .find(|(label, _)| *label == k0)
                    .map_or(k0, |(_, law)| {
                        law.epsilon(k0, e.iter().map(|v| v * v).sum::<f64>().sqrt())
                    })
            })
            .collect();
        Permittivity::from_cells(self.grid, &self.fixed, &eps)
    }

    pub fn solve(
        &self,
        potential: &mut [f64],
        linear: Options,
        outer: NonlinearOptions,
        gpu: Option<&Gpu>,
    ) -> GpuResult<NonlinearReport> {
        mg::validate(potential, &self.fixed, self.grid, linear)?;
        if outer.max_iterations == 0
            || !outer.field_tolerance.is_finite()
            || outer.field_tolerance <= 0.0
            || !outer.relaxation.is_finite()
            || outer.relaxation <= 0.0
            || outer.relaxation > 1.0
        {
            return Err(mg::invalid(
                "nonlinear iterations/field tolerance must be positive; relaxation in (0,1]",
            )
            .into());
        }
        let start = Instant::now();
        let mut fields = self.fields(potential);
        let mut material = self.material(&fields)?;
        let residual = |m: &Permittivity, p: &[f64]| -> Result<Residual, SolveError> {
            // A zero-cycle linear solve gauges and copies its output. Keep
            // that rounding out of the nonlinear iterate being measured.
            let mut probe = p.to_vec();
            Ok(m.solve(
                &mut probe,
                Options {
                    max_cycles: 0,
                    ..linear
                },
            )?
            .residual_history[0])
        };
        let mut current = residual(&material, potential)?;
        let mut report = NonlinearReport {
            converged: false,
            iterations: 0,
            inner_cycles: 0,
            residual_history: vec![current],
            field_history: Vec::new(),
            compute: Duration::ZERO,
        };
        let mut relaxation = outer.relaxation;
        for iteration in 0..outer.max_iterations {
            let previous = potential.to_vec();
            let inner = Options {
                relative_tolerance: (linear.relative_tolerance * 0.5)
                    .max(current.relative * 0.02)
                    .min(1e-3),
                absolute_tolerance: linear.absolute_tolerance * 0.5,
                ..linear
            };
            let (converged, cycles, inner_residual) = if let Some(gpu) = gpu {
                let r = material.solve_gpu(gpu, potential, inner)?;
                if let Err(error) = r.check_converged() {
                    potential.copy_from_slice(&previous);
                    return Err(error);
                }
                (
                    r.converged,
                    r.iterations,
                    *r.residual_history.last().unwrap(),
                )
            } else {
                let r = material.solve(potential, inner)?;
                (
                    r.converged,
                    r.iterations,
                    *r.residual_history.last().unwrap(),
                )
            };
            report.inner_cycles += cycles;
            if !converged {
                potential.copy_from_slice(&previous);
                return Err(format!(
                    "nonlinear inner solve did not converge at outer iteration {} after {cycles} cycles: relative residual {:.9e}, target {:.9e}",
                    iteration + 1, inner_residual.relative, inner.relative_tolerance
                ).into());
            }
            let step: Vec<_> = potential
                .iter()
                .zip(&previous)
                .map(|(v, old)| v - old)
                .collect();
            let (next_fields, next_material, next_residual) = loop {
                potential.par_iter_mut().enumerate().for_each(|(i, v)| {
                    if !self.fixed[i] {
                        *v = previous[i] + relaxation * step[i];
                    }
                });
                let f = self.fields(potential);
                let m = self.material(&f)?;
                let r = residual(&m, potential)?;
                if r.absolute <= current.absolute * (1.0 + 1e-8) || relaxation <= 1.0 / 128.0 {
                    break (f, m, r);
                }
                relaxation *= 0.5;
            };
            let scale = next_fields
                .par_iter()
                .map(|e| e.iter().map(|v| v * v).sum::<f64>().sqrt())
                .reduce(|| 0.0, f64::max);
            let change = fields
                .par_iter()
                .zip(&next_fields)
                .map(|(a, b)| (0..3).map(|j| (a[j] - b[j]).powi(2)).sum::<f64>().sqrt())
                .reduce(|| 0.0, f64::max);
            let field_change = if scale == 0.0 { change } else { change / scale };
            report.iterations = iteration + 1;
            report.residual_history.push(next_residual);
            report.field_history.push(field_change);
            report.converged = next_residual.absolute <= linear.absolute_tolerance
                || next_residual.relative <= linear.relative_tolerance;
            report.converged &= field_change <= outer.field_tolerance;
            if report.converged {
                break;
            }
            fields = next_fields;
            material = next_material;
            current = next_residual;
        }
        report.compute = start.elapsed();
        Ok(report)
    }
}
