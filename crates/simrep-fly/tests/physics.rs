use std::f64::consts::PI;

use simrep_core::{FieldType, MirrorFlags, PotentialArray, Symmetry};
use simrep_fly::StopPlane;
use simrep_fly::{
    AnalyticField, ELECTRIC_ACCELERATION, Field, FieldSample, FlightOptions, GridField,
    InstanceField, Integrator, MAGNETIC_FREQUENCY, Particle, SplatReason, fly,
    physics::{
        collision::{
            CollisionModel, DiffusionStatistics, Gas, Hs1, Rng, Sds, fly_collisional,
            fly_many_collisional,
        },
        magnetic::{Coil, MagneticField, MagneticPa},
    },
};

fn norm(v: [f64; 3]) -> f64 {
    v.iter().map(|v| v * v).sum::<f64>().sqrt()
}
fn difference(a: [f64; 3], b: [f64; 3]) -> f64 {
    norm(std::array::from_fn(|i| a[i] - b[i]))
}

#[test]
fn magnetic_pa_ng_spacing_mirrors_and_cylindrical_expansion() {
    let mut pa = PotentialArray::new(9, 9, 1);
    pa.field_type = FieldType::Magnetic;
    pa.dx_mm = 0.5;
    pa.dy_mm = 0.25;
    for y in 0..pa.ny {
        for x in 0..pa.nx {
            pa.potentials[y * pa.nx + x] =
                0.5 * (x as f64 * pa.dx_mm).powi(2) + (y as f64 * pa.dy_mm).powi(2);
        }
    }
    pa.mirrors = MirrorFlags {
        x: true,
        y: true,
        z: false,
    };
    let field = MagneticPa::new(pa.clone(), 100.0).unwrap();
    for p in [[1.5, 0.75, 2.0], [-1.5, -0.75, 2.0]] {
        let b = field.sample(p, 0.0);
        assert_eq!(b.electric_v_per_mm, [0.0; 3]);
        assert!(difference(b.magnetic_gauss, [-100.0 * p[0], -200.0 * p[1], 0.0]) < 1e-9);
    }
    pa.symmetry = Symmetry::Cylindrical;
    let field = MagneticPa::new(pa, 100.0).unwrap();
    let p = [-1.5, 0.45, -0.6];
    assert!(difference(field.sample(p, 0.0).magnetic_gauss, [150.0, -90.0, 120.0]) < 1e-9);

    let mut pa = PotentialArray::new(9, 5, 1);
    pa.field_type = FieldType::Magnetic;
    pa.mirrors.x = true;
    pa.anti_mirrors.x = true;
    for y in 0..pa.ny {
        for x in 0..pa.nx {
            pa.potentials[y * pa.nx + x] = x as f64;
        }
    }
    let field = MagneticPa::new(pa, 3.0).unwrap();
    assert!(
        difference(
            field.sample([-2.0, 2.0, 0.0], 0.0).magnetic_gauss,
            [-3.0, 0.0, 0.0]
        ) < 1e-12
    );
}

#[test]
fn magnetic_zero_dirichlet_mirror_face_is_transparent_but_still_sets_field() {
    let mut pa = PotentialArray::new(9, 5, 5);
    pa.field_type = FieldType::Magnetic;
    pa.mirrors.x = true;
    for z in 0..5 {
        for y in 0..5 {
            for x in 0..9 {
                let i = (z * 5 + y) * 9 + x;
                pa.potentials[i] = x as f64;
                pa.electrodes[i] = x == 0 || x >= 7;
            }
        }
    }
    let field = MagneticPa::new(pa, 3.0).unwrap();
    assert_eq!(field.status([0.0, 2.0, 2.0]), None);
    assert!(matches!(
        field.status([8.0, 2.0, 2.0]),
        Some(SplatReason::Electrode { .. })
    ));
    assert!(
        difference(
            field.sample([0.0, 2.0, 2.0], 0.0).magnetic_gauss,
            [-3.0, 0.0, 0.0]
        ) < 1e-11
    );
}

#[test]
fn magnetic_placement_matches_scalar_reconstruction_without_scaling_strength() {
    let origin = [1.0, -2.0, 3.0];
    let axes = [[0.0, 1.0, 0.0], [0.0, 0.0, 1.0], [1.0, 0.0, 0.0]];
    let bounds = Some([[-50.0; 3], [50.0; 3]]);
    let ng = 37.0;
    for cylindrical in [false, true] {
        let mut pa = PotentialArray::new(9, 5, if cylindrical { 1 } else { 5 });
        pa.mirrors.x = true;
        pa.symmetry = if cylindrical {
            Symmetry::Cylindrical
        } else {
            Symmetry::Planar
        };
        for z in 0..pa.nz {
            for y in 0..pa.ny {
                for x in 0..pa.nx {
                    let i = (z * pa.ny + y) * pa.nx + x;
                    pa.potentials[i] = x as f64 * (1.0 + (y as f64).powi(2)) + (z as f64).powi(3);
                    pa.electrodes[i] = x == 0 || x >= 7;
                }
            }
        }
        for scale in [0.25, 1.0, 3.5] {
            let scalar = InstanceField::new(
                GridField::new(pa.clone()).unwrap(),
                origin,
                axes,
                scale,
                bounds,
                None,
            )
            .unwrap();
            let mut magnetic = pa.clone();
            magnetic.field_type = FieldType::Magnetic;
            let magnetic = MagneticPa::placed(magnetic, ng, origin, axes, scale, bounds)
                .unwrap()
                .with_polarity_flip_axis(0)
                .unwrap();
            for local in [
                [2.1, 1.2, 0.7],
                [-2.1, 1.2, 0.7],
                [7.2, 1.3, 0.8],
                [10.0, 1.0, 0.0],
            ] {
                let p = std::array::from_fn(|a| {
                    origin[a] + scale * (0..3).map(|i| axes[i][a] * local[i]).sum::<f64>()
                });
                let sign = if local[0] < 0.0 { -1.0 } else { 1.0 };
                let expected = scalar
                    .sample(p, 0.0)
                    .electric_v_per_mm
                    .map(|e| e * ng * scale * sign);
                assert!(
                    difference(magnetic.sample(p, 0.0).magnetic_gauss, expected)
                        < 1e-10 * norm(expected).max(1.0)
                );
                assert_eq!(magnetic.contains(p), local[0].abs() <= 8.0);
            }
        }
    }
}

#[test]
fn magnetic_pole_body_starts_at_its_node_not_half_a_cell_early() {
    let mut pa = PotentialArray::new(9, 3, 1);
    pa.field_type = FieldType::Magnetic;
    for y in 0..3 {
        for x in 0..9 {
            pa.electrodes[y * 9 + x] = x <= 2;
        }
    }
    let field = MagneticPa::new(pa, 1.0).unwrap();
    assert_eq!(field.status([2.1, 1.0, 0.0]), None);
    assert!(matches!(
        field.status([1.9, 1.0, 0.0]),
        Some(SplatReason::Electrode { .. })
    ));
    let hit = field
        .intersection([4.0, 1.0, 0.0], [0.0, 1.0, 0.0])
        .unwrap();
    assert!((hit.0 - 0.5).abs() < 1e-14);
    let mut thin = PotentialArray::new(9, 3, 1);
    thin.field_type = FieldType::Magnetic;
    for y in 0..3 {
        thin.electrodes[y * 9 + 3] = true;
        thin.potentials[y * 9 + 3] = 1000.0;
    }
    assert_eq!(
        MagneticPa::new(thin, 1.0).unwrap().status([3.1, 1.0, 0.0]),
        None
    );
}

#[test]
fn biot_savart_segment_loop_and_solenoid_limits() {
    assert_eq!(
        Coil::hoop([0.0; 3], [0.0, 0.0, 1.0], 1.0, 0.0)
            .unwrap()
            .field_gauss([1.0, 0.0, 0.0]),
        [0.0; 3]
    );
    let segment = Coil::segment([0.0, 0.0, -10.0], [0.0, 0.0, 10.0], 2.0).unwrap();
    assert!(
        difference(
            segment.field_gauss([1.0, 0.0, 0.0]),
            [0.0, 40.0 / 101.0_f64.sqrt(), 0.0]
        ) < 1e-14
    );
    let hoop = Coil::hoop([0.0; 3], [0.0, 0.0, 1.0], 30.0, 100.0).unwrap();
    assert!((hoop.field_gauss([0.0; 3])[2] - 2.0 * PI * 100.0 / 30.0).abs() < 1e-13);
    // Independent polygon Biot-Savart integral checks off-axis elliptic K/E.
    let p = [12.0, -8.0, 7.0];
    let mut polygon = [0.0; 3];
    for i in 0..4096 {
        let point = |i: usize| {
            let a = i as f64 * 2.0 * PI / 4096.0;
            [30.0 * a.cos(), 30.0 * a.sin(), 0.0]
        };
        let b = Coil::segment(point(i), point(i + 1), 100.0)
            .unwrap()
            .field_gauss(p);
        for a in 0..3 {
            polygon[a] += b[a];
        }
    }
    assert!(difference(hoop.field_gauss(p), polygon) / norm(polygon) < 4e-7);
    let solenoid =
        MagneticField::solenoid([-50.0, 0.0, 0.0], [50.0, 0.0, 0.0], 10.0, 0.7958, 100).unwrap();
    let b = solenoid.sample([0.0; 3], 0.0).magnetic_gauss;
    assert!((b[0] - 9.806136450441315).abs() < 2e-12);
    assert_eq!([b[1], b[2]], [0.0; 2]);
}

#[test]
fn uniform_magnetic_radius_boris_and_rk45_converge() {
    let field = MagneticField {
        uniform_gauss: [0.0, 0.0, 100.0],
        ..Default::default()
    };
    let ion = Particle::new(40.0, 1.0, [0.0; 3], [1.0, 0.0, 0.0], 0.0).unwrap();
    let omega = MAGNETIC_FREQUENCY * 100.0 / 40.0;
    let radius = 1.0 / omega;
    for integrator in [Integrator::Rk45, Integrator::Boris] {
        let result = fly(
            &field,
            &ion,
            &FlightOptions {
                integrator,
                duration_us: PI / omega,
                initial_step_us: 0.001,
                max_step_us: 0.05,
                ..Default::default()
            },
        )
        .unwrap();
        assert!((result.final_state.position_mm[1] / (-2.0 * radius) - 1.0).abs() < 1e-8);
        assert!((norm(result.final_state.velocity_mm_per_us) - 1.0).abs() < 2e-9);
    }
}

#[test]
fn magnetic_scalar_grid_converges_to_own_analytic_reference_through_eightfold_refinement() {
    let k = PI * 0.5;
    let exact = AnalyticField(|p: [f64; 3], _| FieldSample {
        magnetic_gauss: [
            100.0 * k * (k * p[0]).sin() * (k * p[1]).cosh(),
            -100.0 * k * (k * p[0]).cos() * (k * p[1]).sinh(),
            0.0,
        ],
        ..Default::default()
    });
    let ions: Vec<_> = [0.17, 0.23, 0.31]
        .into_iter()
        .map(|y| Particle::new(40.0, 1.0, [0.11, y, 0.0], [1.0, 0.0, 0.0], 0.0).unwrap())
        .collect();
    let options = FlightOptions {
        duration_us: 2.0,
        max_step_us: 0.01,
        relative_tolerance: 1e-12,
        position_tolerance_mm: 1e-12,
        velocity_tolerance_mm_per_us: 1e-12,
        stop_plane: Some(StopPlane {
            axis: 0,
            coordinate_mm: 0.9,
        }),
        ..Default::default()
    };
    let reference: Vec<_> = ions
        .iter()
        .map(|p| fly(&exact, p, &options).unwrap())
        .collect();
    let mut errors = Vec::new();
    for n in [9, 17, 33, 65] {
        let mut pa = PotentialArray::new(n, n, 1);
        pa.field_type = FieldType::Magnetic;
        pa.dx_mm = 1.0 / (n - 1) as f64;
        pa.dy_mm = pa.dx_mm;
        pa.mirrors.x = true;
        pa.mirrors.y = true;
        for y in 0..n {
            for x in 0..n {
                pa.potentials[y * n + x] =
                    (k * x as f64 * pa.dx_mm).cos() * (k * y as f64 * pa.dy_mm).cosh();
            }
        }
        let field = MagneticPa::new(pa, 100.0).unwrap();
        let squared: f64 = ions
            .iter()
            .zip(&reference)
            .map(|(p, r)| {
                let actual = fly(&field, p, &options).unwrap();
                assert_eq!(actual.splat_reason, SplatReason::StopPlane);
                difference(actual.final_state.position_mm, r.final_state.position_mm).powi(2)
            })
            .sum();
        let rms = (squared / ions.len() as f64).sqrt();
        println!("magnetic own analytic reference: grid={n}, RMS position error={rms:.12e} mm");
        errors.push(rms);
    }
    assert!(errors.windows(2).all(|e| e[1] < e[0] * 0.35), "{errors:?}");
    assert!(errors[3] < 1e-7, "{errors:?}");
}

#[test]
fn hs1_elastic_com_and_collision_rate_limits() {
    let hs = Hs1::default();
    let mean = (8.0 * 1.3806505e-23 * 273.0 / (PI * 4.0 * 1.6605402e-27)).sqrt() / 1000.0;
    assert!((hs.mean_relative_speed([0.0; 3]) / mean - 1.0).abs() < 1e-14);
    assert!((hs.mean_relative_speed([1e6, 0.0, 0.0]) / 1e6 - 1.0).abs() < 1e-11);
    assert!(hs.collision_rate_per_us([0.0; 3]) > 0.0);
    // With a negligibly cold gas and equal masses, mean outgoing speed^2 is
    // half the initial value for a uniform hard-sphere impact area.
    let cold = Hs1 {
        gas: Gas {
            temperature_k: 1e-12,
            mass_amu: 40.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut rng = Rng::new(723);
    let mean_energy: f64 = (0..50000)
        .map(|_| norm(cold.scatter(40.0, [1.0, 0.0, 0.0], &mut rng)).powi(2))
        .sum::<f64>()
        / 50000.0;
    assert!((mean_energy - 0.5).abs() < 0.004);
}

#[test]
fn hs1_seed_is_reproducible_and_parallel_order_independent() {
    let field = AnalyticField(|_, _| FieldSample::default());
    let model = CollisionModel::Hs1(Hs1::default());
    let ion = Particle::new(40.0, 1.0, [0.0; 3], [1.0, 0.0, 0.0], 8.0).unwrap();
    let options = FlightOptions {
        duration_us: 10.0,
        ..Default::default()
    };
    let particles = vec![ion; 32];
    let a = fly_many_collisional(&field, &particles, &options, &model, 15).unwrap();
    let b = fly_many_collisional(&field, &particles, &options, &model, 15).unwrap();
    for (a, b) in a.iter().zip(&b) {
        assert_eq!(a.final_state, b.final_state);
    }
    assert_ne!(a[0].final_state, a[1].final_state);
    let single = fly_collisional(&field, &particles[0], &options, &model, 15).unwrap();
    assert_eq!(single.final_state, a[0].final_state);
    assert_eq!(single.splat_reason, SplatReason::TimeLimit);
}

#[test]
fn neighboring_ion_streams_are_not_shifted_copies() {
    let mut first = Rng::new(15);
    let mut second = Rng::new(15_u64.wrapping_add(0x9e3779b97f4a7c15));
    first.uniform();
    assert_ne!(first.uniform(), second.uniform());
    for _ in 0..10000 {
        let u = first.uniform();
        assert!(u > 0.0 && u < 1.0);
    }
}

/// External SIMION dataset is intentionally not a repository dependency.
/// Run explicitly with SIMREP_MBMR=/path/to/collision_sds/mbmr.dat.
#[test]
fn sds_runtime_icdf_transport_and_exact_drift() {
    let Ok(path) = std::env::var("SIMREP_MBMR") else {
        return;
    };
    let statistics = DiffusionStatistics::from_path(path).unwrap();
    assert_eq!(statistics.quantile(2.0, 0.0), 0.0);
    let q1 = statistics.quantile(1.0, 0.5);
    let q2 = statistics.quantile(2.0, 0.5);
    assert!((statistics.quantile(1.5, 0.5) / (q1 * q2).sqrt() - 1.0).abs() < 1e-14);
    let sds = Sds {
        gas: Gas {
            pressure_pa: 760.0 * 133.322,
            temperature_k: 273.15,
            mass_amu: 28.94515,
            velocity_mm_per_us: [0.0; 3],
        },
        gas_diameter_nm: 0.366,
        ion_diameter_nm: Some(1.0),
        mobility_stp: Some(1.5),
        diffusion: false,
        statistics,
    };
    let transport = sds.transport(100.0, 1.0);
    let field = AnalyticField(|_, _| FieldSample {
        electric_v_per_mm: [0.1, 0.0, 0.0],
        ..Default::default()
    });
    let ion = Particle::new(100.0, 1.0, [0.0; 3], [0.0; 3], 0.0).unwrap();
    let h = 10.0;
    let expected_v = ELECTRIC_ACCELERATION * 0.1 / 100.0 / transport.damping_per_us;
    let model = CollisionModel::Sds(sds);
    let result = fly_collisional(
        &field,
        &ion,
        &FlightOptions {
            duration_us: h,
            max_step_us: h,
            initial_step_us: h,
            ..Default::default()
        },
        &model,
        2,
    )
    .unwrap();
    let decay = (-transport.damping_per_us * h).exp();
    assert!(
        (result.final_state.velocity_mm_per_us[0] / (expected_v * (1.0 - decay)) - 1.0).abs()
            < 1e-12
    );
    let expected_x = expected_v * (h - (1.0 - decay) / transport.damping_per_us);
    assert!((result.final_state.position_mm[0] / expected_x - 1.0).abs() < 1e-12);

    let CollisionModel::Sds(mut sds) = model else {
        unreachable!()
    };
    sds.gas.pressure_pa = 1.0;
    let transport = sds.transport(100.0, -1.0);
    let model = CollisionModel::Sds(sds);
    let field = MagneticField {
        uniform_gauss: [0.0, 0.0, 100.0],
        ..Default::default()
    };
    let ion = Particle::new(100.0, -1.0, [0.0; 3], [1.0, 0.0, 0.0], 0.0).unwrap();
    let time = 20.0;
    let result = fly_collisional(
        &field,
        &ion,
        &FlightOptions {
            duration_us: time,
            initial_step_us: time,
            max_step_us: time,
            ..Default::default()
        },
        &model,
        2,
    )
    .unwrap();
    let omega = -MAGNETIC_FREQUENCY;
    let damping = transport.damping_per_us;
    let decay = (-damping * time).exp();
    let expected_v = [
        decay * (omega * time).cos(),
        -decay * (omega * time).sin(),
        0.0,
    ];
    assert!(difference(result.final_state.velocity_mm_per_us, expected_v) < 1e-12);
    let denominator = damping * damping + omega * omega;
    let expected_p = [
        (damping + decay * (-damping * (omega * time).cos() + omega * (omega * time).sin()))
            / denominator,
        (decay * (damping * (omega * time).sin() + omega * (omega * time).cos()) - omega)
            / denominator,
        0.0,
    ];
    assert!(difference(result.final_state.position_mm, expected_p) < 1e-10);
}
