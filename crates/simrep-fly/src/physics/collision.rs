//! Seeded HS1 elastic collisions and empirical SDS diffusion.
//!
//! Independent streams are keyed by input ion index, not Rayon scheduling.
//! No SIMION data are embedded: SDS requires the installed mbmr.dat at runtime.

use std::{f64::consts::PI, path::Path};

use rayon::prelude::*;

use crate::{
    ELECTRIC_ACCELERATION, Field, FlightOptions, FlightResult, FlyError, MAGNETIC_FREQUENCY,
    Particle, Vector, cross, dot,
};

const HS_K: f64 = 1.3806505e-23;
const HS_AMU: f64 = 1.6605402e-27;
const SDS_K: f64 = 1.3806503e-23;
const SDS_AMU: f64 = 1.66053873e-27;
const SDS_E: f64 = 1.602176462e-19;
const STP_T: f64 = 273.15;

/// SplitMix64 with a 52-bit open uniform, polar Box-Muller normal and
/// isotropic sphere sampler. Integer/uniform sequences are platform-stable;
/// transcendental f64 rounding can differ between system math libraries.
pub struct Rng {
    state: u64,
}

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self { state: mix(seed) }
    }
    pub fn uniform(&mut self) -> f64 {
        self.state = self.state.wrapping_add(0x9e3779b97f4a7c15);
        let z = mix(self.state);
        ((z >> 12) as f64 + 0.5) * (1.0 / 4503599627370496.0)
    }
    fn normal(&mut self) -> f64 {
        loop {
            let x = 2.0 * self.uniform() - 1.0;
            let y = 2.0 * self.uniform() - 1.0;
            let s = x * x + y * y;
            if s > 0.0 && s < 1.0 {
                return x * (-2.0 * s.ln() / s).sqrt();
            }
        }
    }
    fn sphere(&mut self, radius: f64) -> Vector {
        let z = 2.0 * self.uniform() - 1.0;
        let phi = 2.0 * PI * self.uniform();
        let r = radius * (1.0 - z * z).sqrt();
        [r * phi.cos(), r * phi.sin(), radius * z]
    }
}

// Hash the seed as well as each draw. Without this initial avalanche, adjacent
// ion seeds separated by the generator increment are shifted copies of one
// stream; long trajectories would reuse each other's random subsequences.
fn mix(mut z: u64) -> u64 {
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb);
    z ^ (z >> 31)
}

#[derive(Clone, Debug)]
pub struct Gas {
    pub pressure_pa: f64,
    pub temperature_k: f64,
    pub mass_amu: f64,
    pub velocity_mm_per_us: Vector,
}

impl Default for Gas {
    fn default() -> Self {
        Self {
            pressure_pa: 0.53,
            temperature_k: 273.0,
            mass_amu: 4.0,
            velocity_mm_per_us: [0.0; 3],
        }
    }
}

impl Gas {
    fn validate(&self) -> Result<(), FlyError> {
        if !self.pressure_pa.is_finite()
            || self.pressure_pa < 0.0
            || !self.temperature_k.is_finite()
            || self.temperature_k <= 0.0
            || !self.mass_amu.is_finite()
            || self.mass_amu <= 0.0
            || self.velocity_mm_per_us.iter().any(|v| !v.is_finite())
        {
            return Err(FlyError(
                "gas requires pressure>=0, temperature>0, mass>0 and finite velocity",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct Hs1 {
    pub gas: Gas,
    pub cross_section_m2: f64,
    /// Optional fixed mean free path, as in HS1's _mean_free_path_mm.
    pub mean_free_path_mm: Option<f64>,
    /// Maximum integrated collision hazard per deterministic step.
    pub steps_per_mean_free_time: f64,
}

impl Default for Hs1 {
    fn default() -> Self {
        Self {
            gas: Gas::default(),
            cross_section_m2: 2.27e-18,
            mean_free_path_mm: None,
            steps_per_mean_free_time: 20.0,
        }
    }
}

impl Hs1 {
    pub fn validate(&self) -> Result<(), FlyError> {
        self.gas.validate()?;
        if !self.cross_section_m2.is_finite()
            || self.cross_section_m2 <= 0.0
            || !self.steps_per_mean_free_time.is_finite()
            || self.steps_per_mean_free_time < 1.0
            || self
                .mean_free_path_mm
                .is_some_and(|v| !v.is_finite() || v <= 0.0)
        {
            return Err(FlyError(
                "HS1 requires positive cross section/MFP and steps per mean time >=1",
            ));
        }
        Ok(())
    }

    /// Mean relative gas/ion speed, including the analytic zero-speed limit.
    pub fn mean_relative_speed(&self, ion_velocity: Vector) -> f64 {
        let v: Vector = std::array::from_fn(|a| ion_velocity[a] - self.gas.velocity_mm_per_us[a]);
        let speed = dot(v, v).sqrt();
        let star =
            (2.0 * HS_K * self.gas.temperature_k / (self.gas.mass_amu * HS_AMU)).sqrt() / 1000.0;
        let s = speed / star;
        let mean = 2.0 * star / PI.sqrt();
        if s < 1e-4 {
            mean * (1.0 + s * s / 3.0 - s.powi(4) / 30.0)
        } else {
            mean * ((s + 0.5 / s) * 0.5 * PI.sqrt() * erf(s) + 0.5 * (-s * s).exp())
        }
    }

    pub fn collision_rate_per_us(&self, velocity: Vector) -> f64 {
        if self.gas.pressure_pa == 0.0 {
            return 0.0;
        }
        if let Some(mfp) = self.mean_free_path_mm {
            let v: Vector = std::array::from_fn(|a| velocity[a] - self.gas.velocity_mm_per_us[a]);
            return dot(v, v).sqrt().max(1e-7) / mfp;
        }
        self.gas.pressure_pa * self.cross_section_m2 / (1000.0 * HS_K * self.gas.temperature_k)
            * self.mean_relative_speed(velocity)
    }

    pub fn mean_free_path(&self, velocity: Vector) -> f64 {
        let v: Vector = std::array::from_fn(|a| velocity[a] - self.gas.velocity_mm_per_us[a]);
        let rate = self.collision_rate_per_us(velocity);
        if rate == 0.0 {
            f64::INFINITY
        } else {
            dot(v, v).sqrt() / rate
        }
    }

    /// Mason-Schamp low-field hard-sphere mobility in m^2/(V s).
    /// This first Chapman-Enskog limit is an approximation, not an exact
    /// finite-field solution of the linear Boltzmann equation.
    pub fn low_field_mobility(&self, ion_mass_amu: f64, charge_e: f64) -> f64 {
        let mu = ion_mass_amu * self.gas.mass_amu / (ion_mass_amu + self.gas.mass_amu) * HS_AMU;
        let n = self.gas.pressure_pa / (HS_K * self.gas.temperature_k);
        3.0 * charge_e.abs() * 1.602176634e-19 / (16.0 * n * self.cross_section_m2)
            * (2.0 * PI / (mu * HS_K * self.gas.temperature_k)).sqrt()
    }

    /// Velocity-weighted thermal gas sampling and isotropic relative
    /// outgoing direction in the COM frame. This is distribution-equivalent
    /// to HS1's sqrt(U) impact offset and uniform impact-plane angle.
    pub fn scatter(&self, ion_mass: f64, velocity: Vector, rng: &mut Rng) -> Vector {
        let v: Vector = std::array::from_fn(|a| velocity[a] - self.gas.velocity_mm_per_us[a]);
        let sigma = (HS_K * self.gas.temperature_k / (self.gas.mass_amu * HS_AMU)).sqrt() / 1000.0;
        let scale = dot(v, v).sqrt() + sigma * 1.732 * 3.0;
        let gas = loop {
            let g: Vector = std::array::from_fn(|_| rng.normal() * sigma);
            let relative: Vector = std::array::from_fn(|a| g[a] - v[a]);
            if rng.uniform() < dot(relative, relative).sqrt() / scale {
                break g;
            }
        };
        let relative: Vector = std::array::from_fn(|a| v[a] - gas[a]);
        let outgoing = rng.sphere(dot(relative, relative).sqrt());
        let total = ion_mass + self.gas.mass_amu;
        std::array::from_fn(|a| {
            (ion_mass * v[a] + self.gas.mass_amu * gas[a] + self.gas.mass_amu * outgoing[a]) / total
                + self.gas.velocity_mm_per_us[a]
        })
    }
}

// The same approximation and constants as the shipped HS1 example.
fn erf(z: f64) -> f64 {
    let t = 1.0 / (1.0 + 0.32759109962 * z.abs());
    let p = ((((-1.061405429 * t + 1.453152027) * t - 1.421413741) * t + 0.2844966736) * t
        - 0.254829592)
        * t;
    (1.0 + p * (-z * z).exp()).copysign(z)
}

pub struct DiffusionStatistics {
    values: Vec<f64>,
}

impl DiffusionStatistics {
    /// The five validated 1002-point tables, in their file order.
    pub(crate) fn gpu_values(&self) -> &[f64] {
        &self.values
    }

    pub fn from_path(
        path: impl AsRef<Path>,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let text = std::fs::read_to_string(path)?;
        let values = text
            .lines()
            .flat_map(|line| line.split(';').next().unwrap().split_whitespace())
            .map(str::parse::<f64>)
            .collect::<Result<Vec<_>, _>>()?;
        if values.len() != 5 * 1002
            || values
                .iter()
                .enumerate()
                .any(|(i, v)| !v.is_finite() || *v < 0.0 || (*v == 0.0 && i % 1002 != 0))
            || values
                .chunks_exact(1002)
                .any(|icdf| icdf.windows(2).any(|w| w[0] > w[1]))
        {
            return Err("mbmr.dat must contain five nonnegative, monotone 1002-point ICDFs (zero only at the origin)".into());
        }
        Ok(Self { values })
    }

    pub fn quantile(&self, log_mass_ratio: f64, percentile: f64) -> f64 {
        assert!(log_mass_ratio.is_finite() && (0.0..=1.0).contains(&percentile));
        if percentile == 0.0 {
            return 0.0;
        }
        let n = percentile * 1000.0;
        let low = n.floor() as usize;
        let weight = n - low as f64;
        let table = if log_mass_ratio <= 1.0 {
            0
        } else if log_mass_ratio <= 2.0 {
            1
        } else if log_mass_ratio <= 3.0 {
            2
        } else {
            3
        };
        let interpolate = |i: usize| {
            let start = i * 1002 + low;
            self.values[start] + weight * (self.values[start + 1] - self.values[start])
        };
        let a = interpolate(table).log10();
        let b = interpolate(table + 1).log10();
        10.0_f64.powf(a + (log_mass_ratio - table as f64) * (b - a))
    }
}

pub struct Sds {
    pub gas: Gas,
    pub gas_diameter_nm: f64,
    pub ion_diameter_nm: Option<f64>,
    /// Reduced mobility at STP, in 1e-4 m^2/(V s). None uses SDS's
    /// empirical diameter/mobility estimates (not HS1's cross section).
    pub mobility_stp: Option<f64>,
    pub diffusion: bool,
    pub statistics: DiffusionStatistics,
}

#[derive(Clone, Copy, Debug)]
pub struct SdsTransport {
    pub damping_per_us: f64,
    pub mean_free_path_mm: f64,
    pub mean_speed_mm_per_us: f64,
    pub mobility_stp: f64,
}

impl Sds {
    pub fn validate(&self) -> Result<(), FlyError> {
        self.gas.validate()?;
        if !self.gas_diameter_nm.is_finite()
            || self.gas_diameter_nm <= 0.0
            || [self.ion_diameter_nm, self.mobility_stp]
                .into_iter()
                .flatten()
                .any(|x| !x.is_finite() || x <= 0.0)
        {
            return Err(FlyError(
                "SDS requires positive diameters and reduced mobility",
            ));
        }
        Ok(())
    }

    pub fn transport(&self, mass: f64, charge: f64) -> SdsTransport {
        let air_to_gas = |d: f64| {
            let reduced_air = mass * 28.94515 / (mass + 28.94515);
            let reduced_gas = mass * self.gas.mass_amu / (mass + self.gas.mass_amu);
            ((d + 0.366) / (d + self.gas_diameter_nm)).powi(2) * (reduced_air / reduced_gas).sqrt()
        };
        let mut diameter = self.ion_diameter_nm.unwrap_or(0.120415405 * mass.cbrt());
        if self.ion_diameter_nm.is_none()
            && let Some(ko) = self.mobility_stp
        {
            let logkm = (ko / air_to_gas(diameter) * 1e5).log10();
            diameter = 10.0_f64
                .powf(3.0367 - 0.8504 * logkm + 0.1137 * logkm.powi(2) - 0.0135 * logkm.powi(3));
        }
        let ko = self.mobility_stp.unwrap_or_else(|| {
            let d = diameter.log10();
            1e-5 * 10.0_f64.powf(4.9137 - 1.4491 * d - 0.2772 * d * d + 0.0717 * d.powi(3))
                * air_to_gas(diameter)
        });
        let vk = (8.0 * SDS_K * STP_T / (PI * SDS_AMU)).sqrt() * 1e-3;
        let vi = vk / mass.sqrt();
        let vg = vk / self.gas.mass_amu.sqrt();
        let n = 6.02214199e23 / 22.413996e-3 / 1e9;
        let frequency = 1e-12
            * n
            * PI
            * ((2.0_f64.sqrt() - 0.25) * ((self.gas_diameter_nm + diameter) / 2.0).powi(2) * vi
                + 0.25 * diameter * diameter * vg);
        let t_ratio = self.gas.temperature_k / STP_T;
        let pt_ratio = t_ratio * (760.0 * 133.322 / self.gas.pressure_pa);
        SdsTransport {
            damping_per_us: SDS_E / SDS_AMU * 0.01 / ko * charge.abs() / mass / pt_ratio,
            mean_free_path_mm: vi / frequency * pt_ratio,
            mean_speed_mm_per_us: vi * t_ratio.sqrt(),
            mobility_stp: ko,
        }
    }
}

pub enum CollisionModel {
    Hs1(Hs1),
    Sds(Sds),
}

impl CollisionModel {
    pub fn validate(&self) -> Result<(), FlyError> {
        match self {
            Self::Hs1(hs) => hs.validate(),
            Self::Sds(sds) => sds.validate(),
        }
    }
}

/// Collisional flights are nonrelativistic. Randomness is consumed only
/// after an accepted step; trial rejection and event bisection are pure.
pub fn fly_collisional(
    field: &impl Field,
    particle: &Particle,
    options: &FlightOptions,
    model: &CollisionModel,
    seed: u64,
) -> Result<FlightResult, FlyError> {
    options.validate()?;
    particle.validate(false)?;
    model.validate()?;
    if options.relativistic {
        return Err(FlyError(
            "neutral gas collision models require nonrelativistic dynamics",
        ));
    }
    if matches!(model, CollisionModel::Sds(_)) && particle.charge_e == 0.0 {
        return Err(FlyError("SDS mobility requires nonzero ion charge"));
    }
    let mut process = Process::new(model, particle, seed);
    Ok(crate::flight::fly_with_process(
        field,
        particle,
        options,
        &mut process,
    ))
}

pub fn fly_many_collisional(
    field: &impl Field,
    particles: &[Particle],
    options: &FlightOptions,
    model: &CollisionModel,
    seed: u64,
) -> Result<Vec<FlightResult>, FlyError> {
    particles
        .par_iter()
        .enumerate()
        .map(|(i, p)| {
            fly_collisional(
                field,
                p,
                options,
                model,
                seed.wrapping_add((i as u64).wrapping_mul(0x9e3779b97f4a7c15)),
            )
        })
        .collect()
}

pub(crate) struct Process<'a> {
    model: &'a CollisionModel,
    mass: f64,
    rng: Rng,
    hazard: f64,
    transport: Option<SdsTransport>,
}

impl<'a> Process<'a> {
    fn new(model: &'a CollisionModel, particle: &Particle, seed: u64) -> Self {
        let mut rng = Rng::new(seed);
        let hazard = -rng.uniform().ln();
        let transport = match model {
            CollisionModel::Sds(s) => Some(s.transport(particle.mass_amu, particle.charge_e)),
            _ => None,
        };
        Self {
            model,
            mass: particle.mass_amu,
            rng,
            hazard,
            transport,
        }
    }
    pub(crate) fn limit_step(&self, y: &[f64; 6]) -> f64 {
        if let CollisionModel::Hs1(hs) = self.model {
            let rate = hs.collision_rate_per_us([y[3], y[4], y[5]]);
            if rate > 0.0 {
                return self.hazard.min(1.0 / hs.steps_per_mean_free_time) / rate;
            }
        }
        f64::INFINITY
    }
    pub(crate) fn trial(
        &self,
        field: &impl Field,
        y: &[f64; 6],
        t: f64,
        h: f64,
        ratio: f64,
        options: &FlightOptions,
    ) -> Option<([f64; 6], f64)> {
        let CollisionModel::Sds(sds) = self.model else {
            return None;
        };
        if sds.gas.pressure_pa == 0.0 {
            return None;
        }
        let damping = self.transport.unwrap().damping_per_us;
        let advance = |y: &[f64; 6], time: f64, dt: f64| {
            let at_start = field.sample([y[0], y[1], y[2]], time);
            let mid = damped_step(
                y,
                &at_start,
                ratio,
                damping,
                sds.gas.velocity_mm_per_us,
                dt * 0.5,
            );
            let sample = field.sample([mid[0], mid[1], mid[2]], time + dt * 0.5);
            damped_step(y, &sample, ratio, damping, sds.gas.velocity_mm_per_us, dt)
        };
        let full = advance(y, t, h);
        let half = advance(y, t, h * 0.5);
        let two = advance(&half, t + h * 0.5, h * 0.5);
        let error = (0..6)
            .map(|a| {
                let absolute = if a < 3 {
                    options.position_tolerance_mm
                } else {
                    options.velocity_tolerance_mm_per_us
                };
                (two[a] - full[a]).abs()
                    / (3.0 * (absolute + options.relative_tolerance * y[a].abs().max(two[a].abs())))
            })
            .fold(0.0_f64, f64::max);
        Some((two, error))
    }
    pub(crate) fn accepted(&mut self, from: &[f64; 6], next: &mut [f64; 6], dt: f64) {
        match self.model {
            CollisionModel::Hs1(hs) => {
                let rate = hs.collision_rate_per_us([from[3], from[4], from[5]]);
                self.hazard -= rate * dt;
                if self.hazard <= 1e-12 {
                    let v = hs.scatter(self.mass, [next[3], next[4], next[5]], &mut self.rng);
                    next[3..].copy_from_slice(&v);
                    self.hazard = -self.rng.uniform().ln();
                }
            }
            CollisionModel::Sds(sds) if sds.diffusion && sds.gas.pressure_pa > 0.0 => {
                let transport = self.transport.unwrap();
                let distance = sds
                    .statistics
                    .quantile((self.mass / sds.gas.mass_amu).log10(), self.rng.uniform());
                let n = transport.mean_speed_mm_per_us / transport.mean_free_path_mm * dt;
                let r = (n / 100000.0).sqrt() * distance * transport.mean_free_path_mm;
                let jump = self.rng.sphere(r);
                for a in 0..3 {
                    next[a] += jump[a];
                }
            }
            _ => {}
        }
    }
}

// Integral functions F1=(exp(z*h)-1)/z and F2=(F1-h)/z for
// z=-damping+i*gyrofrequency. The imaginary component multiplies v cross b.
fn integrals(damping: f64, omega: f64, h: f64) -> ([f64; 2], [f64; 2], [f64; 2]) {
    let z = [-damping * h, omega * h];
    let exp = [z[0].exp() * z[1].cos(), z[0].exp() * z[1].sin()];
    if z[0].hypot(z[1]) < 0.1 {
        let mut term = [1.0, 0.0];
        let mut f1 = [1.0, 0.0];
        let mut f2 = [0.5, 0.0];
        for n in 1..20 {
            term = [
                (term[0] * z[0] - term[1] * z[1]) / (n + 1) as f64,
                (term[0] * z[1] + term[1] * z[0]) / (n + 1) as f64,
            ];
            for a in 0..2 {
                f1[a] += term[a];
                f2[a] += term[a] / (n + 2) as f64;
            }
        }
        return (exp, f1.map(|v| v * h), f2.map(|v| v * h * h));
    }
    let denominator = damping * damping + omega * omega;
    let divide = |a: [f64; 2]| {
        [
            (-damping * a[0] + omega * a[1]) / denominator,
            (-omega * a[0] - damping * a[1]) / denominator,
        ]
    };
    let f1 = divide([exp[0] - 1.0, exp[1]]);
    let f2 = divide([f1[0] - h, f1[1]]);
    (exp, f1, f2)
}

fn damped_step(
    y: &[f64; 6],
    sample: &crate::FieldSample,
    ratio: f64,
    damping: f64,
    gas: Vector,
    h: f64,
) -> [f64; 6] {
    let magnitude = dot(sample.magnetic_gauss, sample.magnetic_gauss).sqrt();
    let axis = if magnitude == 0.0 {
        [1.0, 0.0, 0.0]
    } else {
        sample.magnetic_gauss.map(|b| b / magnitude)
    };
    let omega = ratio * MAGNETIC_FREQUENCY * magnitude;
    let (decay, f1, f2) = integrals(damping, omega, h);
    let (parallel_decay, parallel_f1, parallel_f2) = integrals(damping, 0.0, h);
    let v = [y[3], y[4], y[5]];
    let force = std::array::from_fn(|a| {
        ratio * ELECTRIC_ACCELERATION * sample.electric_v_per_mm[a] + damping * gas[a]
    });
    let apply = |v: Vector, perpendicular: [f64; 2], parallel: f64| {
        let projection = dot(v, axis);
        let rotate = cross(v, axis);
        std::array::from_fn::<_, 3, _>(|a| {
            perpendicular[0] * (v[a] - projection * axis[a])
                + perpendicular[1] * rotate[a]
                + parallel * projection * axis[a]
        })
    };
    let damped = apply(v, decay, parallel_decay[0]);
    let impulse = apply(force, f1, parallel_f1[0]);
    let drift = apply(v, f1, parallel_f1[0]);
    let acceleration = apply(force, f2, parallel_f2[0]);
    let mut next = *y;
    for a in 0..3 {
        next[a] += drift[a] + acceleration[a];
        next[a + 3] = damped[a] + impulse[a];
    }
    next
}

#[cfg(test)]
mod tests {
    use super::*;

    // Exact SI constants (2019 SI definitions).
    const SI_E: f64 = 1.602176634e-19; // C
    const SI_K_B: f64 = 1.380649e-23; // J/K
    const SI_AMU: f64 = 1.66053906892e-27; // kg

    /// Formula evaluation of the Mason-Schamp low-field mobility with
    /// supplied constants, not a check of HS1 transport.
    /// K = 3|q| / (16 n sigma) sqrt(2 pi / (mu k T)) in m^2/(V s), with
    /// n = P/(kT) and mu the reduced ion/gas mass in kg.
    fn mason_schamp_mobility(
        e: f64,
        k_b: f64,
        amu: f64,
        gas: &Gas,
        sigma_m2: f64,
        ion_mass_amu: f64,
        charge_e: f64,
    ) -> f64 {
        let mu_kg = ion_mass_amu * gas.mass_amu / (ion_mass_amu + gas.mass_amu) * amu;
        let n = gas.pressure_pa / (k_b * gas.temperature_k);
        3.0 * charge_e.abs() * e / (16.0 * n * sigma_m2)
            * (2.0 * PI / (mu_kg * k_b * gas.temperature_k)).sqrt()
    }

    fn relative_error(got: f64, expected: f64) -> f64 {
        ((got - expected) / expected).abs()
    }

    /// Helium buffer gas at 0.53 Pa / 273 K, sigma = 2.27e-18 m^2, with a
    /// 100 amu singly charged ion.
    fn helium_hs1() -> Hs1 {
        Hs1 {
            gas: Gas {
                pressure_pa: 0.53,
                temperature_k: 273.0,
                mass_amu: 4.0,
                velocity_mm_per_us: [0.0; 3],
            },
            cross_section_m2: 2.27e-18,
            ..Default::default()
        }
    }

    /// Nitrogen buffer gas at 1.0 Pa / 300 K, sigma = 3.0e-18 m^2, with a
    /// 50 amu doubly charged ion.
    fn nitrogen_hs1() -> Hs1 {
        Hs1 {
            gas: Gas {
                pressure_pa: 1.0,
                temperature_k: 300.0,
                mass_amu: 28.0,
                velocity_mm_per_us: [0.0; 3],
            },
            cross_section_m2: 3.0e-18,
            ..Default::default()
        }
    }

    #[test]
    fn low_field_mobility_matches_formula_with_legacy_and_si_constants() {
        // Recomputed with this module's documented constants (the shipped
        // HS1 example's e/k_B/amu values), the formula must agree to
        // floating-point rounding.
        for (hs1, ion_mass_amu, charge_e) in
            [(helium_hs1(), 100.0, 1.0), (nitrogen_hs1(), 50.0, -2.0)]
        {
            let expected = mason_schamp_mobility(
                1.602176634e-19,
                HS_K,
                HS_AMU,
                &hs1.gas,
                hs1.cross_section_m2,
                ion_mass_amu,
                charge_e,
            );
            let got = hs1.low_field_mobility(ion_mass_amu, charge_e);
            assert!(
                relative_error(got, expected) < 1e-12,
                "K = {got}, formula evaluation = {expected}"
            );
        }

        // With exact SI constants the same recomputation differs by the
        // intentional legacy-constant offset in HS_K/HS_AMU (~2.03e-7);
        // the physics must still agree within that bound.
        for (hs1, ion_mass_amu, charge_e) in
            [(helium_hs1(), 100.0, 1.0), (nitrogen_hs1(), 50.0, -2.0)]
        {
            let expected = mason_schamp_mobility(
                SI_E,
                SI_K_B,
                SI_AMU,
                &hs1.gas,
                hs1.cross_section_m2,
                ion_mass_amu,
                charge_e,
            );
            let got = hs1.low_field_mobility(ion_mass_amu, charge_e);
            assert!(
                relative_error(got, expected) < 3e-7,
                "K = {got}, SI evaluation = {expected}"
            );
        }
    }

    #[test]
    fn low_field_mobility_scales_with_absolute_charge() {
        let hs1 = helium_hs1();
        let k = hs1.low_field_mobility(100.0, 1.0);

        // Doubling |q| doubles mobility, and the sign is absorbed by abs().
        let k2 = hs1.low_field_mobility(100.0, -2.0);
        assert!(relative_error(k2 / k, 2.0) < 1e-12);
    }
}
