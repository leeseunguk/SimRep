use simrep_core::PotentialArray;
use simrep_fly::{
    AnalyticField, ELECTRIC_ACCELERATION, FieldSample, FlightOptions, GridField, InstanceField,
    Integrator, Particle, SplatReason, VoltageExpression, WorkbenchField, fly,
    gpu::{Backends, Gpu},
};
use std::collections::BTreeMap;

fn driven(expression: &str) -> InstanceField {
    let mut unit = PotentialArray::new(5, 5, 1);
    for y in 0..5 {
        for x in 0..5 {
            unit.potentials[y * 5 + x] = -(x as f64);
        }
    }
    InstanceField::new(
        GridField::new(PotentialArray::new(5, 5, 1)).unwrap(),
        [0.0; 3],
        [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        1.0,
        None,
        None,
    )
    .unwrap()
    .with_voltage_expressions(vec![(
        GridField::new(unit).unwrap(),
        VoltageExpression::parse(expression, &BTreeMap::new()).unwrap(),
    )])
    .unwrap()
}

#[test]
fn stage_times_birth_times_midpoints_and_narrow_pulses() {
    let p = Particle::new(ELECTRIC_ACCELERATION, 1.0, [0.1, 2.0, 0.0], [0.0; 3], 2.0).unwrap();
    for integrator in [Integrator::Rk45, Integrator::Boris] {
        let options = FlightOptions {
            integrator,
            duration_us: 1.0,
            initial_step_us: 0.01,
            max_step_us: 0.1,
            ..Default::default()
        };
        let result = fly(&driven("t"), &p, &options).unwrap();
        assert_eq!(result.splat_reason, SplatReason::TimeLimit);
        assert!((result.final_state.velocity_mm_per_us[0] - 2.5).abs() < 1e-10);
        assert!((result.final_state.position_mm[0] - (0.1 + 1.0 + 1.0 / 6.0)).abs() < 1e-5);
    }
    let p = Particle::new(ELECTRIC_ACCELERATION, 1.0, [0.1, 2.0, 0.0], [0.0; 3], 0.0).unwrap();
    let options = FlightOptions {
        duration_us: 1.0,
        initial_step_us: 1.0,
        max_step_us: 1.0,
        ..Default::default()
    };
    for integrator in [Integrator::Rk45, Integrator::Boris] {
        let r = fly(
            &driven("pulse(0.333,0.001)"),
            &p,
            &FlightOptions {
                integrator,
                ..options.clone()
            },
        )
        .unwrap();
        assert_eq!(r.splat_reason, SplatReason::TimeLimit);
        assert!((r.final_state.velocity_mm_per_us[0] - 0.001).abs() < 1e-10);
        assert!((r.final_state.position_mm[0] - (0.1 + 0.001 * (1.0 - 0.3335))).abs() < 1e-10);
        if integrator == Integrator::Boris {
            assert_eq!(r.accepted_steps, 3);
        }
    }
}

#[test]
fn default_phase_guard_prevents_whole_cycle_rk_aliasing() {
    let field = driven("cos(2*pi*1000*t)");
    let ion = Particle::new(ELECTRIC_ACCELERATION, 1.0, [0.1, 2.0, 0.0], [0.0; 3], 0.0).unwrap();
    let result = fly(
        &field,
        &ion,
        &FlightOptions {
            duration_us: 0.18,
            // All seven RK stages at this trial length see cos(phase)=1.
            initial_step_us: 0.09,
            sample_every: Some(1),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(result.splat_reason, SplatReason::TimeLimit);
    assert!((result.final_state.position_mm[0] - 0.1).abs() < 1e-10);
    assert!(result.final_state.velocity_mm_per_us[0].abs() < 1e-10);
    let cap = 0.2 / (2000.0 * std::f64::consts::PI);
    assert!(
        result
            .trajectory
            .unwrap()
            .windows(2)
            .all(|p| { p[1].time_us - p[0].time_us <= cap * (1.0 + 1e-12) })
    );
}

fn mathieu_trace(a: f64, q: f64, step: f64) -> f64 {
    let omega = 2.0 * std::f64::consts::PI;
    let f = AnalyticField(move |r: [f64; 3], t: f64| FieldSample {
        potential_v: omega * omega / 8.0
            * (a - 2.0 * q * (omega * t).cos())
            * (r[0] * r[0] - r[1] * r[1]),
        electric_v_per_mm: [
            -omega * omega / 4.0 * (a - 2.0 * q * (omega * t).cos()) * r[0],
            omega * omega / 4.0 * (a - 2.0 * q * (omega * t).cos()) * r[1],
            0.0,
        ],
        ..Default::default()
    });
    let options = FlightOptions {
        duration_us: 1.0,
        max_step_us: step,
        initial_step_us: step,
        relative_tolerance: 1e-12,
        position_tolerance_mm: 1e-12,
        velocity_tolerance_mm_per_us: 1e-12,
        ..Default::default()
    };
    let x = fly(
        &f,
        &Particle::new(ELECTRIC_ACCELERATION, 1.0, [1.0, 0.0, 0.0], [0.0; 3], 0.0).unwrap(),
        &options,
    )
    .unwrap();
    let v = fly(
        &f,
        &Particle::new(ELECTRIC_ACCELERATION, 1.0, [0.0; 3], [1.0, 0.0, 0.0], 0.0).unwrap(),
        &options,
    )
    .unwrap();
    x.final_state.position_mm[0] + v.final_state.velocity_mm_per_us[0]
}

#[test]
fn paul_trap_secular_frequency_and_mathieu_stability() {
    let q = 0.01;
    let trace = mathieu_trace(0.0, q, 0.01);
    let beta = (trace / 2.0).acos() / std::f64::consts::PI;
    let pseudopotential_beta = q / 2.0_f64.sqrt();
    assert!((beta / pseudopotential_beta - 1.0).abs() < 5e-5);
    let mut low = 0.90;
    let mut high = 0.92;
    for _ in 0..30 {
        let mid = (low + high) / 2.0;
        if mathieu_trace(0.0, mid, 0.01).abs() <= 2.0 {
            low = mid;
        } else {
            high = mid;
        }
    }
    assert!(((low + high) / 2.0 - 0.908_046_333_7).abs() < 2e-8);
    assert!((mathieu_trace(0.0, 0.907, 0.01) - mathieu_trace(0.0, 0.907, 0.005)).abs() < 1e-10);
    let (mut lower, mut upper) = (0.9, 1.0);
    let q_fixed = 0.05;
    for _ in 0..30 {
        let a = (lower + upper) / 2.0;
        if mathieu_trace(a, q_fixed, 0.01).abs() <= 2.0 {
            lower = a;
        } else {
            upper = a;
        }
    }
    let b1 = (lower + upper) / 2.0;
    let b1_series = 1.0 - q_fixed - q_fixed * q_fixed / 8.0 + q_fixed.powi(3) / 64.0;
    assert!((b1 - b1_series).abs() < 1e-8);
    eprintln!(
        "Mathieu beta={beta:.12e}, boundary={:.12e}",
        (low + high) / 2.0
    );
    eprintln!("Mathieu b1(q=0.05)={b1:.12e}");
}

#[test]
fn gpu_expression_is_evaluated_at_each_ions_absolute_stage_time() {
    let Some(gpu) = pollster::block_on(Gpu::new(Backends::VULKAN)).unwrap() else {
        return;
    };
    let field = driven("0.1*cos(2*pi*0.14*t)+0.02*t+(-2)^2/100");
    let particles: Vec<_> = [0.0, 2.0, 7.0]
        .into_iter()
        .map(|t| Particle::new(ELECTRIC_ACCELERATION, 1.0, [0.1, 2.0, 0.0], [0.0; 3], t).unwrap())
        .collect();
    let options = FlightOptions {
        duration_us: 1.0,
        max_step_us: 0.05,
        ..Default::default()
    };
    let mut prepared = gpu.prepare(&field, &[]).unwrap();
    let actual = prepared.fly(&particles, &options).unwrap();
    let tolerance = if gpu.has_f64() { 3e-11 } else { 1e-6 };
    for (p, result) in particles.iter().zip(actual.results) {
        let reference = fly(&field, p, &options).unwrap();
        assert_eq!(result.splat_reason, SplatReason::TimeLimit);
        assert!(
            (result.final_state.position_mm[0] - reference.final_state.position_mm[0]).abs()
                < tolerance
        );
        assert!(
            (result.final_state.velocity_mm_per_us[0]
                - reference.final_state.velocity_mm_per_us[0])
                .abs()
                < tolerance
        );
    }
    let first = driven("0.03*t+0.02");
    let mut second_array = PotentialArray::new(5, 5, 1);
    for y in 0..5 {
        for x in 0..5 {
            second_array.potentials[y * 5 + x] = -0.02 * y as f64;
        }
    }
    let second = InstanceField::new(
        GridField::new(second_array).unwrap(),
        [10.0, 4.0, 0.0],
        [[0.0, 1.0, 0.0], [-1.0, 0.0, 0.0], [0.0, 0.0, 1.0]],
        1.0,
        None,
        None,
    )
    .unwrap();
    let workbench =
        WorkbenchField::new(vec![first, second], [[-1.0, -1.0, -1.0], [12.0, 10.0, 1.0]]).unwrap();
    let p = Particle::new(100.0, 1.0, [0.1, 2.0, 0.0], [1.0, 0.1, 0.0], 0.0).unwrap();
    let options = FlightOptions {
        duration_us: 20.0,
        max_step_us: 0.02,
        ..Default::default()
    };
    let reference = fly(&workbench, &p, &options).unwrap();
    let mut resident = gpu.prepare_workbench(&workbench).unwrap();
    let actual = resident.fly(&[p], &options).unwrap();
    assert_eq!(actual.results[0].splat_reason, reference.splat_reason);
    for a in 0..3 {
        assert!(
            (actual.results[0].final_state.position_mm[a] - reference.final_state.position_mm[a])
                .abs()
                < 3e-4
        );
    }
    assert!((actual.results[0].tof_us - reference.tof_us).abs() < 3e-4);

    let mut array = PotentialArray::new(7, 5, 1);
    for y in 0..5 {
        array.electrodes[y * 7 + 1] = true;
        array.electrodes[y * 7 + 4] = true;
    }
    let field = InstanceField::new(
        GridField::new(array)
            .unwrap()
            .with_nearest_node_collisions(),
        [0.0; 3],
        [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        1.0,
        None,
        None,
    )
    .unwrap();
    let p = Particle::new(1.0, 0.0, [1.0, 2.0, 0.0], [1.0, 0.0, 0.0], 0.0).unwrap();
    assert_eq!(fly(&field, &p, &options).unwrap().tof_us, 0.0);
    let reference =
        simrep_fly::fly_many_from_electrode(&field, std::slice::from_ref(&p), &options).unwrap();
    assert!((reference[0].tof_us - 2.5).abs() < 1e-10);
    let mut resident = gpu.prepare(&field, &[]).unwrap();
    let actual = resident.fly_from_electrode(&[p], &options).unwrap();
    assert_eq!(actual.results[0].splat_reason, reference[0].splat_reason);
    assert!((actual.results[0].tof_us - reference[0].tof_us).abs() < 1e-6);

    let field = driven("pulse(0.333,0.001)");
    let p = Particle::new(ELECTRIC_ACCELERATION, 1.0, [0.1, 2.0, 0.0], [0.0; 3], 0.0).unwrap();
    let opts = FlightOptions {
        duration_us: 1.0,
        initial_step_us: 1.0,
        max_step_us: 1.0,
        ..Default::default()
    };
    let mut resident = gpu.prepare(&field, &[]).unwrap();
    let actual = resident.fly(&[p], &opts).unwrap();
    assert!((actual.results[0].final_state.velocity_mm_per_us[0] - 0.001).abs() < 1e-7);

    let mut a = PotentialArray::new(3, 5, 1);
    for y in 0..5 {
        a.electrodes[y * 3 + 2] = true;
    }
    let grid = GridField::new(a)
        .unwrap()
        .with_nearest_node_collisions()
        .with_transparent_faces([false, true, false, false, false, false]);
    let field = InstanceField::new(
        grid,
        [0.0; 3],
        [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        1.0,
        None,
        None,
    )
    .unwrap();
    let p = Particle::new(1.0, 0.0, [0.5, 2.0, 0.0], [1.0, 0.0, 0.0], 0.0).unwrap();
    let reference = fly(&field, &p, &options).unwrap();
    let mut resident = gpu.prepare(&field, &[]).unwrap();
    let actual = resident.fly(&[p], &options).unwrap();
    assert_eq!(reference.splat_reason, SplatReason::OutsideGrid);
    assert_eq!(actual.results[0].splat_reason, reference.splat_reason);
    assert!((actual.results[0].tof_us - reference.tof_us).abs() < 1e-6);
}
