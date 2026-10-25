//! SIMION-documented beam semantics: one pass through leader-normal planes.
//!
//! This is not common-time bunch integration or an iterative steady-state
//! solution. The line kernel is the standard infinite, parallel line-charge
//! field; SIMION's private cloud-overlap kernel is not reproduced.
use std::time::Instant;

use rayon::prelude::*;

use super::{
    ensemble::{EnsembleReport, boris_velocity, crossing, external_error, push},
    space_charge::{COULOMB_V_MM_PER_C, ChargeMode},
};
use crate::{
    Field, FieldSample, FlightOptions, FlightResult, FlyError, Particle, ParticleState,
    SplatReason, Vector, dot,
};

#[derive(Clone, Copy, Debug, Default)]
pub enum BeamInteraction {
    /// Parallel line currents normal to the leader plane, with no axial force.
    #[default]
    Lines,
    /// Azimuthally averaged parallel lines (coaxial cylindrical shells).
    /// The symmetry axis follows the leader; this assumes an axisymmetric
    /// cross-section, not an arbitrary bent/off-axis charge distribution.
    Rings,
}

#[derive(Clone, Debug)]
pub struct BeamOptions {
    pub flight: FlightOptions,
    /// Total absolute current in amperes. Physical charge supplies its sign.
    pub current_a: f64,
    /// Fixed CWFs; omitted weights are one, and lost current is not reassigned.
    pub charge_weights: Vec<f64>,
    pub interaction: BeamInteraction,
    /// Explicit line-cloud radius, never an inferred SIMION cloud diameter.
    pub softening_mm: f64,
    pub space_charge_step_fraction: f64,
}

impl Default for BeamOptions {
    fn default() -> Self {
        Self {
            flight: FlightOptions::default(),
            current_a: 0.0,
            charge_weights: Vec::new(),
            interaction: BeamInteraction::Lines,
            softening_mm: 0.0,
            space_charge_step_fraction: 1.0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BeamTermination {
    Flight(SplatReason),
    /// No forward crossing of the next leader-normal plane (>90 degrees).
    Diverged,
    /// All followers end when particle #1 ends, at their own flight times.
    LeaderStopped,
}

pub struct BeamReport {
    pub ensemble: EnsembleReport,
    /// Distinguishes beam kills from actual geometry hits without changing the
    /// terminal-event contract of independent flight.
    pub terminations: Vec<BeamTermination>,
}

fn normal(state: ParticleState) -> Result<Vector, FlyError> {
    let speed = dot(state.velocity_mm_per_us, state.velocity_mm_per_us).sqrt();
    if !speed.is_finite() || speed == 0.0 {
        return Err(FlyError(
            "beam leader must have nonzero finite forward velocity",
        ));
    }
    Ok(state.velocity_mm_per_us.map(|v| v / speed))
}

/// Free-space transverse E in V/mm. Currents are signed amperes and line
/// density is I/(v.n)*1e-6 C/mm. Self lines are omitted; a ring's self field
/// is half its discontinuity (the symmetric thin-shell limit).
pub fn beam_field(
    positions: &[Vector],
    velocities: &[Vector],
    currents: &[f64],
    leader: ParticleState,
    interaction: BeamInteraction,
    softening_mm: f64,
) -> Result<Vec<Vector>, FlyError> {
    beam_field_and_time_scale(
        positions,
        velocities,
        currents,
        leader,
        (interaction, softening_mm),
        &[],
        false,
    )
    .map(|(field, _)| field)
}

fn beam_field_and_time_scale(
    positions: &[Vector],
    velocities: &[Vector],
    currents: &[f64],
    leader: ParticleState,
    kernel: (BeamInteraction, f64),
    ratios: &[f64],
    limit_step: bool,
) -> Result<(Vec<Vector>, f64), FlyError> {
    let (interaction, softening_mm) = kernel;
    if positions.len() != velocities.len()
        || positions.len() != currents.len()
        || !softening_mm.is_finite()
        || softening_mm < 0.0
    {
        return Err(FlyError("invalid beam field inputs"));
    }
    let n = normal(leader)?;
    let leader_speed = dot(leader.velocity_mm_per_us, n);
    let mut density = Vec::with_capacity(currents.len());
    let mut clocks = Vec::with_capacity(currents.len());
    for (&v, &current) in velocities.iter().zip(currents) {
        let speed = dot(v, n);
        if speed <= 0.0 || !speed.is_finite() || !current.is_finite() {
            return Err(FlyError(
                "beam source does not cross the leader plane forward",
            ));
        }
        density.push(current * 1e-6 / speed);
        clocks.push(leader_speed / speed);
    }
    let transverse = |p: Vector| {
        let r = std::array::from_fn(|a| p[a] - leader.position_mm[a]);
        let along = dot(r, n);
        std::array::from_fn(|a| r[a] - along * n[a])
    };
    let radii: Vec<Vector> = positions.iter().map(|&p| transverse(p)).collect();
    let target = |(i, _): (usize, &Vector)| {
        if !ratios.is_empty() && ratios[i] == 0.0 {
            return ([0.0; 3], f64::INFINITY);
        }
        let mut e = [0.0; 3];
        let mut limit = f64::INFINITY;
        let mut radius = f64::INFINITY;
        let strength = if ratios.is_empty() || !limit_step {
            0.0
        } else {
            crate::ELECTRIC_ACCELERATION * ratios[i].abs() * 2.0 * COULOMB_V_MM_PER_C
        };
        for (j, &lambda) in density.iter().enumerate() {
            if lambda == 0.0 {
                continue;
            }
            let (r, enclosed) = match interaction {
                BeamInteraction::Lines => {
                    if i == j {
                        continue;
                    }
                    (std::array::from_fn(|a| radii[i][a] - radii[j][a]), 1.0)
                }
                BeamInteraction::Rings => {
                    let ri = dot(radii[i], radii[i]);
                    let rj = dot(radii[j], radii[j]);
                    if ri == 0.0 || ri < rj {
                        continue;
                    }
                    (radii[i], if ri == rj { 0.5 } else { 1.0 })
                }
            };
            let r2 = dot(r, r) + softening_mm.powi(2);
            let factor = 2.0 * COULOMB_V_MM_PER_C * lambda * enclosed / r2;
            for a in 0..3 {
                e[a] += factor * r[a];
            }
            if matches!(interaction, BeamInteraction::Lines) && strength > 0.0 {
                radius = radius.min(r2.sqrt());
                // Resolve encounters even when signed source fields cancel.
                // The drift rates use leader time, not equal follower TOFs.
                let dv = std::array::from_fn(|a| {
                    velocities[i][a] * clocks[i] - velocities[j][a] * clocks[j]
                });
                let along = dot(dv, n);
                let dv = std::array::from_fn(|a| dv[a] - along * n[a]);
                let speed = dot(dv, dv).sqrt();
                if speed > 0.0 {
                    limit = limit.min(r2.sqrt() / speed);
                }
                limit = limit.min((r2 / (strength * lambda.abs())).sqrt() / clocks[i]);
            }
        }
        if matches!(interaction, BeamInteraction::Lines) && strength > 0.0 {
            let acceleration = crate::ELECTRIC_ACCELERATION * ratios[i].abs() * dot(e, e).sqrt();
            limit = limit.min((radius / acceleration).sqrt() / clocks[i]);
        }
        (e, limit)
    };
    let values: Vec<_> = if positions.len() < if ratios.is_empty() { 256 } else { 64 } {
        positions.iter().enumerate().map(target).collect()
    } else {
        positions.par_iter().enumerate().map(target).collect()
    };
    let mut fields = Vec::with_capacity(values.len());
    let mut limit = f64::INFINITY;
    for (e, h) in values {
        fields.push(e);
        limit = limit.min(h);
    }
    if fields.iter().flatten().any(|v| !v.is_finite()) {
        return Err(FlyError(
            "singular beam line field; resolve coincident lines or specify softening",
        ));
    }
    Ok((fields, limit))
}

// With the field frozen at the midpoint, find each particle's *own* forward
// flight time to the new plane. Newton's derivative is the plane-normal
// velocity. The bounded solve fails explicitly rather than projecting space
// without advancing that particle's velocity and clock.
fn to_plane(
    field: &impl Field,
    particle: &Particle,
    old: ParticleState,
    plane: ParticleState,
    electric: Vector,
    guess: f64,
    options: &FlightOptions,
) -> Result<Option<ParticleState>, FlyError> {
    let n = normal(plane)?;
    let distance = dot(
        std::array::from_fn(|a| plane.position_mm[a] - old.position_mm[a]),
        n,
    );
    if distance < -options.position_tolerance_mm {
        return Ok(None);
    }
    let mut h = guess.max(0.0);
    let plane_roundoff = 64.0
        * f64::EPSILON
        * plane
            .position_mm
            .iter()
            .chain(&old.position_mm)
            .map(|v| v.abs())
            .fold(1.0, f64::max);
    for _ in 0..24 {
        let next = push(field, particle, old, h, electric, options.relativistic);
        let miss = dot(
            std::array::from_fn(|a| plane.position_mm[a] - next.position_mm[a]),
            n,
        );
        let speed = dot(next.velocity_mm_per_us, n);
        if !speed.is_finite() || speed <= 0.0 {
            return Ok(None);
        }
        if miss.abs() <= plane_roundoff {
            return Ok(Some(next));
        }
        h += miss / speed;
        if !h.is_finite() || h < 0.0 {
            return Ok(None);
        }
    }
    Err(FlyError("beam follower plane crossing did not converge"))
}

/// Fly through transverse planes defined by particle #1. TOBs offset reported
/// times only. Static external fields are required; different followers have
/// different flight times. See `docs/physics-space-charge.md` for assumptions.
pub fn fly_beam(
    field: &impl Field,
    particles: &[Particle],
    options: &BeamOptions,
) -> Result<BeamReport, FlyError> {
    let o = &options.flight;
    o.validate()?;
    if !options.softening_mm.is_finite()
        || options.softening_mm < 0.0
        || !options.space_charge_step_fraction.is_finite()
        || options.space_charge_step_fraction <= 0.0
        || options.space_charge_step_fraction > 1.0
    {
        return Err(FlyError("invalid beam cloud radius or step fraction"));
    }
    for p in particles {
        p.validate(o.relativistic)?;
    }
    if field.time_scale_us().is_some() || field.next_time_event(0.0).is_some() {
        return Err(FlyError("beam mode requires a static external field"));
    }
    let currents =
        ChargeMode::Coulombic(options.current_a).charges(particles, &options.charge_weights)?;
    let mut states: Vec<_> = particles
        .iter()
        .map(|p| ParticleState {
            time_us: 0.0,
            ..p.initial
        })
        .collect();
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
    let mut terminations = vec![BeamTermination::LeaderStopped; particles.len()];
    let mut live: Vec<_> = (0..particles.len()).collect();
    let mut step = o.initial_step_us.min(o.max_step_us);
    let mut event_step = None::<f64>;
    let mut cached_force = None;
    let mut global_steps = 0;
    let mut rejected_steps = 0;
    let _ = rayon::current_num_threads();
    let start = Instant::now();
    if !states.is_empty() {
        normal(states[0])?;
        // Initial cross sections may lie upstream of the leader's launch plane.
        for i in 1..states.len() {
            match to_plane(field, &particles[i], states[i], states[0], [0.0; 3], 0.0, o)? {
                Some(s) => states[i] = s,
                None => {
                    terminations[i] = BeamTermination::Diverged;
                    live.retain(|&j| j != i);
                }
            }
        }
    }
    for &i in &live {
        if let Some(reason) = field.status(states[i].position_mm).or_else(|| {
            o.stop_plane
                .filter(|p| states[i].position_mm[p.axis] == p.coordinate_mm)
                .map(|_| SplatReason::StopPlane)
        }) {
            terminations[i] = BeamTermination::Flight(reason);
            results[i].splat_reason = reason;
        }
    }
    live.retain(|&i| !matches!(terminations[i], BeamTermination::Flight(_)));
    while live.first() == Some(&0) && states[0].time_us < o.duration_us {
        if global_steps + rejected_steps >= o.max_steps {
            for &i in &live {
                results[i].splat_reason = SplatReason::StepLimit;
            }
            terminations[0] = BeamTermination::Flight(SplatReason::StepLimit);
            break;
        }
        let n = normal(states[0])?;
        let leader_speed = dot(states[0].velocity_mm_per_us, n);
        // A follower that turns back cannot intersect future planes.
        let population = live.len();
        live.retain(|&i| {
            if dot(states[i].velocity_mm_per_us, n) <= 0.0 {
                terminations[i] = BeamTermination::Diverged;
                false
            } else {
                true
            }
        });
        if live.len() != population {
            cached_force = None;
        }
        if live.first() != Some(&0) {
            break;
        }
        let positions: Vec<_> = live.iter().map(|&i| states[i].position_mm).collect();
        let velocities: Vec<_> = live.iter().map(|&i| states[i].velocity_mm_per_us).collect();
        let source: Vec<_> = live.iter().map(|&i| currents[i]).collect();
        let ratios: Vec<_> = live
            .iter()
            .map(|&i| particles[i].charge_e / particles[i].mass_amu)
            .collect();
        let (initial_e, encounter_time) = if let Some(force) = cached_force.take() {
            force
        } else {
            beam_field_and_time_scale(
                &positions,
                &velocities,
                &source,
                states[0],
                (options.interaction, options.softening_mm),
                &ratios,
                true,
            )?
        };
        let mut h = step
            .min(o.duration_us - states[0].time_us)
            .min(options.space_charge_step_fraction * encounter_time);
        for (k, &i) in live.iter().enumerate() {
            let clock = leader_speed / dot(velocities[k], n);
            h = h.min(
                o.step_limit_us(
                    field,
                    states[i],
                    particles[i].charge_e / particles[i].mass_amu,
                ) / clock,
            );
            if matches!(options.interaction, BeamInteraction::Lines) {
                continue;
            }
            let acceleration = crate::ELECTRIC_ACCELERATION
                * (particles[i].charge_e / particles[i].mass_amu).abs()
                * dot(initial_e[k], initial_e[k]).sqrt();
            let mut radius = f64::INFINITY;
            for (j, p) in positions.iter().enumerate() {
                if j == k {
                    continue;
                }
                let r = std::array::from_fn(|a| p[a] - positions[k][a]);
                let along = dot(r, n);
                let r = std::array::from_fn(|a| r[a] - along * n[a]);
                let r2 = dot(r, r) + options.softening_mm.powi(2);
                radius = radius.min(r2.sqrt());
            }
            h = h.min(options.space_charge_step_fraction * (radius / acceleration).sqrt() / clock);
        }
        if h <= 0.0 || states[0].time_us + h == states[0].time_us {
            return Err(FlyError("beam plane step needs a step below min_step_us"));
        }
        // Drift to the force plane before applying its central line kick.
        // Including the old SC acceleration in these positions introduces
        // artificial pair torque and can collapse attractive encounters.
        // Half-kick velocities still predict the normal/density to O(h^2).
        let mut mid_leader = push(
            field,
            &particles[0],
            states[0],
            0.5 * h,
            [0.0; 3],
            o.relativistic,
        );
        mid_leader.velocity_mm_per_us = boris_velocity(
            &particles[0],
            mid_leader.velocity_mm_per_us,
            FieldSample {
                electric_v_per_mm: initial_e[0],
                ..FieldSample::default()
            },
            0.5 * h,
            o.relativistic,
        );
        let mid_target = |(k, &i): (usize, &usize)| {
            let guess = 0.5 * h * leader_speed / dot(velocities[k], n);
            if i == 0 {
                return Ok(Some(mid_leader));
            }
            let Some(mut mid) = to_plane(
                field,
                &particles[i],
                states[i],
                mid_leader,
                [0.0; 3],
                guess,
                o,
            )?
            else {
                return Ok(None);
            };
            mid.velocity_mm_per_us = boris_velocity(
                &particles[i],
                mid.velocity_mm_per_us,
                FieldSample {
                    electric_v_per_mm: initial_e[k],
                    ..FieldSample::default()
                },
                mid.time_us - states[i].time_us,
                o.relativistic,
            );
            Ok(Some(mid))
        };
        // Unlike the cheap pair sum at small N, each target performs several
        // reconstructed-field samples and a plane root solve.
        let midpoint: Vec<_> = if live.len() >= 64 {
            live.par_iter()
                .enumerate()
                .map(mid_target)
                .collect::<Result<_, _>>()?
        } else {
            live.iter()
                .enumerate()
                .map(mid_target)
                .collect::<Result<_, _>>()?
        };
        if let Some(k) = midpoint.iter().position(Option::is_none) {
            let i = live.remove(k);
            terminations[i] = BeamTermination::Diverged;
            continue;
        }
        let midpoint: Vec<_> = midpoint.into_iter().flatten().collect();
        let mid_p: Vec<_> = midpoint.iter().map(|s| s.position_mm).collect();
        let mid_v: Vec<_> = midpoint.iter().map(|s| s.velocity_mm_per_us).collect();
        let e = match beam_field_and_time_scale(
            &mid_p,
            &mid_v,
            &source,
            mid_leader,
            (options.interaction, options.softening_mm),
            &ratios,
            false,
        ) {
            Ok((e, _)) => e,
            Err(_) => {
                cached_force = Some((initial_e, encounter_time));
                event_step = None;
                step = 0.5 * h;
                rejected_steps += 1;
                continue;
            }
        };
        let next_leader = push(field, &particles[0], states[0], h, e[0], o.relativistic);
        let end_target = |(k, &i): (usize, &usize)| {
            // Do not subtract nearly equal absolute follower clocks: a half
            // step may be below one clock ULP even when the full step advances.
            let guess = h * leader_speed / dot(velocities[k], n);
            if i == 0 {
                Ok(Some(next_leader))
            } else {
                to_plane(field, &particles[i], states[i], next_leader, e[k], guess, o)
            }
        };
        let next: Vec<_> = if live.len() >= 64 {
            live.par_iter()
                .enumerate()
                .map(end_target)
                .collect::<Result<_, _>>()?
        } else {
            live.iter()
                .enumerate()
                .map(end_target)
                .collect::<Result<_, _>>()?
        };
        if let Some(k) = next.iter().position(Option::is_none) {
            let i = live.remove(k);
            terminations[i] = BeamTermination::Diverged;
            continue;
        }
        let next: Vec<_> = next.into_iter().flatten().collect();
        let target_error = |(&i, s): (&usize, &ParticleState)| {
            external_error(
                field,
                &particles[i],
                states[i],
                s.time_us - states[i].time_us,
                o,
            )
        };
        let error = if live.len() >= 64 {
            live.par_iter()
                .zip(&next)
                .map(target_error)
                .reduce(|| 0.0, f64::max)
        } else {
            live.iter().zip(&next).map(target_error).fold(0.0, f64::max)
        };
        if !error.is_finite() {
            return Err(FlyError("non-finite beam external-field error"));
        }
        if error > 1.0 {
            cached_force = Some((initial_e, encounter_time));
            event_step = None;
            rejected_steps += 1;
            step = h * (0.8 * error.powf(-1.0 / 3.0)).clamp(0.1, 0.5);
            if step < o.min_step_us {
                return Err(FlyError(
                    "beam external-field tolerance needs a step below min_step_us",
                ));
            }
            continue;
        }
        let events: Vec<_> = live
            .iter()
            .zip(&next)
            .map(|(&i, s)| crossing(field, states[i].position_mm, s.position_mm, o))
            .collect();
        let fraction = events
            .iter()
            .flatten()
            .map(|event| event.0)
            .fold(1.0, f64::min);
        if fraction == 0.0 || states[0].time_us + h * fraction == states[0].time_us {
            for ((&i, s), &event) in live.iter().zip(&next).zip(&events) {
                if let Some((f, reason)) = event
                    && f == fraction
                {
                    let old = states[i];
                    states[i] = ParticleState {
                        position_mm: std::array::from_fn(|a| {
                            old.position_mm[a] + f * (s.position_mm[a] - old.position_mm[a])
                        }),
                        velocity_mm_per_us: std::array::from_fn(|a| {
                            old.velocity_mm_per_us[a]
                                + f * (s.velocity_mm_per_us[a] - old.velocity_mm_per_us[a])
                        }),
                        time_us: old.time_us,
                    };
                    results[i].splat_reason = reason;
                    terminations[i] = BeamTermination::Flight(reason);
                }
            }
            live.retain(|&i| !matches!(terminations[i], BeamTermination::Flight(_)));
            if let Some(uncut) = event_step.take() {
                step = uncut;
            }
            continue;
        }
        let plane_roundoff = 64.0
            * f64::EPSILON
            * next_leader
                .position_mm
                .iter()
                .map(|v| v.abs())
                .fold(1.0, f64::max);
        if fraction > 0.0
            && fraction < 1.0 - 1e-9
            && (1.0 - fraction) * h * leader_speed > plane_roundoff
        {
            cached_force = Some((initial_e, encounter_time));
            event_step.get_or_insert(h);
            step = h * fraction;
            rejected_steps += 1;
            continue;
        }
        let endpoint_force = if events.iter().all(Option::is_none) {
            let next_p: Vec<_> = next.iter().map(|s| s.position_mm).collect();
            let next_v: Vec<_> = next.iter().map(|s| s.velocity_mm_per_us).collect();
            match beam_field_and_time_scale(
                &next_p,
                &next_v,
                &source,
                next_leader,
                (options.interaction, options.softening_mm),
                &ratios,
                true,
            ) {
                Ok(force) if h <= 2.0 * options.space_charge_step_fraction * force.1 => Some(force),
                _ => {
                    // A step may enter an encounter much faster than its
                    // initial scale predicts. Reject before accepting a
                    // coincident/sub-clock state; do not soften the line.
                    cached_force = Some((initial_e, encounter_time));
                    event_step = None;
                    step = 0.5 * h;
                    rejected_steps += 1;
                    continue;
                }
            }
        } else {
            None
        };
        for ((&i, &candidate), &event) in live.iter().zip(&next).zip(&events) {
            let old = states[i];
            let mut s = candidate;
            if let Some((f, reason)) = event {
                s = ParticleState {
                    position_mm: std::array::from_fn(|a| {
                        old.position_mm[a] + f * (s.position_mm[a] - old.position_mm[a])
                    }),
                    velocity_mm_per_us: std::array::from_fn(|a| {
                        old.velocity_mm_per_us[a]
                            + f * (s.velocity_mm_per_us[a] - old.velocity_mm_per_us[a])
                    }),
                    time_us: old.time_us + f * (s.time_us - old.time_us),
                };
                results[i].splat_reason = reason;
                terminations[i] = BeamTermination::Flight(reason);
            }
            if s.position_mm
                .iter()
                .chain(&s.velocity_mm_per_us)
                .chain([&s.time_us])
                .any(|v| !v.is_finite())
            {
                return Err(FlyError("non-finite beam state"));
            }
            states[i] = s;
            results[i].accepted_steps += 1;
            if o.sample_every
                .is_some_and(|n| results[i].accepted_steps % n == 0)
                && let Some(samples) = &mut results[i].trajectory
            {
                samples.push(ParticleState {
                    time_us: s.time_us + particles[i].initial.time_us,
                    ..s
                });
            }
        }
        let population = live.len();
        live.retain(|&i| !matches!(terminations[i], BeamTermination::Flight(_)));
        if live.len() == population {
            cached_force = endpoint_force;
        }
        global_steps += 1;
        // A localized loss is not evidence that the next plane needs a tiny
        // step. Restore the pre-event trial; all physical/error limits are
        // evaluated again with the reduced source population.
        step = if live.len() < population {
            event_step.take().unwrap_or(2.0 * h)
        } else {
            2.0 * h
        }
        .min(o.max_step_us);
    }
    if !states.is_empty() && matches!(terminations[0], BeamTermination::LeaderStopped) {
        terminations[0] = BeamTermination::Flight(SplatReason::TimeLimit);
    }
    for (i, r) in results.iter_mut().enumerate() {
        r.tof_us = states[i].time_us;
        states[i].time_us += particles[i].initial.time_us;
        r.final_state = states[i];
        r.rejected_steps = rejected_steps;
        if let Some(samples) = &mut r.trajectory
            && samples.last() != Some(&states[i])
        {
            samples.push(states[i]);
        }
    }
    Ok(BeamReport {
        ensemble: EnsembleReport {
            results,
            global_steps,
            rejected_steps,
            compute_time: start.elapsed(),
        },
        terminations,
    })
}
