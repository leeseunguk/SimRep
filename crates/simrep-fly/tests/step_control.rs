use std::collections::BTreeMap;

use simrep_core::PotentialArray;
use simrep_fly::{
    AnalyticField, ELECTRIC_ACCELERATION, Field, FieldSample, FlightOptions, GridField,
    InstanceField, MAGNETIC_FREQUENCY, Particle, SplatReason, StepLimits, StopPlane,
    VoltageExpression, fly,
};

#[test]
fn transit_limit_scales_with_grid_placement_speed_and_acceleration() {
    let mut array = PotentialArray::new(5, 1, 1);
    array.dx_mm = 0.25;
    // Inactive axes must not impose a smaller transit limit.
    array.dy_mm = 1e-6;
    let field = InstanceField::new(
        GridField::new(array).unwrap(),
        [0.0; 3],
        [[0.0, 1.0, 0.0], [-1.0, 0.0, 0.0], [0.0, 0.0, 1.0]],
        2.0,
        None,
        None,
    )
    .unwrap();
    assert_eq!(field.spatial_scale_mm(), Some(0.5));
    let ion = Particle::new(1.0, 0.0, [0.0; 3], [0.0, 0.01, 0.0], 0.0).unwrap();
    struct Accelerating;
    impl Field for Accelerating {
        fn sample(&self, _: [f64; 3], _: f64) -> FieldSample {
            FieldSample {
                electric_v_per_mm: [2.0 / ELECTRIC_ACCELERATION, 0.0, 0.0],
                ..Default::default()
            }
        }
        fn spatial_scale_mm(&self) -> Option<f64> {
            Some(1.0)
        }
    }
    let rest = Particle::new(1.0, 1.0, [0.0; 3], [0.0; 3], 0.0).unwrap();
    for max_grid_cells in [1.0, 4.0] {
        let options = FlightOptions {
            step_limits: StepLimits {
                max_grid_cells,
                ..StepLimits::default()
            },
            ..FlightOptions::default()
        };
        assert_eq!(
            options.step_limit_us(&field, ion.initial, 0.0),
            50.0 * max_grid_cells
        );
        assert!(
            (options.step_limit_us(&Accelerating, rest.initial, 1.0) - max_grid_cells.sqrt()).abs()
                < 1e-14
        );
        let explicit = FlightOptions {
            max_step_us: 10.0,
            ..options
        };
        assert_eq!(
            explicit.step_limit_us(&Accelerating, rest.initial, 1.0),
            10.0
        );
    }
}

#[test]
fn cyclotron_guard_and_slow_field_free_flights_do_not_have_a_fixed_ceiling() {
    let field = AnalyticField(|_, _| FieldSample {
        magnetic_gauss: [0.0, 0.0, 100.0],
        ..Default::default()
    });
    let ion = Particle::new(2.0, -1.0, [0.0; 3], [0.01, 0.0, 0.0], 0.0).unwrap();
    let options = FlightOptions::default();
    assert!(
        (options.step_limit_us(&field, ion.initial, -0.5) - 0.2 / (50.0 * MAGNETIC_FREQUENCY))
            .abs()
            < 1e-14
    );
    let drift = fly(
        &AnalyticField(|_, _| FieldSample::default()),
        &ion,
        &FlightOptions {
            duration_us: 1000.0,
            ..options
        },
    )
    .unwrap();
    assert_eq!(drift.splat_reason, SplatReason::TimeLimit);
    assert!(drift.accepted_steps < 20);
    assert!((drift.final_state.position_mm[0] - 10.0).abs() < 1e-12);
}

#[test]
fn rf_phase_scale_is_parameter_bound_and_keeps_the_fastest_component() {
    let e = VoltageExpression::parse(
        "3*cos(2*pi*f*(t+4))+sin(2*pi*10*t)+exp(-t/2)",
        &BTreeMap::from([("f".into(), 140.0)]),
    )
    .unwrap();
    assert!((e.time_scale_us().unwrap() - 1.0 / (280.0 * std::f64::consts::PI)).abs() < 1e-15);
    assert_eq!(
        VoltageExpression::parse("pulse(1,0.000001)", &BTreeMap::new())
            .unwrap()
            .next_event(1.0),
        Some(1.000001)
    );
}

#[test]
fn geometry_and_voltage_metadata_are_cached_until_the_next_event() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Pulse {
        expression: VoltageExpression,
        spatial_queries: AtomicUsize,
        phase_queries: AtomicUsize,
        event_queries: AtomicUsize,
    }
    impl Field for Pulse {
        fn sample(&self, _: [f64; 3], t: f64) -> FieldSample {
            FieldSample {
                electric_v_per_mm: [
                    self.expression.evaluate(t) / ELECTRIC_ACCELERATION,
                    0.0,
                    0.0,
                ],
                ..Default::default()
            }
        }
        fn spatial_scale_mm(&self) -> Option<f64> {
            self.spatial_queries.fetch_add(1, Ordering::Relaxed);
            Some(1.0)
        }
        fn time_scale_us(&self) -> Option<f64> {
            self.phase_queries.fetch_add(1, Ordering::Relaxed);
            Some(0.625)
        }
        fn next_time_event(&self, t: f64) -> Option<f64> {
            self.event_queries.fetch_add(1, Ordering::Relaxed);
            self.expression.next_event(t)
        }
    }
    let field = Pulse {
        expression: VoltageExpression::parse("pulse(0.333,0.000001)", &BTreeMap::new()).unwrap(),
        spatial_queries: AtomicUsize::new(0),
        phase_queries: AtomicUsize::new(0),
        event_queries: AtomicUsize::new(0),
    };
    let ion = Particle::new(1.0, 1.0, [0.0; 3], [1.0, 0.0, 0.0], 0.0).unwrap();
    let result = fly(
        &field,
        &ion,
        &FlightOptions {
            automatic_tolerance: false,
            duration_us: 2.0,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(result.splat_reason, SplatReason::TimeLimit);
    assert!((result.final_state.velocity_mm_per_us[0] - 1.000001).abs() < 1e-12);
    assert_eq!(field.spatial_queries.load(Ordering::Relaxed), 1);
    assert_eq!(field.phase_queries.load(Ordering::Relaxed), 1);
    assert_eq!(field.event_queries.load(Ordering::Relaxed), 3);
}

#[test]
fn automatic_tolerance_estimates_grid_variation_and_respects_explicit_values() {
    struct GridVariation {
        magnetic: bool,
        dynamic: bool,
        quartic: bool,
    }
    impl Field for GridVariation {
        fn sample(&self, p: [f64; 3], _: f64) -> FieldSample {
            FieldSample {
                electric_v_per_mm: [
                    1.0 + if self.quartic { p[0].powi(4) } else { p[0] },
                    0.0,
                    0.0,
                ],
                magnetic_gauss: [0.0, 0.0, if self.magnetic { 1.0 } else { 0.0 }],
                ..Default::default()
            }
        }
        fn spatial_scale_mm(&self) -> Option<f64> {
            Some(0.25)
        }
        fn time_scale_us(&self) -> Option<f64> {
            self.dynamic.then_some(1.0)
        }
    }
    let mut field = GridVariation {
        magnetic: false,
        dynamic: false,
        quartic: true,
    };
    let initial = Particle::new(1.0, 1.0, [0.0; 3], [1.0, 0.0, 0.0], 0.0)
        .unwrap()
        .initial;
    let options = FlightOptions::default();
    let resolved = options.with_field_tolerance(&field, initial);
    assert_eq!(resolved.relative_tolerance, 1e-8);
    assert_eq!(resolved.position_tolerance_mm, 1e-8);
    assert_eq!(resolved.velocity_tolerance_mm_per_us, 1e-8);
    let mut faster = initial;
    faster.velocity_mm_per_us[0] = 10.0;
    assert_eq!(
        options
            .with_field_tolerance(&field, faster)
            .velocity_tolerance_mm_per_us,
        1e-7
    );
    for explicit in [
        FlightOptions {
            automatic_tolerance: false,
            ..options.clone()
        },
        FlightOptions {
            max_step_us: 0.1,
            ..options.clone()
        },
        FlightOptions {
            position_tolerance_mm: 1e-14,
            ..options.clone()
        },
    ] {
        let resolved = explicit.with_field_tolerance(&field, initial);
        assert_eq!(resolved.relative_tolerance, explicit.relative_tolerance);
        assert_eq!(
            resolved.position_tolerance_mm,
            explicit.position_tolerance_mm
        );
    }
    field.magnetic = true;
    assert_eq!(
        options
            .with_field_tolerance(&field, initial)
            .relative_tolerance,
        1e-12
    );
    field.magnetic = false;
    field.dynamic = true;
    assert_eq!(
        options
            .with_field_tolerance(&field, initial)
            .relative_tolerance,
        1e-12
    );
    field.dynamic = false;
    field.quartic = false;
    assert_eq!(
        options
            .with_field_tolerance(&field, initial)
            .relative_tolerance,
        1e-12
    );
}

#[test]
fn rk45_reuses_derivatives_and_the_automatic_ceiling_sample() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Counting(AtomicUsize);
    impl Field for Counting {
        fn sample(&self, _: [f64; 3], _: f64) -> FieldSample {
            self.0.fetch_add(1, Ordering::Relaxed);
            FieldSample {
                electric_v_per_mm: [1.0 / ELECTRIC_ACCELERATION, 0.0, 0.0],
                ..Default::default()
            }
        }
        fn time_scale_us(&self) -> Option<f64> {
            Some(0.625)
        }
    }
    let field = Counting(AtomicUsize::new(0));
    for (max_step_us, initial_step_us, duration_us, birth) in [
        (0.125, 0.125, 1.0, 0.0),
        (f64::MAX, 0.125, 1.0, 0.0),
        (0.03, 0.03, 0.31, 17.3),
    ] {
        let ion = Particle::new(1.0, 1.0, [0.0; 3], [0.0; 3], birth).unwrap();
        field.0.store(0, Ordering::Relaxed);
        let result = fly(
            &field,
            &ion,
            &FlightOptions {
                duration_us,
                initial_step_us,
                max_step_us,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(
            result.accepted_steps,
            (duration_us / initial_step_us).ceil() as usize
        );
        assert_eq!(result.rejected_steps, 0);
        assert_eq!(
            field.0.load(Ordering::Relaxed),
            6 * result.accepted_steps + 1
        );
        assert!((result.final_state.position_mm[0] - 0.5 * duration_us.powi(2)).abs() < 1e-14);
        assert!((result.final_state.velocity_mm_per_us[0] - duration_us).abs() < 1e-14);
    }
}

#[test]
fn automatic_steps_hit_a_thin_electrode_and_split_a_narrow_pulse() {
    let mut array = PotentialArray::new(11, 1, 1);
    array.electrodes[5] = true;
    let ion = Particle::new(1.0, 0.0, [0.0; 3], [100.0, 0.0, 0.0], 0.0).unwrap();
    for max_step_us in [f64::MAX, 1.0] {
        let result = fly(
            &GridField::new(array.clone())
                .unwrap()
                .with_nearest_node_collisions(),
            &ion,
            &FlightOptions {
                initial_step_us: 1.0,
                max_step_us,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(result.splat_reason, SplatReason::Electrode { index: 5 });
        assert!((result.final_state.position_mm[0] - 4.5).abs() < 1e-11);
        assert!((result.tof_us - 0.045).abs() < 1e-12);
    }
    struct Pulse(VoltageExpression);
    impl Field for Pulse {
        fn sample(&self, _: [f64; 3], t: f64) -> FieldSample {
            FieldSample {
                electric_v_per_mm: [self.0.evaluate(t) / ELECTRIC_ACCELERATION, 0.0, 0.0],
                ..Default::default()
            }
        }
        fn next_time_event(&self, t: f64) -> Option<f64> {
            self.0.next_event(t)
        }
    }
    let pulse = Pulse(VoltageExpression::parse("pulse(0.333,0.000001)", &BTreeMap::new()).unwrap());
    let ion = Particle::new(1.0, 1.0, [0.0; 3], [0.0; 3], 0.0).unwrap();
    let result = fly(&pulse, &ion, &FlightOptions::default()).unwrap();
    assert_eq!(result.splat_reason, SplatReason::TimeLimit);
    assert!((result.final_state.velocity_mm_per_us[0] - 1e-6).abs() < 1e-13);
}

#[test]
fn event_localization_uses_the_requested_position_budget() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    let samples = AtomicUsize::new(0);
    let field = AnalyticField(|_, _| {
        samples.fetch_add(1, Ordering::Relaxed);
        FieldSample::default()
    });
    let ion = Particle::new(1.0, 1.0, [0.0; 3], [1.0, 0.0, 0.0], 0.0).unwrap();
    let result = fly(
        &field,
        &ion,
        &FlightOptions {
            initial_step_us: 1.0,
            max_step_us: 1.0,
            position_tolerance_mm: 1e-10,
            stop_plane: Some(StopPlane {
                axis: 0,
                coordinate_mm: 0.37,
            }),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(result.splat_reason, SplatReason::StopPlane);
    assert!((result.final_state.position_mm[0] - 0.37).abs() <= 1e-10);
    assert!((result.tof_us - 0.37).abs() <= 1e-10);
    assert!(samples.load(Ordering::Relaxed) <= 230);
}

#[test]
fn curved_event_proposals_preserve_the_verified_time_bracket() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    let samples = AtomicUsize::new(0);
    let field = AnalyticField(|_, _| {
        samples.fetch_add(1, Ordering::Relaxed);
        FieldSample {
            electric_v_per_mm: [0.1 / ELECTRIC_ACCELERATION, 0.0, 0.0],
            ..Default::default()
        }
    });
    let ion = Particle::new(1.0, 1.0, [0.0; 3], [1.0, 0.0, 0.0], 0.0).unwrap();
    let result = fly(
        &field,
        &ion,
        &FlightOptions {
            initial_step_us: 1.0,
            max_step_us: 1.0,
            position_tolerance_mm: 1e-10,
            stop_plane: Some(StopPlane {
                axis: 0,
                coordinate_mm: 0.37,
            }),
            ..Default::default()
        },
    )
    .unwrap();
    let expected_tof = 0.74 / (1.0 + 1.074_f64.sqrt());
    assert_eq!(result.splat_reason, SplatReason::StopPlane);
    assert!((result.final_state.position_mm[0] - 0.37).abs() <= 1e-10);
    assert!((result.tof_us - expected_tof).abs() <= 1e-10);
    let count = samples.load(Ordering::Relaxed);
    assert!(count <= 100, "sample count: {count}");
}
