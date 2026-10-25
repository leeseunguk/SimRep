use rayon::prelude::*;

use crate::{
    ELECTRIC_ACCELERATION, Field, FieldSample, FlyError, MAGNETIC_FREQUENCY, SPEED_OF_LIGHT,
    SplatReason, Vector, cross, dot,
};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ParticleState {
    pub position_mm: Vector,
    pub velocity_mm_per_us: Vector,
    /// Absolute time, including birth time.
    pub time_us: f64,
}

#[derive(Clone, Debug)]
pub struct Particle {
    pub mass_amu: f64,
    pub charge_e: f64,
    pub initial: ParticleState,
}

impl Particle {
    pub fn new(
        mass_amu: f64,
        charge_e: f64,
        position_mm: Vector,
        velocity_mm_per_us: Vector,
        birth_time_us: f64,
    ) -> Result<Self, FlyError> {
        let particle = Self {
            mass_amu,
            charge_e,
            initial: ParticleState {
                position_mm,
                velocity_mm_per_us,
                time_us: birth_time_us,
            },
        };
        particle.validate(false)?;
        Ok(particle)
    }

    /// Nonrelativistic kinetic energy in eV and a nonzero direction vector.
    pub fn from_energy(
        mass_amu: f64,
        charge_e: f64,
        position_mm: Vector,
        energy_ev: f64,
        direction: Vector,
        birth_time_us: f64,
    ) -> Result<Self, FlyError> {
        Self::energy(
            mass_amu,
            charge_e,
            position_mm,
            energy_ev,
            direction,
            birth_time_us,
            false,
        )
    }

    pub fn from_relativistic_energy(
        mass_amu: f64,
        charge_e: f64,
        position_mm: Vector,
        energy_ev: f64,
        direction: Vector,
        birth_time_us: f64,
    ) -> Result<Self, FlyError> {
        Self::energy(
            mass_amu,
            charge_e,
            position_mm,
            energy_ev,
            direction,
            birth_time_us,
            true,
        )
    }

    fn energy(
        mass: f64,
        charge: f64,
        position: Vector,
        energy: f64,
        direction: Vector,
        birth: f64,
        relativistic: bool,
    ) -> Result<Self, FlyError> {
        let norm = dot(direction, direction).sqrt();
        if !energy.is_finite() || energy < 0.0 || !norm.is_finite() || norm == 0.0 {
            return Err(FlyError(
                "energy must be nonnegative and direction finite and nonzero",
            ));
        }
        let speed = if relativistic {
            let k = energy * ELECTRIC_ACCELERATION / (mass * SPEED_OF_LIGHT * SPEED_OF_LIGHT);
            SPEED_OF_LIGHT * (k * (k + 2.0)).sqrt() / (k + 1.0)
        } else {
            (2.0 * energy * ELECTRIC_ACCELERATION / mass).sqrt()
        };
        Self::new(
            mass,
            charge,
            position,
            direction.map(|d| d * speed / norm),
            birth,
        )
    }

    pub(crate) fn validate(&self, relativistic: bool) -> Result<(), FlyError> {
        if !self.mass_amu.is_finite()
            || self.mass_amu <= 0.0
            || !self.charge_e.is_finite()
            || !self.initial.time_us.is_finite()
            || self
                .initial
                .position_mm
                .iter()
                .chain(&self.initial.velocity_mm_per_us)
                .any(|x| !x.is_finite())
        {
            return Err(FlyError(
                "particle mass must be positive and all state values finite",
            ));
        }
        if relativistic
            && dot(
                self.initial.velocity_mm_per_us,
                self.initial.velocity_mm_per_us,
            ) >= SPEED_OF_LIGHT.powi(2)
        {
            return Err(FlyError("relativistic initial speed must be below c"));
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Integrator {
    #[default]
    Rk45,
    Boris,
}

#[derive(Clone, Copy, Debug)]
pub struct StopPlane {
    /// Cartesian axis: 0 = x, 1 = y, 2 = z.
    pub axis: usize,
    pub coordinate_mm: f64,
}

#[derive(Clone, Debug)]
pub struct FlightOptions {
    pub integrator: Integrator,
    /// Flight duration measured from each particle's birth.
    pub duration_us: f64,
    /// Adaptive initial step, or fixed Boris step.
    pub initial_step_us: f64,
    pub min_step_us: f64,
    /// Explicit upper bound. `f64::MAX` leaves the ceiling to `step_limits`.
    pub max_step_us: f64,
    pub step_limits: StepLimits,
    /// Estimate the launch grid-error budget when numeric tolerances and the
    /// automatic step ceiling retain their defaults. False uses exact values.
    pub automatic_tolerance: bool,
    pub relative_tolerance: f64,
    pub position_tolerance_mm: f64,
    pub velocity_tolerance_mm_per_us: f64,
    pub max_steps: usize,
    pub relativistic: bool,
    /// Save the initial state, every N accepted steps, and the final state.
    pub sample_every: Option<usize>,
    pub stop_plane: Option<StopPlane>,
}

impl Default for FlightOptions {
    fn default() -> Self {
        Self {
            integrator: Integrator::Rk45,
            duration_us: 10.0,
            initial_step_us: 0.001,
            min_step_us: 1e-14,
            max_step_us: f64::MAX,
            step_limits: StepLimits::default(),
            automatic_tolerance: true,
            relative_tolerance: 1e-12,
            position_tolerance_mm: 1e-12,
            velocity_tolerance_mm_per_us: 1e-12,
            max_steps: 1_000_000,
            relativistic: false,
            sample_every: None,
            stop_plane: None,
        }
    }
}

impl FlightOptions {
    /// Resolve a bounded interpolation-error estimate for static electric grids.
    /// A forward fourth difference vanishes for cubic-potential fields. Missing
    /// grid metadata, magnetic stencil samples, known voltage time dependence,
    /// and explicit settings retain exact numeric tolerances. This estimates
    /// grid error, not a continuum bound.
    pub fn with_field_tolerance(&self, field: &impl Field, initial: ParticleState) -> Self {
        let mut resolved = self.clone();
        if !self.automatic_tolerance
            || self.max_step_us != f64::MAX
            || self.relative_tolerance != 1e-12
            || self.position_tolerance_mm != 1e-12
            || self.velocity_tolerance_mm_per_us != 1e-12
            || field.time_scale_us().is_some()
            || field.next_time_event(initial.time_us).is_some()
        {
            return resolved;
        }
        let Some(spacing) = field.spatial_scale_mm() else {
            return resolved;
        };
        let speed = dot(initial.velocity_mm_per_us, initial.velocity_mm_per_us).sqrt();
        if speed == 0.0 {
            return resolved;
        }
        let mut difference = [0.0; 3];
        let mut magnitude = 0.0_f64;
        for (i, weight) in [1.0, -4.0, 6.0, -4.0, 1.0].into_iter().enumerate() {
            let p = std::array::from_fn(|a| {
                initial.position_mm[a] + i as f64 * spacing * initial.velocity_mm_per_us[a] / speed
            });
            let sample = field.sample(p, initial.time_us);
            let electric = sample.electric_v_per_mm;
            if sample.magnetic_gauss != [0.0; 3] || electric == [0.0; 3] {
                return resolved;
            }
            magnitude = magnitude.max(dot(electric, electric).sqrt());
            for a in 0..3 {
                difference[a] += weight * electric[a];
            }
        }
        let tolerance = (1e-3 * dot(difference, difference).sqrt() / magnitude).clamp(1e-12, 1e-8);
        resolved.relative_tolerance = tolerance;
        resolved.position_tolerance_mm = tolerance;
        resolved.velocity_tolerance_mm_per_us = if tolerance > self.relative_tolerance {
            tolerance.max(tolerance * speed)
        } else {
            tolerance
        };
        resolved
    }

    pub(crate) fn validate(&self) -> Result<(), FlyError> {
        if !self.duration_us.is_finite()
            || self.duration_us < 0.0
            || [
                self.initial_step_us,
                self.min_step_us,
                self.max_step_us,
                self.position_tolerance_mm,
                self.velocity_tolerance_mm_per_us,
            ]
            .iter()
            .any(|x| !x.is_finite() || *x <= 0.0)
            || !self.relative_tolerance.is_finite()
            || self.relative_tolerance < 0.0
            || self.min_step_us > self.max_step_us
            || self.max_steps == 0
            || self.sample_every == Some(0)
            || [
                self.step_limits.max_grid_cells,
                self.step_limits.max_cyclotron_radians,
                self.step_limits.max_voltage_phase_radians,
            ]
            .iter()
            .any(|v| !v.is_finite() || *v <= 0.0)
            || self
                .stop_plane
                .is_some_and(|p| p.axis > 2 || !p.coordinate_mm.is_finite())
        {
            return Err(FlyError(
                "invalid flight limits, tolerances or sampling interval",
            ));
        }
        Ok(())
    }
}

/// Additional ceilings shared by independent and coupled flight, and GPU callers.
#[derive(Clone, Copy, Debug)]
pub struct StepLimits {
    pub max_grid_cells: f64,
    pub max_cyclotron_radians: f64,
    pub max_voltage_phase_radians: f64,
}

impl Default for StepLimits {
    fn default() -> Self {
        Self {
            max_grid_cells: 4.0,
            max_cyclotron_radians: 0.2,
            max_voltage_phase_radians: 0.2,
        }
    }
}

impl FlightOptions {
    /// Conservative local transit/rotation ceiling, in physical mm and us.
    /// The electric acceleration term also limits particles starting at rest.
    pub fn step_limit_us(
        &self,
        field: &impl Field,
        state: ParticleState,
        charge_to_mass: f64,
    ) -> f64 {
        if self.max_step_us != f64::MAX {
            return self.max_step_us;
        }
        let sample = field.sample(state.position_mm, state.time_us);
        self.step_limit_with_sample(
            state,
            charge_to_mass,
            sample,
            (field.spatial_scale_mm(), field.time_scale_us()),
        )
    }

    #[inline]
    fn step_limit_with_sample(
        &self,
        state: ParticleState,
        charge_to_mass: f64,
        sample: FieldSample,
        scales: (Option<f64>, Option<f64>),
    ) -> f64 {
        let speed = dot(state.velocity_mm_per_us, state.velocity_mm_per_us).sqrt();
        let gamma = if self.relativistic {
            1.0 / (1.0 - (speed / SPEED_OF_LIGHT).powi(2)).sqrt()
        } else {
            1.0
        };
        let acceleration = charge_to_mass.abs()
            * ELECTRIC_ACCELERATION
            * dot(sample.electric_v_per_mm, sample.electric_v_per_mm).sqrt()
            / gamma;
        let mut limit = self.max_step_us;
        if let Some(spacing) = scales.0 {
            let distance = self.step_limits.max_grid_cells * spacing;
            if acceleration == 0.0 {
                if speed > 0.0 {
                    limit = limit.min(distance / speed);
                }
            } else {
                // Stable positive root of speed*h + acceleration*h*h/2 = distance.
                let denominator = speed + speed.hypot((2.0 * acceleration * distance).sqrt());
                limit = limit.min(2.0 * distance / denominator);
            }
        }
        let omega = charge_to_mass.abs()
            * MAGNETIC_FREQUENCY
            * dot(sample.magnetic_gauss, sample.magnetic_gauss).sqrt()
            / gamma;
        if omega > 0.0 {
            limit = limit.min(self.step_limits.max_cyclotron_radians / omega);
        }
        if let Some(scale) = scales.1 {
            limit = limit.min(self.step_limits.max_voltage_phase_radians * scale);
        }
        limit
    }
}

#[derive(Clone, Debug)]
pub struct FlightResult {
    pub final_state: ParticleState,
    pub splat_reason: SplatReason,
    pub tof_us: f64,
    pub accepted_steps: usize,
    pub rejected_steps: usize,
    pub trajectory: Option<Vec<ParticleState>>,
}

pub fn fly(
    field: &impl Field,
    particle: &Particle,
    options: &FlightOptions,
) -> Result<FlightResult, FlyError> {
    options.validate()?;
    particle.validate(options.relativistic)?;
    Ok(fly_validated(field, particle, options, false))
}

/// Independent, deterministic per-particle integration; output preserves order.
pub fn fly_many(
    field: &impl Field,
    particles: &[Particle],
    options: &FlightOptions,
) -> Result<Vec<FlightResult>, FlyError> {
    options.validate()?;
    for particle in particles {
        particle.validate(options.relativistic)?;
    }
    Ok(particles
        .par_iter()
        .map(|p| fly_validated(field, p, options, false))
        .collect())
}

/// Explicit surface emission: ignore only the initial contiguous metal run.
/// Once the trajectory reaches vacuum, all subsequent electrode hits splat.
pub fn fly_many_from_electrode(
    field: &impl Field,
    particles: &[Particle],
    options: &FlightOptions,
) -> Result<Vec<FlightResult>, FlyError> {
    options.validate()?;
    for p in particles {
        p.validate(options.relativistic)?;
    }
    Ok(particles
        .par_iter()
        .map(|p| fly_validated(field, p, options, true))
        .collect())
}

#[derive(Clone, Copy, Default)]
struct Time {
    sum: f64,
    compensation: f64,
}

impl Time {
    fn add(&mut self, step: f64) {
        let corrected = step - self.compensation;
        let sum = self.sum + corrected;
        self.compensation = (sum - self.sum) - corrected;
        self.sum = sum;
    }
}

// The second triplet is proper velocity gamma*v when relativistic.
type State = [f64; 6];

#[derive(Default)]
struct RkCache {
    first: Option<(State, f64, State, FieldSample)>,
    last: Option<(State, f64, State, FieldSample)>,
    next_event: Option<f64>,
}

fn velocity(y: &State, relativistic: bool) -> Vector {
    let u = [y[3], y[4], y[5]];
    let gamma = if relativistic {
        (1.0 + dot(u, u) / SPEED_OF_LIGHT.powi(2)).sqrt()
    } else {
        1.0
    };
    u.map(|v| v / gamma)
}

fn position(y: &State) -> Vector {
    [y[0], y[1], y[2]]
}

fn derivative(sample: FieldSample, y: &State, ratio: f64, relativistic: bool) -> State {
    let v = velocity(y, relativistic);
    let magnetic = cross(v, sample.magnetic_gauss);
    [
        v[0],
        v[1],
        v[2],
        ratio
            * (ELECTRIC_ACCELERATION * sample.electric_v_per_mm[0]
                + MAGNETIC_FREQUENCY * magnetic[0]),
        ratio
            * (ELECTRIC_ACCELERATION * sample.electric_v_per_mm[1]
                + MAGNETIC_FREQUENCY * magnetic[1]),
        ratio
            * (ELECTRIC_ACCELERATION * sample.electric_v_per_mm[2]
                + MAGNETIC_FREQUENCY * magnetic[2]),
    ]
}

// Dormand-Prince 5(4), with the embedded velocity error measured in physical
// velocity, not proper velocity. Both absolute tolerances use SIMION units.
fn rk45(
    field: &impl Field,
    y: &State,
    (time, end_time, h): (f64, f64, f64),
    ratio: f64,
    options: &FlightOptions,
    cache: &mut RkCache,
    estimate_error: bool,
) -> (State, f64) {
    const C: [f64; 7] = [0.0, 1.0 / 5.0, 3.0 / 10.0, 4.0 / 5.0, 8.0 / 9.0, 1.0, 1.0];
    const A: [[f64; 6]; 7] = [
        [0.0; 6],
        [1.0 / 5.0, 0.0, 0.0, 0.0, 0.0, 0.0],
        [3.0 / 40.0, 9.0 / 40.0, 0.0, 0.0, 0.0, 0.0],
        [44.0 / 45.0, -56.0 / 15.0, 32.0 / 9.0, 0.0, 0.0, 0.0],
        [
            19372.0 / 6561.0,
            -25360.0 / 2187.0,
            64448.0 / 6561.0,
            -212.0 / 729.0,
            0.0,
            0.0,
        ],
        [
            9017.0 / 3168.0,
            -355.0 / 33.0,
            46732.0 / 5247.0,
            49.0 / 176.0,
            -5103.0 / 18656.0,
            0.0,
        ],
        [
            35.0 / 384.0,
            0.0,
            500.0 / 1113.0,
            125.0 / 192.0,
            -2187.0 / 6784.0,
            11.0 / 84.0,
        ],
    ];
    const B4: [f64; 7] = [
        5179.0 / 57600.0,
        0.0,
        7571.0 / 16695.0,
        393.0 / 640.0,
        -92097.0 / 339200.0,
        187.0 / 2100.0,
        1.0 / 40.0,
    ];
    let mut k = [[0.0; 6]; 7];
    k[0] = match cache.first {
        Some((_, _, first, _)) => first,
        None => {
            let sample = field.sample(position(y), time);
            let first = derivative(sample, y, ratio, options.relativistic);
            cache.first = Some((*y, time, first, sample));
            first
        }
    };
    if k[0].iter().any(|v| !v.is_finite()) {
        return (*y, f64::INFINITY);
    }
    let mut high = *y;
    for s in 1..7 {
        high = *y;
        for (j, previous) in k.iter().enumerate().take(s) {
            for a in 0..6 {
                high[a] += h * A[s][j] * previous[a];
            }
        }
        let mut stage_time = time + C[s] * h;
        if C[s] == 1.0 {
            stage_time = if let Some(edge) = cache
                .next_event
                .filter(|&edge| edge == stage_time || edge == end_time)
            {
                edge.next_down()
            } else {
                end_time
            };
        }
        let sample = field.sample(position(&high), stage_time);
        k[s] = derivative(sample, &high, ratio, options.relativistic);
        if k[s].iter().any(|v| !v.is_finite()) {
            return (high, f64::INFINITY);
        }
        if s == 6 {
            cache.last = Some((high, stage_time, k[s], sample));
        }
    }
    if !estimate_error {
        return (high, 0.0);
    }
    let mut low = *y;
    for (s, stage) in k.iter().enumerate() {
        for a in 0..6 {
            low[a] += h * B4[s] * stage[a];
        }
    }
    let mut error: f64 = 0.0;
    for a in 0..3 {
        let scale = options.position_tolerance_mm
            + options.relative_tolerance * y[a].abs().max(high[a].abs());
        error = error.max((high[a] - low[a]).abs() / scale);
    }
    let initial_v = [k[0][0], k[0][1], k[0][2]];
    let high_v = [k[6][0], k[6][1], k[6][2]];
    let low_v = velocity(&low, options.relativistic);
    for a in 0..3 {
        let scale = options.velocity_tolerance_mm_per_us
            + options.relative_tolerance * initial_v[a].abs().max(high_v[a].abs());
        error = error.max((high_v[a] - low_v[a]).abs() / scale);
    }
    (high, error)
}

fn boris(
    field: &impl Field,
    y: &State,
    time: f64,
    h: f64,
    ratio: f64,
    options: &FlightOptions,
) -> State {
    let v = velocity(y, options.relativistic);
    let mid = std::array::from_fn(|a| y[a] + 0.5 * h * v[a]);
    let sample = field.sample(mid, time + 0.5 * h);
    let impulse = sample
        .electric_v_per_mm
        .map(|e| 0.5 * h * ratio * ELECTRIC_ACCELERATION * e);
    let minus: Vector = std::array::from_fn(|a| y[a + 3] + impulse[a]);
    let gamma = if options.relativistic {
        (1.0 + dot(minus, minus) / SPEED_OF_LIGHT.powi(2)).sqrt()
    } else {
        1.0
    };
    let t = sample
        .magnetic_gauss
        .map(|b| 0.5 * h * ratio * MAGNETIC_FREQUENCY * b / gamma);
    let s = t.map(|v| 2.0 * v / (1.0 + dot(t, t)));
    let rotate = cross(minus, t);
    let prime = std::array::from_fn(|a| minus[a] + rotate[a]);
    let rotate = cross(prime, s);
    let mut next = *y;
    for a in 0..3 {
        next[a + 3] = minus[a] + rotate[a] + impulse[a];
    }
    let next_v = velocity(&next, options.relativistic);
    for a in 0..3 {
        next[a] = mid[a] + 0.5 * h * next_v[a];
    }
    next
}

fn event(
    field: &impl Field,
    from: Vector,
    to: Vector,
    options: &FlightOptions,
    launch: bool,
) -> Option<(f64, SplatReason)> {
    let grid = if launch {
        field.launch_intersection(from, to)
    } else {
        field.intersection(from, to)
    };
    let plane = options.stop_plane.and_then(|p| {
        let distance = to[p.axis] - from[p.axis];
        if distance == 0.0 {
            return None;
        }
        let fraction = (p.coordinate_mm - from[p.axis]) / distance;
        (0.0..=1.0)
            .contains(&fraction)
            .then_some((fraction, SplatReason::StopPlane))
    });
    match (grid, plane) {
        (Some(a), Some(b)) => Some(if a.0 <= b.0 { a } else { b }),
        (a, b) => a.or(b),
    }
}

fn fly_validated(
    field: &impl Field,
    particle: &Particle,
    options: &FlightOptions,
    launch: bool,
) -> FlightResult {
    fly_physics(field, particle, options, launch, None)
}

pub(crate) fn fly_with_process(
    field: &impl Field,
    particle: &Particle,
    options: &FlightOptions,
    process: &mut crate::physics::collision::Process<'_>,
) -> FlightResult {
    fly_physics(field, particle, options, false, Some(process))
}

fn fly_physics(
    field: &impl Field,
    particle: &Particle,
    options: &FlightOptions,
    launch: bool,
    mut process: Option<&mut crate::physics::collision::Process<'_>>,
) -> FlightResult {
    let resolved = options.with_field_tolerance(field, particle.initial);
    let options = &resolved;
    let v = particle.initial.velocity_mm_per_us;
    let gamma = if options.relativistic {
        1.0 / (1.0 - dot(v, v) / SPEED_OF_LIGHT.powi(2)).sqrt()
    } else {
        1.0
    };
    let p = particle.initial.position_mm;
    let mut y = [p[0], p[1], p[2], gamma * v[0], gamma * v[1], gamma * v[2]];
    let ratio = particle.charge_e / particle.mass_amu;
    let mut time = Time::default();
    let mut h = options
        .initial_step_us
        .clamp(options.min_step_us, options.max_step_us);
    let mut accepted = 0;
    let mut rejected = 0;
    let mut cache = RkCache {
        next_event: field.next_time_event(particle.initial.time_us),
        ..Default::default()
    };
    let scales = if options.max_step_us == f64::MAX {
        (field.spatial_scale_mm(), field.time_scale_us())
    } else {
        (None, None)
    };
    let mut trajectory = options.sample_every.map(|_| vec![particle.initial]);
    let initial = field.status(p);
    let mut departing = launch
        && matches!(
            initial,
            Some(SplatReason::Electrode { .. } | SplatReason::SurfaceElectrode { .. })
        );
    let mut reason = initial.filter(|_| !departing).or_else(|| {
        options
            .stop_plane
            .filter(|plane| p[plane.axis] == plane.coordinate_mm)
            .map(|_| SplatReason::StopPlane)
    });
    while reason.is_none() && time.sum < options.duration_us {
        if accepted + rejected >= options.max_steps {
            reason = Some(SplatReason::StepLimit);
            break;
        }
        h = h.min(options.duration_us - time.sum);
        if let Some(p) = &process {
            h = h.min(p.limit_step(&y));
        }
        if time.sum + h == time.sum {
            reason = Some(SplatReason::StepUnderflow);
            break;
        }
        let absolute_time = particle.initial.time_us + time.sum;
        if cache.next_event.is_some_and(|edge| edge <= absolute_time) {
            cache.next_event = field.next_time_event(absolute_time);
        }
        if cache
            .first
            .is_some_and(|(state, t, _, _)| state != y || t != absolute_time)
        {
            cache.first = None;
        }
        if options.max_step_us == f64::MAX {
            let sample = match cache.first {
                Some((_, _, _, sample)) => sample,
                None => {
                    let sample = field.sample(position(&y), absolute_time);
                    let first = derivative(sample, &y, ratio, options.relativistic);
                    cache.first = Some((y, absolute_time, first, sample));
                    sample
                }
            };
            h = h.min(options.step_limit_with_sample(
                ParticleState {
                    position_mm: position(&y),
                    velocity_mm_per_us: velocity(&y, options.relativistic),
                    time_us: absolute_time,
                },
                ratio,
                sample,
                scales,
            ));
        }
        if let Some(edge) = cache.next_event {
            h = h.min(edge - absolute_time);
        }
        // Rejected trials and event probes share their initial derivative.
        // The final RK stage is FSAL, but its state and time must match exactly:
        // collisions can change velocity and voltage edges sample the left side.
        let mut trial = |h, estimate_error| {
            process
                .as_deref()
                .and_then(|p| p.trial(field, &y, absolute_time, h, ratio, options))
                .unwrap_or_else(|| match options.integrator {
                    Integrator::Rk45 => {
                        let mut endpoint = time;
                        endpoint.add(h);
                        rk45(
                            field,
                            &y,
                            (absolute_time, particle.initial.time_us + endpoint.sum, h),
                            ratio,
                            options,
                            &mut cache,
                            estimate_error,
                        )
                    }
                    Integrator::Boris => (boris(field, &y, absolute_time, h, ratio, options), 0.0),
                })
        };
        let (mut next, error) = trial(h, true);
        if next.iter().any(|v| !v.is_finite()) || !error.is_finite() {
            reason = Some(SplatReason::NonFinite);
            break;
        }
        if error > 1.0 {
            rejected += 1;
            let reduced = h * (0.9 * error.powf(-0.2)).clamp(0.1, 0.5);
            if reduced < options.min_step_us {
                reason = Some(SplatReason::StepUnderflow);
                break;
            }
            h = reduced;
            continue;
        }
        if let Some((_, mut hit)) = event(field, position(&y), position(&next), options, departing)
        {
            // Re-integrate to the event, rather than linearly interpolating a
            // curved trajectory's position, velocity or time of flight.
            let mut low = 0.0;
            let mut high = h;
            let mut high_state = next;
            let initial_v = velocity(&y, options.relativistic);
            let final_v = velocity(&next, options.relativistic);
            let chord: Vector = std::array::from_fn(|a| (next[a] - y[a]) / h);
            let speed = dot(initial_v, initial_v)
                .max(dot(final_v, final_v))
                .max(dot(chord, chord))
                .sqrt();
            // Do not resolve the event more finely than the requested absolute
            // position budget. The chord speed also covers turning endpoints.
            let event_time_tolerance = options.position_tolerance_mm / speed;
            let mut low_position = position(&y);
            let mut high_position = position(&next);
            let mut previous = h;
            let mut change = h;
            // Chord intersections propose times, never accepted event states.
            // Every bracket update is verified by re-integrating from y.
            for _ in 0..8 {
                if high - low <= event_time_tolerance {
                    break;
                }
                let Some((fraction, _)) =
                    event(field, low_position, high_position, options, departing)
                else {
                    break;
                };
                let probe_time = low + fraction * (high - low);
                if probe_time <= low || probe_time >= high {
                    previous = probe_time;
                    change = 0.0;
                    break;
                }
                change = (probe_time - previous).abs();
                previous = probe_time;
                let (probe, _) = trial(probe_time, false);
                if let Some((_, localized_hit)) =
                    event(field, position(&y), position(&probe), options, departing)
                {
                    high = probe_time;
                    high_state = probe;
                    high_position = position(&probe);
                    hit = localized_hit;
                } else {
                    low = probe_time;
                    low_position = position(&probe);
                }
                if change <= event_time_tolerance {
                    break;
                }
            }
            // Try the other side of the latest estimate before falling back
            // to bisection. A poor estimate cannot invalidate the bracket.
            let margin = (2.0 * change).max(event_time_tolerance);
            let probe_time = if previous == high {
                high - margin
            } else {
                low + margin
            };
            if probe_time > low && probe_time < high {
                let (probe, _) = trial(probe_time, false);
                if let Some((_, localized_hit)) =
                    event(field, position(&y), position(&probe), options, departing)
                {
                    high = probe_time;
                    high_state = probe;
                    hit = localized_hit;
                } else {
                    low = probe_time;
                }
            }
            for _ in 0..48 {
                if high - low <= event_time_tolerance {
                    break;
                }
                let mid = 0.5 * (low + high);
                let (probe, _) = trial(mid, false);
                if let Some((_, localized_hit)) =
                    event(field, position(&y), position(&probe), options, departing)
                {
                    high = mid;
                    high_state = probe;
                    hit = localized_hit;
                } else {
                    low = mid;
                }
            }
            h = high;
            next = high_state;
            reason = Some(hit);
        }
        if reason.is_none()
            && let Some(p) = &mut process
        {
            let before = position(&next);
            p.accepted(&y, &mut next, h);
            // An SDS jump can cross a thin electrode even if its endpoint
            // is in vacuum. Diffusion consumes the full accepted time step.
            if let Some((fraction, hit)) = event(field, before, position(&next), options, departing)
            {
                for a in 0..3 {
                    next[a] = before[a] + fraction * (next[a] - before[a]);
                }
                reason = Some(hit);
            }
        }
        time.add(h);
        y = next;
        cache.first = cache.last.take();
        if departing
            && !matches!(
                field.status(position(&y)),
                Some(SplatReason::Electrode { .. } | SplatReason::SurfaceElectrode { .. })
            )
        {
            departing = false;
        }
        accepted += 1;
        if options.sample_every.is_some_and(|n| accepted % n == 0)
            && let Some(samples) = &mut trajectory
        {
            samples.push(ParticleState {
                position_mm: position(&y),
                velocity_mm_per_us: velocity(&y, options.relativistic),
                time_us: particle.initial.time_us + time.sum,
            });
        }
        if options.integrator == Integrator::Rk45 {
            let factor = if error == 0.0 {
                5.0
            } else {
                (0.9 * error.powf(-0.2)).clamp(0.2, 5.0)
            };
            h = (h * factor).clamp(options.min_step_us, options.max_step_us);
        } else {
            // A voltage edge may shorten this step, not Boris's later fixed steps.
            h = options
                .initial_step_us
                .clamp(options.min_step_us, options.max_step_us);
        }
    }
    let final_state = ParticleState {
        position_mm: position(&y),
        velocity_mm_per_us: velocity(&y, options.relativistic),
        time_us: particle.initial.time_us + time.sum,
    };
    if let Some(samples) = &mut trajectory
        && samples.last() != Some(&final_state)
    {
        samples.push(final_state);
    }
    FlightResult {
        final_state,
        splat_reason: reason.unwrap_or(SplatReason::TimeLimit),
        tof_us: time.sum,
        accepted_steps: accepted,
        rejected_steps: rejected,
        trajectory,
    }
}
