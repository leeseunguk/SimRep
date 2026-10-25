//! Represented charges, deterministic parallel Coulomb sums, and electrostatic CIC PIC.
use rayon::prelude::*;
use simrep_solve::mg::{self, Geometry, Grid, Options};

use crate::{ELECTRIC_ACCELERATION, FlyError, Particle, Vector, dot};

pub const ELEMENTARY_CHARGE_C: f64 = 1.602_176_634e-19;
/// Vacuum permittivity in F/mm; rho in C/mm³ gives rho/epsilon in V/mm².
pub const EPSILON_0_F_PER_MM: f64 = 8.854_187_812_8e-15;
pub const COULOMB_V_MM_PER_C: f64 = 1.0 / (4.0 * std::f64::consts::PI * EPSILON_0_F_PER_MM);

#[derive(Clone, Copy, Debug)]
pub enum ChargeMode {
    /// Multiply physical charge by this nonnegative number and the particle CWF.
    Factor(f64),
    /// Total absolute represented charge in C, shared in proportion to |q_e|*CWF.
    Coulombic(f64),
}

impl ChargeMode {
    pub fn charges(self, particles: &[Particle], weights: &[f64]) -> Result<Vec<f64>, FlyError> {
        let amount = match self {
            Self::Factor(v) | Self::Coulombic(v) => v,
        };
        if !amount.is_finite()
            || amount < 0.0
            || (!weights.is_empty() && weights.len() != particles.len())
            || weights.iter().any(|v| !v.is_finite() || *v < 0.0)
        {
            return Err(FlyError(
                "charge amount and CWFs must be finite and nonnegative; CWF count must match particles",
            ));
        }
        let weight = |i: usize| weights.get(i).copied().unwrap_or(1.0);
        let total: f64 = particles
            .iter()
            .enumerate()
            .map(|(i, p)| p.charge_e.abs() * weight(i))
            .sum();
        if !total.is_finite()
            || (matches!(self, Self::Coulombic(_)) && amount > 0.0 && total == 0.0)
        {
            return Err(FlyError(
                "coulombic charge needs a nonzero finite charged CWF sum",
            ));
        }
        let charges: Vec<_> = particles
            .iter()
            .enumerate()
            .map(|(i, p)| match self {
                Self::Factor(_) => amount * p.charge_e * ELEMENTARY_CHARGE_C * weight(i),
                Self::Coulombic(_) if p.charge_e != 0.0 && total > 0.0 => {
                    amount * p.charge_e * weight(i) / total
                }
                Self::Coulombic(_) => 0.0,
            })
            .collect();
        if charges.iter().any(|q| !q.is_finite()) {
            return Err(FlyError("represented charge overflows"));
        }
        Ok(charges)
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct DirectCharge {
    /// Plummer softening length. Zero is exact point Coulomb; coincident nonzero
    /// point sources are rejected, never silently discarded.
    pub softening_mm: f64,
}

impl DirectCharge {
    pub fn field(&self, positions: &[Vector], charges_c: &[f64]) -> Result<Vec<Vector>, FlyError> {
        if positions.len() != charges_c.len()
            || !self.softening_mm.is_finite()
            || self.softening_mm < 0.0
        {
            return Err(FlyError("invalid direct space-charge inputs or softening"));
        }
        let target = |(i, p): (usize, &Vector)| {
            let mut e = [0.0; 3];
            for (j, (other, &q)) in positions.iter().zip(charges_c).enumerate() {
                if i == j || q == 0.0 {
                    continue;
                }
                let r: Vector = std::array::from_fn(|a| p[a] - other[a]);
                let r2 = dot(r, r) + self.softening_mm.powi(2);
                let scale = COULOMB_V_MM_PER_C * q / (r2 * r2.sqrt());
                for a in 0..3 {
                    e[a] += scale * r[a];
                }
            }
            e
        };
        // At small live populations the two Rayon launches per global step
        // cost more than the pair sums. Keep each target's source order intact.
        let fields: Vec<Vector> = if positions.len() < 256 {
            positions.iter().enumerate().map(target).collect()
        } else {
            positions.par_iter().enumerate().map(target).collect()
        };
        if fields.iter().flatten().any(|v| !v.is_finite()) {
            return Err(FlyError(
                "singular or non-finite Coulomb field; resolve the encounter or specify softening",
            ));
        }
        Ok(fields)
    }

    pub(crate) fn time_scale(
        &self,
        positions: &[Vector],
        velocities: &[Vector],
        charges: &[f64],
        ratios: &[f64],
    ) -> f64 {
        let target = |(i, p): (usize, &Vector)| {
            let mut limit = f64::INFINITY;
            for j in 0..positions.len() {
                if i == j || charges[j] == 0.0 || ratios[i] == 0.0 {
                    continue;
                }
                let r: Vector = std::array::from_fn(|a| p[a] - positions[j][a]);
                let dv: Vector = std::array::from_fn(|a| velocities[i][a] - velocities[j][a]);
                let r2 = dot(r, r) + self.softening_mm.powi(2);
                let acceleration =
                    ELECTRIC_ACCELERATION * ratios[i].abs() * COULOMB_V_MM_PER_C * charges[j].abs();
                limit = limit.min((r2 * r2.sqrt() / acceleration).sqrt());
                let speed = dot(dv, dv).sqrt();
                if speed > 0.0 {
                    limit = limit.min(r2.sqrt() / speed);
                }
            }
            limit
        };
        if positions.len() < 256 {
            positions
                .iter()
                .enumerate()
                .map(target)
                .fold(f64::INFINITY, f64::min)
        } else {
            positions
                .par_iter()
                .enumerate()
                .map(target)
                .reduce(|| f64::INFINITY, f64::min)
        }
    }
}

/// Separate node-centred mesh. Planar coordinates are Cartesian mm. Cylindrical
/// storage axes are r,z (nz=1), physical axis is Cartesian z through origin.
/// A planar nz=1 mesh represents a sheet extruded through spacing[2] mm.
/// Fixed nodes impose zero *space-charge correction*; add the external field
/// separately to retain its electrode voltage, including time-varying voltage.
pub struct PicMesh {
    pub grid: Grid,
    pub origin_mm: Vector,
    pub options: Options,
    pub(super) fixed: Vec<bool>,
    potential: Vec<f64>,
    electric: Vec<Vector>,
    pub last_cycles: usize,
    pub last_residual: f64,
    plasma_frequency_per_us: f64,
}

impl PicMesh {
    pub fn new(
        grid: Grid,
        origin_mm: Vector,
        electrodes: Vec<bool>,
        options: Options,
    ) -> Result<Self, FlyError> {
        let count = grid
            .dims
            .into_iter()
            .try_fold(1usize, usize::checked_mul)
            .ok_or(FlyError("PIC grid size overflow"))?;
        if electrodes.len() != count || origin_mm.iter().any(|v| !v.is_finite()) {
            return Err(FlyError("invalid PIC origin or electrode mask"));
        }
        // Validate with the same contract as the solver, including an anchor.
        let mut potential = vec![0.0; count];
        mg::solve(&mut potential, &electrodes, grid, options)
            .map_err(|_| FlyError("invalid PIC mesh or Poisson options"))?;
        Ok(Self {
            grid,
            origin_mm,
            options,
            fixed: electrodes,
            potential,
            electric: vec![[0.0; 3]; count],
            last_cycles: 0,
            last_residual: 0.0,
            plasma_frequency_per_us: 0.0,
        })
    }

    pub fn potentials(&self) -> &[f64] {
        &self.potential
    }

    fn mesh_position(&self, p: Vector) -> Vector {
        let q: Vector = std::array::from_fn(|a| p[a] - self.origin_mm[a]);
        match self.grid.geometry {
            Geometry::Planar => q,
            Geometry::Cylindrical { radial_origin } => {
                [q[0].hypot(q[1]) - radial_origin, q[2], 0.0]
            }
        }
    }

    fn stencil(&self, p: Vector) -> Result<Vec<(usize, f64)>, FlyError> {
        let q = self.mesh_position(p);
        let mut nodes = [(0usize, 1.0)].to_vec();
        let mut stride = 1;
        for (a, &coordinate) in q.iter().enumerate() {
            let n = self.grid.dims[a];
            if n > 1 {
                let u = coordinate / self.grid.spacing[a];
                if !u.is_finite() || u < 0.0 || u > (n - 1) as f64 {
                    return Err(FlyError("live particle lies outside the PIC mesh"));
                }
                let lo = (u.floor() as usize).min(n - 2);
                let f = u - lo as f64;
                nodes = nodes
                    .into_iter()
                    .flat_map(|(i, w)| {
                        [
                            (i + stride * lo, w * (1.0 - f)),
                            (i + stride * (lo + 1), w * f),
                        ]
                    })
                    .collect();
            }
            stride *= n;
        }
        Ok(nodes)
    }

    /// Nodal dual-cell volume in mm³, including cylindrical ring Jacobian.
    pub fn volume_mm3(&self, i: usize) -> f64 {
        let c = [
            i % self.grid.dims[0],
            (i / self.grid.dims[0]) % self.grid.dims[1],
            i / (self.grid.dims[0] * self.grid.dims[1]),
        ];
        let width = |a: usize| {
            self.grid.spacing[a]
                * if self.grid.dims[a] > 1 && (c[a] == 0 || c[a] + 1 == self.grid.dims[a]) {
                    0.5
                } else {
                    1.0
                }
        };
        match self.grid.geometry {
            Geometry::Planar => (0..3).map(width).product(),
            Geometry::Cylindrical { radial_origin } => {
                let h = self.grid.spacing[0];
                let r = radial_origin + c[0] as f64 * h;
                let lo = if c[0] == 0 {
                    radial_origin
                } else {
                    r - 0.5 * h
                };
                let hi = if c[0] + 1 == self.grid.dims[0] {
                    r
                } else {
                    r + 0.5 * h
                };
                std::f64::consts::PI * (hi * hi - lo * lo) * width(1)
            }
        }
    }

    /// CIC deposit conserves charge in vacuum. Portions whose cloud overlaps a
    /// conductor go to that conductor and do not act as a vacuum source.
    pub fn deposit(&self, positions: &[Vector], charges: &[f64]) -> Result<Vec<f64>, FlyError> {
        if positions.len() != charges.len() {
            return Err(FlyError("PIC charge count mismatch"));
        }
        let mut rho = vec![0.0; self.potential.len()];
        for (&p, &q) in positions.iter().zip(charges) {
            for (i, w) in self.stencil(p)? {
                if !self.fixed[i] {
                    rho[i] += q * w;
                }
            }
        }
        for (i, v) in rho.iter_mut().enumerate() {
            *v /= self.volume_mm3(i);
        }
        Ok(rho)
    }

    pub fn field(
        &mut self,
        positions: &[Vector],
        charges: &[f64],
        ratios: &[f64],
    ) -> Result<Vec<Vector>, FlyError> {
        let mut source = self.deposit(positions, charges)?;
        for v in &mut source {
            *v /= EPSILON_0_F_PER_MM;
        }
        let max_source = source.iter().map(|v| v.abs()).fold(0.0, f64::max);
        let max_ratio = ratios.iter().map(|v| v.abs()).fold(0.0, f64::max);
        self.plasma_frequency_per_us = (ELECTRIC_ACCELERATION * max_ratio * max_source).sqrt();
        let report = mg::solve_poisson(
            &mut self.potential,
            &self.fixed,
            &source,
            self.grid,
            self.options,
        )
        .map_err(|_| FlyError("PIC Poisson solve failed"))?;
        self.last_cycles = report.iterations;
        self.last_residual = report.residual_history.last().unwrap().absolute;
        if !report.converged {
            return Err(FlyError("PIC Poisson iteration budget exhausted"));
        }
        let strides = [1, self.grid.dims[0], self.grid.dims[0] * self.grid.dims[1]];
        for i in 0..self.potential.len() {
            let c = [
                i % self.grid.dims[0],
                (i / self.grid.dims[0]) % self.grid.dims[1],
                i / strides[2],
            ];
            for a in 0..3 {
                let n = self.grid.dims[a];
                self.electric[i][a] = if n == 1
                    || (!self.fixed[i] && (c[a] == 0 || c[a] + 1 == n))
                    || (matches!(
                        self.grid.geometry,
                        Geometry::Cylindrical { radial_origin: 0.0 }
                    ) && a == 0
                        && c[a] == 0)
                {
                    0.0
                } else {
                    let lo = c[a].saturating_sub(1);
                    let hi = (c[a] + 1).min(n - 1);
                    -(self.potential[i + (hi - c[a]) * strides[a]]
                        - self.potential[i - (c[a] - lo) * strides[a]])
                        / ((hi - lo) as f64 * self.grid.spacing[a])
                };
            }
        }
        positions
            .iter()
            .map(|&p| {
                let mut e = [0.0; 3];
                for (i, w) in self.stencil(p)? {
                    for (a, v) in e.iter_mut().enumerate() {
                        *v += w * self.electric[i][a];
                    }
                }
                if matches!(self.grid.geometry, Geometry::Cylindrical { .. }) {
                    let x = p[0] - self.origin_mm[0];
                    let y = p[1] - self.origin_mm[1];
                    let r = x.hypot(y);
                    Ok(if r == 0.0 {
                        [0.0, 0.0, e[1]]
                    } else {
                        [e[0] * x / r, e[0] * y / r, e[1]]
                    })
                } else {
                    Ok(e)
                }
            })
            .collect()
    }

    pub(crate) fn time_scale(&self, velocities: &[Vector]) -> f64 {
        let cell = self.grid.spacing.into_iter().fold(f64::INFINITY, f64::min);
        let speed = velocities
            .iter()
            .map(|v| dot(*v, *v).sqrt())
            .fold(0.0, f64::max);
        (1.0 / self.plasma_frequency_per_us).min(cell / speed)
    }
}

pub enum SpaceCharge {
    Direct(DirectCharge),
    Pic(Box<PicMesh>),
    GpuPic(Box<super::gpu_pic::GpuPic>),
}
