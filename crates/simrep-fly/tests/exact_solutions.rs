//! CPU public-API checks; derivations and validity are in exact_solutions.json.
use std::f64::consts::PI;

use simrep_core::{
    uncertainty::{Level, observed_order},
    validation::Gate,
};
use simrep_fly::{
    AnalyticField, ELECTRIC_ACCELERATION, Field, FieldSample, FlightOptions, Integrator,
    MAGNETIC_FREQUENCY, Particle, SPEED_OF_LIGHT, SplatReason, StopPlane, fly,
    physics::space_charge::EPSILON_0_F_PER_MM,
};
use simrep_geom::{Electrode, Sdf};
use simrep_solve::{
    cutcell::CutCells,
    mg::{self, Geometry, Grid, Options},
};

fn record(name: &str, metric: &str, value: f64, datum: f64, u_num: Option<f64>, band: &str) {
    println!(
        "{name}: S={value:.12e}, D={datum:.12e}, error={:.6e}, u_num={u_num:?}",
        value - datum
    );
    Gate {
        name,
        reference: "exact_solutions.json; derived here",
        metric,
        value,
        datum,
        u_num,
        u_input: 0.0,
        u_datum: 0.0,
        band,
    }
    .record()
    .unwrap();
}

fn study(name: &str, metric: &str, values: [f64; 3], datum: f64, band: &str) {
    let levels = std::array::from_fn(|i| Level {
        h: 2.0_f64.powi(i as i32),
        value: values[2 - i],
    });
    let triplet = observed_order(&levels).unwrap();
    println!("{name}: coarse to fine {values:?}, p={:?}", triplet.p);
    let roundoff_limited = values.windows(2).all(|pair| {
        (pair[0] - pair[1]).abs() <= 64.0 * f64::EPSILON * pair[0].abs().max(pair[1].abs())
    });
    record(
        name,
        metric,
        values[2],
        datum,
        if roundoff_limited {
            None
        } else {
            triplet.gci_fine.map(|g| g * values[2].abs())
        },
        band,
    );
}

fn fixed_step(duration: f64, n: usize) -> FlightOptions {
    FlightOptions {
        integrator: Integrator::Boris,
        duration_us: duration,
        initial_step_us: duration / n as f64,
        max_step_us: duration / n as f64,
        ..Default::default()
    }
}

#[test]
fn parallel_plate_deflection() {
    let field = AnalyticField(|_, _| FieldSample {
        electric_v_per_mm: [0.0, 2.0, 0.0],
        ..Default::default()
    });
    let ion = Particle::new(40.0, 1.0, [0.0; 3], [10.0, 0.0, 0.0], 0.0).unwrap();
    let result = fly(
        &field,
        &ion,
        &FlightOptions {
            duration_us: 2.0,
            stop_plane: Some(StopPlane {
                axis: 0,
                coordinate_mm: 10.0,
            }),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(result.splat_reason, SplatReason::StopPlane);
    let exact = ELECTRIC_ACCELERATION * 2.0 / 40.0 / 2.0;
    assert!((result.tof_us - 1.0).abs() < 1e-12);
    assert!((result.final_state.position_mm[1] - exact).abs() < 1e-10);
    record(
        "exact-plate-deflection",
        "deflection (mm)",
        result.final_state.position_mm[1],
        exact,
        None,
        "absolute tolerance 1e-10 mm; polynomial integration at roundoff; no identifiable GCI",
    );
}

fn sector_exit(field: &impl Field, ion: &Particle, transit: f64, n: usize) -> [f64; 3] {
    // Start the detector after leaving the entrance plane, not on that plane.
    let warm = fly(field, ion, &fixed_step(transit / 32.0, n / 32)).unwrap();
    assert_eq!(warm.splat_reason, SplatReason::TimeLimit);
    let continuing = Particle {
        initial: warm.final_state,
        ..ion.clone()
    };
    let result = fly(
        field,
        &continuing,
        &FlightOptions {
            stop_plane: Some(StopPlane {
                axis: 1,
                coordinate_mm: 0.0,
            }),
            ..fixed_step(2.0 * transit, 2 * n)
        },
    )
    .unwrap();
    assert_eq!(result.splat_reason, SplatReason::StopPlane);
    result.final_state.position_mm
}

#[test]
fn spherical_and_cylindrical_passband_and_first_order_focus() {
    let (r1, r2, voltage) = (9.0_f64, 11.0_f64, 100.0);
    let radius = (r1 + r2) / 2.0;
    for spherical in [true, false] {
        let energy = if spherical {
            voltage * r1 * r2 / (r2 * r2 - r1 * r1)
        } else {
            voltage / (2.0 * (r2 / r1).ln())
        };
        let theta = if spherical { PI } else { PI / 2.0_f64.sqrt() };
        let start = PI - theta;
        let field = AnalyticField(|p: [f64; 3], _| {
            let r = if spherical {
                p.iter().map(|v| v * v).sum::<f64>().sqrt()
            } else {
                p[0].hypot(p[1])
            };
            let strength = if spherical {
                voltage * r1 * r2 / ((r2 - r1) * r.powi(3))
            } else {
                voltage / ((r2 / r1).ln() * r * r)
            };
            FieldSample {
                electric_v_per_mm: [
                    -strength * p[0],
                    -strength * p[1],
                    if spherical { -strength * p[2] } else { 0.0 },
                ],
                ..Default::default()
            }
        });
        let ion = |alpha: f64| {
            Particle::from_energy(
                40.0,
                1.0,
                [radius * start.cos(), radius * start.sin(), 0.0],
                energy,
                [-(start - alpha).sin(), (start - alpha).cos(), 0.0],
                0.0,
            )
            .unwrap()
        };
        let transit = theta * radius / (2.0 * ELECTRIC_ACCELERATION * energy / 40.0).sqrt();
        let values = [128, 256, 512].map(|n| -sector_exit(&field, &ion(0.0), transit, n)[0]);
        assert!((values[2] / radius - 1.0).abs() < 5e-5);
        let name = if spherical {
            "spherical"
        } else {
            "cylindrical"
        };
        study(
            &format!("exact-{name}-passband"),
            "exit radius (mm)",
            values,
            radius,
            "relative tolerance 5e-5; three-step Celik GCI; central-ray energy from passband formula",
        );
        let alpha = 1e-3;
        // Odd angular response measures the first-order focusing coefficient.
        let plus = -sector_exit(&field, &ion(alpha), transit, 4096)[0];
        let minus = -sector_exit(&field, &ion(-alpha), transit, 4096)[0];
        let response = (plus - minus) / (2.0 * alpha * radius);
        assert!(response.abs() < 2e-5, "{name}: response={response}");
        record(
            &format!("exact-{name}-focus"),
            "normalized angular response (1/rad)",
            response,
            0.0,
            None,
            "absolute tolerance 2e-5; alpha=1e-3 rad; paraxial zero, finite-ray O(alpha^2); no GCI",
        );
    }
}

#[test]
fn relativistic_cyclotron_radius_and_period() {
    let b = 1e8;
    let ion =
        Particle::from_relativistic_energy(1.0, 1.0, [0.0; 3], 1e9, [1.0, 0.0, 0.0], 0.0).unwrap();
    let speed = ion.initial.velocity_mm_per_us[0];
    let gamma = 1.0 / (1.0 - (speed / SPEED_OF_LIGHT).powi(2)).sqrt();
    let omega = MAGNETIC_FREQUENCY * b / gamma;
    let radius = speed / omega;
    let period = 2.0 * PI / omega;
    let field = AnalyticField(|_, _| FieldSample {
        magnetic_gauss: [0.0, 0.0, b],
        ..Default::default()
    });
    let values = [128, 256, 512].map(|n| {
        let result = fly(
            &field,
            &ion,
            &FlightOptions {
                relativistic: true,
                ..fixed_step(period / 4.0, n)
            },
        )
        .unwrap();
        assert_eq!(result.splat_reason, SplatReason::TimeLimit);
        let p = result.final_state.position_mm;
        let angle = p[0].atan2(p[1] + radius);
        [p[0], 2.0 * PI * result.tof_us / angle]
    });
    for (column, name, exact, unit) in [
        (0, "radius", radius, "radius (mm)"),
        (1, "period", period, "period (us)"),
    ] {
        let samples = values.map(|v| v[column]);
        assert!((samples[2] / exact - 1.0).abs() < 2e-6);
        study(
            &format!("exact-relativistic-cyclotron-{name}"),
            unit,
            samples,
            exact,
            "relative tolerance 2e-6; three-step Celik GCI; gamma from initial speed",
        );
    }
}

#[test]
fn crossed_field_drift_and_wien_balance() {
    let b = 10_000.0;
    let omega = MAGNETIC_FREQUENCY * b / 40.0;
    let period = 2.0 * PI / omega;
    let drift = -ELECTRIC_ACCELERATION / (MAGNETIC_FREQUENCY * b);
    let field = AnalyticField(|_, _| FieldSample {
        electric_v_per_mm: [1.0, 0.0, 0.0],
        magnetic_gauss: [0.0, 0.0, b],
        ..Default::default()
    });
    let rest = Particle::new(40.0, 1.0, [0.0; 3], [0.0; 3], 0.0).unwrap();
    let values = [128, 256, 512].map(|n| {
        let result = fly(&field, &rest, &fixed_step(period, n)).unwrap();
        assert_eq!(result.splat_reason, SplatReason::TimeLimit);
        result.final_state.position_mm[1] / period
    });
    assert!((values[2] / drift - 1.0).abs() < 2e-5);
    study(
        "exact-exb-drift",
        "mean drift velocity (mm/us)",
        values,
        drift,
        "relative tolerance 2e-5; one physical gyroperiod; three-step Celik GCI",
    );
    let ion = Particle::new(40.0, 1.0, [0.0; 3], [0.0, drift, 0.0], 0.0).unwrap();
    let result = fly(&field, &ion, &fixed_step(period / 4.0, 128)).unwrap();
    let error = result.final_state.position_mm[0];
    assert!(error.abs() < 1e-12);
    assert!((result.final_state.position_mm[1] - drift * period / 4.0).abs() < 1e-12);
    record(
        "exact-wien-balance",
        "transverse displacement (mm)",
        error,
        0.0,
        None,
        "absolute tolerance 1e-12 mm; exact force cancellation at roundoff; no identifiable GCI",
    );
    let detuned = Particle::new(40.0, 1.0, [0.0; 3], [0.0, drift * 0.9, 0.0], 0.0).unwrap();
    let result = fly(&field, &detuned, &fixed_step(period / 4.0, 512)).unwrap();
    assert!((result.final_state.position_mm[0] / (-0.1 * drift / omega) - 1.0).abs() < 1e-5);
}

#[test]
fn child_langmuir_poisson_profile_and_current() {
    // Verify the CL source/profile on x/d in [1/8,1], excluding its singular
    // density at the cold cathode. This is not a steady-emission PIC test.
    let voltage: f64 = 50.0;
    let mass = 5.485_799_090_65e-4;
    let current = 4.0 / 9.0
        * EPSILON_0_F_PER_MM
        * (2.0 * ELECTRIC_ACCELERATION / mass).sqrt()
        * 1e6
        * voltage.powf(1.5);
    let values = [17, 33, 65].map(|n| {
        let h = 0.875 / (n - 1) as f64;
        let grid = Grid {
            dims: [n, 3, 1],
            spacing: [h, 0.5, 1.0],
            geometry: Geometry::Planar,
        };
        let exact = |i: usize| voltage * (0.125 + (i % n) as f64 * h).powf(4.0 / 3.0);
        let fixed: Vec<_> = (0..3 * n).map(|i| i % n == 0 || i % n == n - 1).collect();
        let mut potential: Vec<_> = fixed
            .iter()
            .enumerate()
            .map(|(i, &f)| if f { exact(i) } else { 0.0 })
            .collect();
        let source: Vec<_> = (0..3 * n)
            .map(|i| {
                let speed = (2.0 * ELECTRIC_ACCELERATION * exact(i) / mass).sqrt();
                -current / (EPSILON_0_F_PER_MM * speed * 1e6)
            })
            .collect();
        assert!(
            mg::solve_poisson(
                &mut potential,
                &fixed,
                &source,
                grid,
                Options {
                    absolute_tolerance: 1e-8,
                    relative_tolerance: 0.0,
                    max_cycles: 200,
                    ..Default::default()
                },
            )
            .unwrap()
            .converged
        );
        // Same physical point x/d=9/16 on all grids. Infer current from
        // the solved potential and source, not from prescribed boundary data.
        let i = n + n / 2;
        let curvature = (potential[i - 1] - 2.0 * potential[i] + potential[i + 1]) / (h * h);
        let speed = (2.0 * ELECTRIC_ACCELERATION * potential[i] / mass).sqrt();
        EPSILON_0_F_PER_MM * curvature * speed * 1e6
    });
    assert!((values[2] / current - 1.0).abs() < 2e-4);
    study(
        "exact-child-langmuir-current",
        "current density (A/mm^2)",
        values,
        current,
        "relative tolerance 2e-4; three-grid Celik GCI; prescribed CL density, not an emission-limit prediction",
    );
}

#[test]
fn toroidal_small_gap_calibration() {
    // A circular annulus in axisymmetric (r,z) is a pair of torus surfaces.
    let (major, minor, gap) = (4.0, 1.0, 0.2);
    let radius = major + minor;
    let values = [49, 97, 193].map(|n| {
        let h = 2.4 / (n - 1) as f64;
        let origin = [major - 1.2, -1.2, 0.0];
        let circle = |a| Sdf::Cylinder {
            center: [major, 0.0, 0.0],
            radius: a,
            axis: 2,
        };
        let outside = Sdf::Box {
            center: [major, 0.0, 0.0],
            half_extent: [4.0; 3],
        }
        .difference(circle(minor + gap / 2.0));
        let cuts = CutCells::new(
            Grid {
                dims: [n, n, 1],
                spacing: [h, h, 1.0],
                geometry: Geometry::Cylindrical {
                    radial_origin: origin[0],
                },
            },
            origin,
            &[
                Electrode::new(0.0, circle(minor - gap / 2.0)),
                Electrode::new(1.0, outside),
            ],
        )
        .unwrap();
        let mut potential = cuts.values.clone();
        assert!(
            cuts.solve(
                &mut potential,
                Options {
                    relative_tolerance: 1e-12,
                    max_cycles: 200,
                    ..Default::default()
                },
            )
            .unwrap()
            .converged
        );
        let ix = ((radius - origin[0]) / h).round() as usize;
        let iz = ((0.0 - origin[1]) / h).round() as usize;
        let electric = cuts.electric_field(&potential, ix + n * iz).unwrap()[0].abs();
        2.0 / (electric * radius)
    });
    let asymptote = 2.0 * gap / radius;
    assert!((values[2] / asymptote - 1.0).abs() < 0.02);
    study(
        "exact-toroidal-small-gap",
        "k=U/(K/q), dimensionless",
        values,
        asymptote,
        "relative approximation envelope 2%; d/a=0.2, d/R0=0.04; GCI excludes finite-gap model error",
    );
}
