//! Synchronized, second-order drift/Boris-kick/drift ensemble integration.
use std::time::{Duration, Instant};

use super::space_charge::{ChargeMode, SpaceCharge};
use crate::{
    ELECTRIC_ACCELERATION, Field, FieldSample, FlightOptions, FlightResult, FlyError,
    MAGNETIC_FREQUENCY, Particle, ParticleState, SPEED_OF_LIGHT, SplatReason, Vector, cross, dot,
};

#[derive(Clone, Debug)]
pub struct EnsembleOptions {
    pub flight: FlightOptions,
    pub charge_mode: ChargeMode,
    /// Empty means one for every particle. Source weights do not alter q/m.
    pub charge_weights: Vec<f64>,
    /// Fraction of the local pair/plasma/cell-transit time, in (0,1].
    pub space_charge_step_fraction: f64,
}

impl Default for EnsembleOptions {
    fn default() -> Self {
        Self {
            flight: FlightOptions::default(),
            charge_mode: ChargeMode::Factor(1.0),
            charge_weights: Vec::new(),
            space_charge_step_fraction: 0.1,
        }
    }
}

pub struct EnsembleReport {
    pub results: Vec<FlightResult>,
    pub global_steps: usize,
    pub rejected_steps: usize,
    /// Integration, deposition, field sums and per-step Poisson solves only.
    /// Mesh construction, input parsing, external field reconstruction and output are excluded.
    pub compute_time: Duration,
}

pub(super) fn boris_velocity(
    particle: &Particle,
    v: Vector,
    sample: FieldSample,
    h: f64,
    relativistic: bool,
) -> Vector {
    let ratio = particle.charge_e / particle.mass_amu;
    let gamma = if relativistic {
        1.0 / (1.0 - dot(v, v) / SPEED_OF_LIGHT.powi(2)).sqrt()
    } else {
        1.0
    };
    let impulse = sample
        .electric_v_per_mm
        .map(|e| 0.5 * h * ratio * ELECTRIC_ACCELERATION * e);
    let minus: Vector = std::array::from_fn(|a| gamma * v[a] + impulse[a]);
    let gamma_minus = if relativistic {
        (1.0 + dot(minus, minus) / SPEED_OF_LIGHT.powi(2)).sqrt()
    } else {
        1.0
    };
    let t = sample
        .magnetic_gauss
        .map(|b| 0.5 * h * ratio * MAGNETIC_FREQUENCY * b / gamma_minus);
    let s = t.map(|b| 2.0 * b / (1.0 + dot(t, t)));
    let rotation = cross(minus, t);
    let prime = std::array::from_fn(|a| minus[a] + rotation[a]);
    let rotation = cross(prime, s);
    let u = std::array::from_fn(|a| minus[a] + rotation[a] + impulse[a]);
    let next_gamma = if relativistic {
        (1.0 + dot(u, u) / SPEED_OF_LIGHT.powi(2)).sqrt()
    } else {
        1.0
    };
    u.map(|a| a / next_gamma)
}

pub(super) fn push(
    field: &impl Field,
    particle: &Particle,
    old: ParticleState,
    h: f64,
    charge_e: Vector,
    relativistic: bool,
) -> ParticleState {
    let mid = std::array::from_fn(|a| old.position_mm[a] + 0.5 * h * old.velocity_mm_per_us[a]);
    let mut sample = field.sample(mid, old.time_us + 0.5 * h);
    for (a, &e) in charge_e.iter().enumerate() {
        sample.electric_v_per_mm[a] += e;
    }
    let v = boris_velocity(particle, old.velocity_mm_per_us, sample, h, relativistic);
    ParticleState {
        position_mm: std::array::from_fn(|a| mid[a] + 0.5 * h * v[a]),
        velocity_mm_per_us: v,
        time_us: old.time_us + h,
    }
}

pub(super) fn external_error(
    field: &impl Field,
    particle: &Particle,
    state: ParticleState,
    h: f64,
    o: &FlightOptions,
) -> f64 {
    let whole = push(field, particle, state, h, [0.0; 3], o.relativistic);
    let half = push(field, particle, state, 0.5 * h, [0.0; 3], o.relativistic);
    let halves = push(field, particle, half, 0.5 * h, [0.0; 3], o.relativistic);
    let mut error = 0.0_f64;
    for a in 0..3 {
        let scale = o.position_tolerance_mm
            + o.relative_tolerance * state.position_mm[a].abs().max(halves.position_mm[a].abs());
        error = error.max((whole.position_mm[a] - halves.position_mm[a]).abs() / (3.0 * scale));
        let scale = o.velocity_tolerance_mm_per_us
            + o.relative_tolerance
                * state.velocity_mm_per_us[a]
                    .abs()
                    .max(halves.velocity_mm_per_us[a].abs());
        error = error.max(
            (whole.velocity_mm_per_us[a] - halves.velocity_mm_per_us[a]).abs() / (3.0 * scale),
        );
    }
    error
}

pub(super) fn crossing(
    field: &impl Field,
    from: Vector,
    to: Vector,
    options: &FlightOptions,
) -> Option<(f64, SplatReason)> {
    let grid = field.intersection(from, to);
    let plane = options.stop_plane.and_then(|p| {
        let delta = to[p.axis] - from[p.axis];
        if delta == 0.0 {
            return None;
        }
        let f = (p.coordinate_mm - from[p.axis]) / delta;
        (0.0..=1.0)
            .contains(&f)
            .then_some((f, SplatReason::StopPlane))
    });
    grid.into_iter()
        .chain(plane)
        .min_by(|a, b| a.0.total_cmp(&b.0))
}

/// Coupled ensemble, preserving input order. Existing independent fly APIs are unchanged.
///
/// Positions and physical velocities are collocated at global endpoints. Drift
/// predicts ALL live positions at t+dt/2; SC and external fields are evaluated
/// there, then a Boris kick and second half drift complete the step. Births
/// split the global interval exactly, so their first push is only the remaining
/// part of that interval: no pre-birth source, unequal-time force or retroactive
/// force. After a push, first segment crossings are localized exactly as
/// fractions of that segment and those sources are absent from the next step.
pub fn fly_ensemble(
    field: &impl Field,
    particles: &[Particle],
    options: &EnsembleOptions,
    space_charge: &mut SpaceCharge,
) -> Result<EnsembleReport, FlyError> {
    let o = &options.flight;
    o.validate()?;
    // The coupled push is second order and does not embed the Coulomb force in
    // RK45's error estimate. Retain its one-cell ceiling independently of the
    // looser independent-flight transit budget.
    let mut step_control = o.clone();
    step_control.step_limits.max_grid_cells = step_control.step_limits.max_grid_cells.min(1.0);
    if !options.space_charge_step_fraction.is_finite()
        || options.space_charge_step_fraction <= 0.0
        || options.space_charge_step_fraction > 1.0
    {
        return Err(FlyError("space-charge step fraction must lie in (0,1]"));
    }
    for particle in particles {
        particle.validate(o.relativistic)?;
    }
    let charges = options
        .charge_mode
        .charges(particles, &options.charge_weights)?;
    let mut states: Vec<_> = particles.iter().map(|p| p.initial).collect();
    let mut results: Vec<_> = particles
        .iter()
        .map(|p| FlightResult {
            final_state: p.initial,
            splat_reason: SplatReason::TimeLimit,
            tof_us: 0.0,
            accepted_steps: 0,
            rejected_steps: 0,
            trajectory: o.sample_every.map(|_| vec![p.initial]),
        })
        .collect();
    let mut finished = vec![false; particles.len()];
    let mut order: Vec<_> = (0..particles.len()).collect();
    order.sort_by(|&a, &b| {
        states[a]
            .time_us
            .total_cmp(&states[b].time_us)
            .then(a.cmp(&b))
    });
    let mut pending = 0;
    let mut live = Vec::new();
    let mut time = order.first().map_or(0.0, |&i| states[i].time_us);
    let mut step = o.initial_step_us.clamp(o.min_step_us, o.max_step_us);
    let mut global_steps = 0;
    let mut rejected_steps = 0;
    // Runtime worker creation is preparation, not particle computation.
    let _ = rayon::current_num_threads();
    let start = Instant::now();
    while pending < order.len() || !live.is_empty() {
        // Insert births before the next force evaluation, at their exact time.
        while pending < order.len() && particles[order[pending]].initial.time_us <= time {
            let i = order[pending];
            pending += 1;
            let initial_hit = field.status(states[i].position_mm).or_else(|| {
                o.stop_plane
                    .filter(|p| states[i].position_mm[p.axis] == p.coordinate_mm)
                    .map(|_| SplatReason::StopPlane)
            });
            if let Some(hit) = initial_hit {
                results[i].splat_reason = hit;
                finished[i] = true;
            } else if o.duration_us == 0.0 {
                finished[i] = true;
            } else {
                live.push(i);
            }
        }
        if live.is_empty() {
            if pending < order.len() {
                time = particles[order[pending]].initial.time_us;
                continue;
            }
            break;
        }
        if global_steps + rejected_steps >= o.max_steps {
            break;
        }
        let mut h = step.min(o.max_step_us);
        for &i in &live {
            h = h.min(particles[i].initial.time_us + o.duration_us - time);
            h = h.min(step_control.step_limit_us(
                field,
                states[i],
                particles[i].charge_e / particles[i].mass_amu,
            ));
        }
        if pending < order.len() {
            h = h.min(particles[order[pending]].initial.time_us - time);
        }
        if let Some(edge) = field.next_time_event(time) {
            h = h.min(edge - time);
        }
        // Keep the force-time drift inside absorbing geometry. Acceleration
        // can still change the final crossing; that is handled after the push.
        if matches!(space_charge, SpaceCharge::Pic(_) | SpaceCharge::GpuPic(_)) {
            for &i in &live {
                let old = states[i];
                let drift =
                    std::array::from_fn(|a| old.position_mm[a] + h * old.velocity_mm_per_us[a]);
                if let Some((f, _)) = crossing(field, old.position_mm, drift, o)
                    && f > 0.0
                {
                    h *= f;
                }
            }
        }
        let positions: Vec<_> = live.iter().map(|&i| states[i].position_mm).collect();
        let velocities: Vec<_> = live.iter().map(|&i| states[i].velocity_mm_per_us).collect();
        let source: Vec<_> = live.iter().map(|&i| charges[i]).collect();
        let ratios: Vec<_> = live
            .iter()
            .map(|&i| particles[i].charge_e / particles[i].mass_amu)
            .collect();
        if let SpaceCharge::Direct(direct) = space_charge {
            let limit = options.space_charge_step_fraction
                * direct.time_scale(&positions, &velocities, &source, &ratios);
            if limit < o.min_step_us {
                return Err(FlyError(
                    "direct closest-approach criterion needs a step below min_step_us",
                ));
            }
            h = h.min(limit);
        }
        if h <= 0.0 || time + h == time {
            return Err(FlyError("ensemble global time step underflow"));
        }
        let error = live
            .iter()
            .map(|&i| external_error(field, &particles[i], states[i], h, o))
            .fold(0.0, f64::max);
        if !error.is_finite() {
            return Err(FlyError("non-finite external-field error estimate"));
        }
        if error > 1.0 {
            rejected_steps += 1;
            step = h * (0.8 * error.powf(-1.0 / 3.0)).clamp(0.1, 0.5);
            if step < o.min_step_us {
                return Err(FlyError(
                    "ensemble external-field tolerance needs a step below min_step_us",
                ));
            }
            continue;
        }
        // Force time is common to every live source and target.
        let midpoint: Vec<Vector> = positions
            .iter()
            .zip(&velocities)
            .map(|(p, v)| std::array::from_fn(|a| p[a] + 0.5 * h * v[a]))
            .collect();
        let fields = match space_charge {
            SpaceCharge::Direct(direct) => direct.field(&midpoint, &source)?,
            SpaceCharge::Pic(pic) => {
                let fields = pic.field(&midpoint, &source, &ratios)?;
                let limit = options.space_charge_step_fraction * pic.time_scale(&velocities);
                if h > limit {
                    step = limit;
                    rejected_steps += 1;
                    if step < o.min_step_us {
                        return Err(FlyError(
                            "PIC plasma/cell criterion needs a step below min_step_us",
                        ));
                    }
                    continue;
                }
                fields
            }
            SpaceCharge::GpuPic(pic) => {
                pic.evaluate(&midpoint, &source, &ratios)?;
                let limit = options.space_charge_step_fraction * pic.time_scale(&velocities);
                if h > limit {
                    step = limit;
                    rejected_steps += 1;
                    if step < o.min_step_us {
                        return Err(FlyError("GPU PIC criterion needs a step below min_step_us"));
                    }
                    continue;
                }
                Vec::new()
            }
        };
        let end = time + h;
        let next_states: Vec<_> = if let SpaceCharge::GpuPic(pic) = space_charge {
            pic.push(field, particles, &states, &live, h, o.relativistic)?
        } else {
            live.iter()
                .zip(&fields)
                .map(|(&i, &e)| push(field, &particles[i], states[i], h, e, o.relativistic))
                .collect()
        };
        let first_crossing = live
            .iter()
            .zip(&next_states)
            .filter_map(|(&i, next)| {
                crossing(field, states[i].position_mm, next.position_mm, o).map(|event| event.0)
            })
            .fold(1.0, f64::min);
        // Re-evaluate every force on a shortened GLOBAL interval, not just the
        // exiting particle. No survivor receives a post-splat source impulse.
        if first_crossing == 0.0 || time + h * first_crossing == time {
            for (&i, next) in live.iter().zip(&next_states) {
                if let Some((fraction, reason)) =
                    crossing(field, states[i].position_mm, next.position_mm, o)
                    && fraction == first_crossing
                {
                    let old = states[i];
                    states[i] = ParticleState {
                        position_mm: std::array::from_fn(|a| {
                            old.position_mm[a]
                                + fraction * (next.position_mm[a] - old.position_mm[a])
                        }),
                        velocity_mm_per_us: std::array::from_fn(|a| {
                            old.velocity_mm_per_us[a]
                                + fraction
                                    * (next.velocity_mm_per_us[a] - old.velocity_mm_per_us[a])
                        }),
                        time_us: time,
                    };
                    finished[i] = true;
                    results[i].splat_reason = reason;
                }
            }
            live.retain(|&i| !finished[i]);
            continue;
        }
        if first_crossing < 1.0 - 1e-9 {
            step = h * first_crossing;
            rejected_steps += 1;
            continue;
        }
        for (&i, &candidate) in live.iter().zip(&next_states) {
            let old = states[i];
            let mut next = candidate;
            if next
                .position_mm
                .iter()
                .chain(&next.velocity_mm_per_us)
                .any(|v| !v.is_finite())
            {
                return Err(FlyError("non-finite ensemble state"));
            }
            if let Some((f, reason)) = crossing(field, old.position_mm, next.position_mm, o) {
                next = ParticleState {
                    position_mm: std::array::from_fn(|a| {
                        old.position_mm[a] + f * (next.position_mm[a] - old.position_mm[a])
                    }),
                    velocity_mm_per_us: std::array::from_fn(|a| {
                        old.velocity_mm_per_us[a]
                            + f * (next.velocity_mm_per_us[a] - old.velocity_mm_per_us[a])
                    }),
                    time_us: time + f * h,
                };
                finished[i] = true;
                results[i].splat_reason = reason;
            } else {
                next.time_us = end;
                if end >= particles[i].initial.time_us + o.duration_us {
                    finished[i] = true;
                }
            }
            states[i] = next;
            results[i].accepted_steps += 1;
            if o.sample_every
                .is_some_and(|n| results[i].accepted_steps % n == 0)
                && let Some(samples) = &mut results[i].trajectory
            {
                samples.push(next);
            }
        }
        live.retain(|&i| !finished[i]);
        global_steps += 1;
        time = end;
        step = (h * if error == 0.0 {
            2.0
        } else {
            (0.8 * error.powf(-1.0 / 3.0)).clamp(0.5, 2.0)
        })
        .clamp(o.min_step_us, o.max_step_us);
    }
    for (i, result) in results.iter_mut().enumerate() {
        result.final_state = states[i];
        result.tof_us = states[i].time_us - particles[i].initial.time_us;
        result.rejected_steps = rejected_steps;
        if !finished[i] {
            result.splat_reason = SplatReason::StepLimit;
        }
        if let Some(samples) = &mut result.trajectory
            && samples.last() != Some(&states[i])
        {
            samples.push(states[i]);
        }
    }
    Ok(EnsembleReport {
        results,
        global_steps,
        rejected_steps,
        compute_time: start.elapsed(),
    })
}
