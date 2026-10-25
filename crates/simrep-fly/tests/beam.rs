use simrep_fly::{
    AnalyticField, ELECTRIC_ACCELERATION, Field, FieldSample, FlightOptions, Particle, SplatReason,
    physics::beam::beam_field,
    physics::space_charge::COULOMB_V_MM_PER_C,
    physics::{BeamInteraction, BeamOptions, BeamTermination, fly_beam},
};

fn particle(position: [f64; 3], velocity: [f64; 3], birth: f64, charge: f64) -> Particle {
    Particle::new(1.0, charge, position, velocity, birth).unwrap()
}

fn options(dt: f64, duration: f64) -> BeamOptions {
    BeamOptions {
        flight: FlightOptions {
            initial_step_us: dt,
            max_step_us: dt,
            duration_us: duration,
            ..Default::default()
        },
        space_charge_step_fraction: 1.0,
        ..Default::default()
    }
}

#[test]
fn line_current_field_uses_current_over_plane_speed_and_has_no_axial_force() {
    let positions = [[0.0, -1.0, 0.0], [0.0, 1.0, 0.0]];
    let velocities = [[1.0, 0.0, 0.0], [2.0, 0.0, 0.0]];
    let leader = particle([0.0; 3], [1.0, 0.0, 0.0], 0.0, 0.0).initial;
    let e = beam_field(
        &positions,
        &velocities,
        &[1e-6, 2e-6],
        leader,
        BeamInteraction::Lines,
        0.0,
    )
    .unwrap();
    let expected = COULOMB_V_MM_PER_C * 1e-12;
    assert!((e[0][1] + expected).abs() < 1e-13);
    assert!((e[1][1] - expected).abs() < 1e-13);
    assert_eq!(e[0][0], 0.0);
    assert_eq!(e[1][0], 0.0);
}

#[test]
fn grazing_lines_resolve_relative_motion_even_with_weak_net_acceleration() {
    let field = AnalyticField(|_, _| FieldSample::default());
    let p = [
        particle([0.0; 3], [1.0, 0.0, 0.0], 0.0, 0.0),
        particle([0.0, -1.0, 0.0005], [1.0, 10.0, 0.0], 0.0, 1.0),
        particle([0.0, 1.0, -0.0005], [1.0, -10.0, 0.0], 0.0, 1.0),
    ];
    let mut o = options(0.1, 0.2);
    o.current_a = 1e-10;
    o.space_charge_step_fraction = 0.1;
    let coarse = fly_beam(&field, &p, &o).unwrap();
    o.space_charge_step_fraction = 0.05;
    let fine = fly_beam(&field, &p, &o).unwrap();
    o.flight.max_step_us = 1e-6;
    let reference = fly_beam(&field, &p, &o).unwrap();
    let error = |report: &simrep_fly::physics::BeamReport| {
        report.ensemble.results[1]
            .final_state
            .position_mm
            .iter()
            .zip(reference.ensemble.results[1].final_state.position_mm)
            .map(|(a, b)| (a - b).powi(2))
            .sum::<f64>()
            .sqrt()
    };
    assert!(error(&coarse) < 0.001, "coarse error: {}", error(&coarse));
    assert!(
        error(&coarse) > 3.0 * error(&fine),
        "coarse/fine errors: {} / {}",
        error(&coarse),
        error(&fine)
    );
}

#[test]
fn attracting_lines_preserve_angular_momentum_at_coarse_fractions() {
    let field = AnalyticField(|_, _| FieldSample::default());
    let current = 1e-8;
    let strength = ELECTRIC_ACCELERATION * COULOMB_V_MM_PER_C * current * 1e-6;
    let speed = (0.5 * strength).sqrt();
    let p = [
        particle([0.0; 3], [1.0, 0.0, 0.0], 0.0, 0.0),
        particle([0.0, -0.5, 0.0], [1.0, 0.0, speed], 0.0, 1.0),
        particle([0.0, 0.5, 0.0], [2.0, 0.0, -speed], 0.0, -1.0),
    ];
    for fraction in [0.1, 0.75, 0.8, 1.0] {
        let mut o = options(1.0, 2.0);
        o.current_a = current;
        o.space_charge_step_fraction = fraction;
        o.flight.relativistic = false;
        o.flight.max_steps = 10000;
        let report = fly_beam(&field, &p, &o).unwrap();
        let angular_momentum: f64 = report.ensemble.results[1..]
            .iter()
            .map(|r| {
                let p = r.final_state.position_mm;
                let v = r.final_state.velocity_mm_per_us;
                p[1] * v[2] - p[2] * v[1]
            })
            .sum();
        assert!(
            (angular_momentum + speed).abs() < 1e-10 * speed,
            "fraction {fraction}: angular momentum changed from {} to {angular_momentum}",
            -speed
        );
    }
}

#[test]
fn charged_rotating_leader_retains_second_order_convergence() {
    let field = AnalyticField(|_, _| FieldSample {
        electric_v_per_mm: [0.0, 0.02, -0.01],
        ..FieldSample::default()
    });
    let p = [
        particle([0.0; 3], [2.0, 0.0, 0.0], 0.0, 1.0),
        particle([0.0, 1.0, 0.2], [1.5, 0.1, 0.0], 0.0, 1.0),
        particle([0.0, -0.7, 0.1], [2.5, -0.2, 0.1], 0.0, -1.0),
    ];
    let mut o = options(1e-4, 0.2);
    o.current_a = 1e-9;
    o.flight.relativistic = false;
    let reference = fly_beam(&field, &p, &o).unwrap();
    let mut errors = Vec::new();
    for dt in [0.02, 0.01, 0.005] {
        o.flight.initial_step_us = dt;
        o.flight.max_step_us = dt;
        let report = fly_beam(&field, &p, &o).unwrap();
        errors.push(
            report
                .ensemble
                .results
                .iter()
                .zip(&reference.ensemble.results)
                .flat_map(|(a, b)| {
                    a.final_state
                        .position_mm
                        .iter()
                        .zip(b.final_state.position_mm)
                        .map(|(a, b)| (a - b).powi(2))
                })
                .sum::<f64>()
                .sqrt(),
        );
    }
    eprintln!("charged rotating beam errors={errors:?}");
    assert!(errors[0] / errors[1] > 3.5 && errors[1] / errors[2] > 3.5);
}

#[test]
fn localized_loss_restores_the_uncut_plane_step() {
    struct EarlyLoss(f64);
    impl Field for EarlyLoss {
        fn sample(&self, _: [f64; 3], _: f64) -> FieldSample {
            FieldSample::default()
        }
        fn intersection(&self, from: [f64; 3], to: [f64; 3]) -> Option<(f64, SplatReason)> {
            (from[1] == 1.0 && from[0] < self.0 && to[0] >= self.0).then(|| {
                (
                    (self.0 - from[0]) / (to[0] - from[0]),
                    SplatReason::OutsideGrid,
                )
            })
        }
    }
    let p = [
        particle([0.0; 3], [1.0, 0.0, 0.0], 0.0, 0.0),
        particle([0.0, 1.0, 0.0], [1.0, 0.0, 0.0], 0.0, 0.0),
    ];
    for plane in [1e-8, 0.123_456_789_012_345_6, 0.232_967_85, 0.587_912_3] {
        let mut o = options(0.1, 1.0);
        o.flight.max_steps = 100;
        let report = fly_beam(&EarlyLoss(plane), &p, &o).unwrap();
        assert_eq!(
            report.terminations[1],
            BeamTermination::Flight(SplatReason::OutsideGrid)
        );
        assert!((report.ensemble.results[1].tof_us - plane).abs() < 1e-14);
        assert!((report.ensemble.results[0].tof_us - 1.0).abs() < 1e-14);
        assert!(
            report.ensemble.global_steps <= 12,
            "{} unnecessary recovery planes",
            report.ensemble.global_steps
        );
    }
}

#[test]
fn leader_planes_have_unequal_times_and_ignore_birth_times() {
    struct LeaderDetector;
    impl Field for LeaderDetector {
        fn sample(&self, _: [f64; 3], _: f64) -> FieldSample {
            FieldSample::default()
        }
        fn intersection(&self, from: [f64; 3], to: [f64; 3]) -> Option<(f64, SplatReason)> {
            (from[1] == 0.0 && to[0] >= 4.0)
                .then(|| ((4.0 - from[0]) / (to[0] - from[0]), SplatReason::StopPlane))
        }
    }
    let field = LeaderDetector;
    let p = [
        particle([0.0; 3], [2.0, 0.0, 0.0], 7.0, 0.0),
        particle([0.0, 1.0, 0.0], [1.0, 0.0, 0.0], 91.0, 1.0),
    ];
    let o = options(0.13, 3.0);
    let r = fly_beam(&field, &p, &o).unwrap();
    assert_eq!(
        r.terminations,
        [
            BeamTermination::Flight(SplatReason::StopPlane),
            BeamTermination::LeaderStopped
        ]
    );
    assert!((r.ensemble.results[0].tof_us - 2.0).abs() < 1e-12);
    assert!((r.ensemble.results[1].tof_us - 4.0).abs() < 1e-12);
    for result in &r.ensemble.results {
        assert!((result.final_state.position_mm[0] - 4.0).abs() < 1e-12);
    }
    let shifted = p.map(|mut p| {
        p.initial.time_us += 100.0;
        p
    });
    let again = fly_beam(&field, &shifted, &o).unwrap();
    for (a, b) in r.ensemble.results.iter().zip(&again.ensemble.results) {
        assert_eq!(a.final_state.position_mm, b.final_state.position_mm);
        assert_eq!(
            a.final_state.velocity_mm_per_us,
            b.final_state.velocity_mm_per_us
        );
        assert_eq!(a.tof_us, b.tof_us);
        assert!((b.final_state.time_us - a.final_state.time_us - 100.0).abs() < 1e-12);
    }
}

#[test]
fn backward_follower_is_killed_without_changing_current_weights() {
    let field = AnalyticField(|_, _| FieldSample::default());
    let p = [
        particle([0.0; 3], [1.0, 0.0, 0.0], 0.0, 0.0),
        particle([0.0, 1.0, 0.0], [-1.0, 0.0, 0.0], 0.0, 1.0),
    ];
    let r = fly_beam(&field, &p, &options(0.1, 0.3)).unwrap();
    assert_eq!(r.terminations[1], BeamTermination::Diverged);
    assert_eq!(r.ensemble.results[1].tof_us, 0.0);
    assert!((r.ensemble.results[0].tof_us - 0.3).abs() < 1e-14);
}

#[test]
fn moving_normal_keeps_followers_on_the_leader_plane() {
    let field = AnalyticField(|_, _| FieldSample {
        electric_v_per_mm: [0.0, 0.01, 0.0],
        ..Default::default()
    });
    let p = [
        particle([0.0; 3], [3.0, 0.0, 0.0], 0.0, 1.0),
        particle([0.0, 0.4, 0.2], [2.5, 0.0, 0.0], 0.0, 1.0),
    ];
    let r = fly_beam(&field, &p, &options(0.003, 0.2)).unwrap();
    let leader = r.ensemble.results[0].final_state;
    let other = r.ensemble.results[1].final_state;
    let miss: f64 = (0..3)
        .map(|a| (other.position_mm[a] - leader.position_mm[a]) * leader.velocity_mm_per_us[a])
        .sum();
    assert!(miss.abs() < 1e-10, "plane miss={miss:e}");
    assert!(other.time_us > leader.time_us);
}

#[test]
fn follower_boundary_is_localized_without_sub_ulp_plane_livelock() {
    struct Boundary;
    impl Field for Boundary {
        fn sample(&self, _: [f64; 3], _: f64) -> FieldSample {
            FieldSample {
                electric_v_per_mm: [0.0, 0.001, 0.0],
                ..Default::default()
            }
        }
        fn intersection(&self, from: [f64; 3], to: [f64; 3]) -> Option<(f64, SplatReason)> {
            (to[1] > 0.5).then(|| {
                (
                    (0.5 - from[1]) / (to[1] - from[1]),
                    SplatReason::OutsideGrid,
                )
            })
        }
    }
    let p = [
        particle([0.0; 3], [1.3, 0.0, 0.0], 0.0, 0.0),
        particle([0.0, 0.31, 0.0], [1.1, 1.0, 0.0], 0.0, 1.0),
    ];
    let mut o = options(0.017, 0.3);
    o.flight.max_steps = 1000;
    let report = fly_beam(&Boundary, &p, &o).unwrap();
    assert_eq!(
        report.terminations[1],
        BeamTermination::Flight(SplatReason::OutsideGrid)
    );
    assert_eq!(report.ensemble.results[1].final_state.position_mm[1], 0.5);
    assert_eq!(
        report.ensemble.results[0].splat_reason,
        SplatReason::TimeLimit
    );
    assert!(report.ensemble.global_steps < 100);
}

// Independent RK4 integration of the cold uniform-cylinder envelope R''=C/R.
fn envelope(c: f64, t: f64) -> f64 {
    let h = t / 16384.0;
    let (mut r, mut v) = (1.0, 0.0);
    for _ in 0..16384 {
        let (k1r, k1v) = (v, c / r);
        let (k2r, k2v) = (v + 0.5 * h * k1v, c / (r + 0.5 * h * k1r));
        let (k3r, k3v) = (v + 0.5 * h * k2v, c / (r + 0.5 * h * k2r));
        let (k4r, k4v) = (v + h * k3v, c / (r + h * k3r));
        r += h * (k1r + 2.0 * k2r + 2.0 * k3r + k4r) / 6.0;
        v += h * (k1v + 2.0 * k2v + 2.0 * k3v + k4v) / 6.0;
    }
    r
}

#[test]
fn uniform_current_cylinder_envelope_and_second_order_convergence() {
    let field = AnalyticField(|_, _| FieldSample::default());
    let speed = 10.0;
    let current = 1e-8;
    let duration = 0.48;
    let mut p = vec![particle([0.0; 3], [speed, 0.0, 0.0], 0.0, 0.0)];
    for i in 0..48 {
        let r = ((i as f64 + 0.5) / 48.0).sqrt();
        p.push(particle([0.0, r, 0.0], [speed, 0.0, 0.0], 0.0, 1.0));
    }
    let expected = envelope(
        ELECTRIC_ACCELERATION * 2.0 * COULOMB_V_MM_PER_C * current * 1e-6 / speed,
        duration,
    );
    let mut errors = Vec::new();
    for dt in [0.04, 0.02, 0.01] {
        let mut o = options(dt, duration);
        o.current_a = current;
        o.interaction = BeamInteraction::Rings;
        let report = fly_beam(&field, &p, &o).unwrap();
        let rms = (report.ensemble.results[1..]
            .iter()
            .map(|r| r.final_state.position_mm[1].powi(2))
            .sum::<f64>()
            / 24.0)
            .sqrt();
        errors.push((rms - expected).abs());
    }
    eprintln!("beam uniform cylinder analytic={expected:.12}, errors={errors:?}");
    assert!(errors[2] / expected < 1e-4);
    assert!(errors[0] / errors[1] > 3.7 && errors[1] / errors[2] > 3.7);
}
