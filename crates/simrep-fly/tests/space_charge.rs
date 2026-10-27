use simrep_fly::{
    AnalyticField, ELECTRIC_ACCELERATION, Field, FieldSample, FlightOptions, Particle, SplatReason,
    StopPlane,
    physics::space_charge::{COULOMB_V_MM_PER_C, ELEMENTARY_CHARGE_C, EPSILON_0_F_PER_MM},
    physics::{ChargeMode, DirectCharge, EnsembleOptions, PicMesh, SpaceCharge, fly_ensemble},
};
use simrep_solve::mg::{Geometry, Grid, Options};

fn zero() -> AnalyticField<impl Fn([f64; 3], f64) -> FieldSample + Sync> {
    AnalyticField(|_, _| FieldSample::default())
}
fn particle(p: [f64; 3], v: [f64; 3], t: f64) -> Particle {
    Particle::new(1.0, 1.0, p, v, t).unwrap()
}
fn options(dt: f64, duration: f64, mode: ChargeMode) -> EnsembleOptions {
    EnsembleOptions {
        flight: FlightOptions {
            initial_step_us: dt,
            max_step_us: dt,
            duration_us: duration,
            relative_tolerance: 1e-7,
            position_tolerance_mm: 1e-9,
            velocity_tolerance_mm_per_us: 1e-9,
            ..FlightOptions::default()
        },
        charge_mode: mode,
        space_charge_step_fraction: 1.0,
        ..EnsembleOptions::default()
    }
}
fn point() -> SpaceCharge {
    SpaceCharge::Direct(DirectCharge::default())
}

#[test]
fn represented_charge_weights_and_coulomb_units() {
    let mut particles = vec![
        particle([-1.0, 0.0, 0.0], [0.0; 3], 0.0),
        particle([1.0, 0.0, 0.0], [0.0; 3], 0.0),
    ];
    particles[0].charge_e = -1.0;
    particles[1].charge_e = -2.0;
    let q = ChargeMode::Factor(1000.0)
        .charges(&particles, &[2.0, 3.0])
        .unwrap();
    assert!((q[0] / ELEMENTARY_CHARGE_C + 2000.0).abs() < 1e-10);
    assert!((q[1] / ELEMENTARY_CHARGE_C + 6000.0).abs() < 1e-10);
    let q = ChargeMode::Coulombic(1e-5)
        .charges(&particles, &[2.0, 3.0])
        .unwrap();
    assert!((q[0] + 2.5e-6).abs() < 1e-20);
    assert!((q[1] + 7.5e-6).abs() < 1e-20);
    let fields = DirectCharge::default()
        .field(&[[-1.0, 0.0, 0.0], [1.0, 0.0, 0.0]], &[1e-15, 1e-15])
        .unwrap();
    assert!((fields[0][0] + COULOMB_V_MM_PER_C * 1e-15 / 4.0).abs() < 1e-16);
    assert_eq!(fields[0][0], -fields[1][0]);
    simrep_core::validation::Gate {
        name: "exact-coulomb-point-charge",
        reference: "exact_solutions.json; Coulomb inverse-square field",
        metric: "electric field at 2 mm (V/mm)",
        value: fields[1][0],
        datum: COULOMB_V_MM_PER_C * 1e-15 / 4.0,
        u_num: None,
        u_input: 0.0,
        u_datum: 0.0,
        band: "existing absolute tolerance 1e-16 V/mm; direct algebra at roundoff; no GCI",
    }
    .record()
    .unwrap();
    assert!(
        DirectCharge::default()
            .field(&[[0.0; 3]; 2], &[1e-15; 2])
            .is_err()
    );
}

#[test]
fn direct_small_and_parallel_populations_keep_source_order() {
    let positions: Vec<_> = (0..257)
        .map(|i| [i as f64 * 0.03, (i % 7) as f64 * 0.2, 0.1])
        .collect();
    let charges: Vec<_> = (0..257)
        .map(|i| {
            if i < 103 {
                (i as f64 - 51.0) * 1e-17
            } else {
                0.0
            }
        })
        .collect();
    for softening_mm in [0.0, 0.01] {
        let direct = DirectCharge { softening_mm };
        let small = direct.field(&positions[..103], &charges[..103]).unwrap();
        for threads in [1, 4] {
            let pool = rayon::ThreadPoolBuilder::new()
                .num_threads(threads)
                .build()
                .unwrap();
            for n in [255, 256, 257] {
                let fields = pool
                    .install(|| direct.field(&positions[..n], &charges[..n]))
                    .unwrap();
                assert_eq!(small, fields[..103]);
            }
        }
    }
}

/// Cold spherical envelope R''=C/R², R(0)=1, R'(0)=0, implicit exact solution.
fn sphere_radius(c: f64, t: f64) -> f64 {
    let mut lo = 1.0;
    let mut hi = 1.0 + c * t * t;
    for _ in 0..80 {
        let r: f64 = 0.5 * (lo + hi);
        let time = ((r * (r - 1.0)).sqrt() + (r - 1.0).sqrt().asinh()) / (2.0 * c).sqrt();
        if time < t {
            lo = r;
        } else {
            hi = r;
        }
    }
    0.5 * (lo + hi)
}

#[test]
fn direct_second_order_against_exact_two_body_expansion() {
    let particles = [
        particle([-1.0, 0.0, 0.0], [0.0; 3], 0.0),
        particle([1.0, 0.0, 0.0], [0.0; 3], 0.0),
    ];
    let total = 2e-14;
    let expected = sphere_radius(
        ELECTRIC_ACCELERATION * COULOMB_V_MM_PER_C * total / 8.0,
        0.7,
    );
    let mut errors = Vec::new();
    let mut values = Vec::new();
    for dt in [0.04, 0.02, 0.01] {
        let report = fly_ensemble(
            &zero(),
            &particles,
            &options(dt, 0.7, ChargeMode::Coulombic(total)),
            &mut point(),
        )
        .unwrap();
        errors.push((report.results[1].final_state.position_mm[0] - expected).abs());
        values.push(report.results[1].final_state.position_mm[0]);
        assert_eq!(
            report.results[0].final_state.position_mm[0],
            -report.results[1].final_state.position_mm[0]
        );
    }
    eprintln!("two-body dt halving errors: {errors:?}");
    assert!(errors[0] / errors[1] > 3.7 && errors[1] / errors[2] > 3.7);
    let levels = std::array::from_fn(|i| simrep_core::uncertainty::Level {
        h: 0.01 * 2.0_f64.powi(i as i32),
        value: values[2 - i],
    });
    let triplet = simrep_core::uncertainty::observed_order(&levels).unwrap();
    println!("two-body radius: S={:.12e}, D={expected:.12e}", values[2]);
    simrep_core::validation::Gate {
        name: "exact-coulomb-two-body",
        reference: "exact_solutions.json; implicit two-body expansion",
        metric: "half separation (mm)",
        value: values[2],
        datum: expected,
        u_num: triplet.gci_fine.map(|g| g * values[2].abs()),
        u_input: 0.0,
        u_datum: 0.0,
        band: "existing dt=0.04,0.02,0.01 us; three-step Celik GCI; adjacent order > log2(3.7)",
    }
    .record()
    .unwrap();
}

#[test]
fn births_splats_and_source_removal_are_time_ordered() {
    let particles = [
        particle([0.0, 0.0, 0.0], [1.0, 0.0, 0.0], 0.0),
        particle([0.0, 2.0, 0.0], [1.0, 0.0, 0.0], 0.37),
    ];
    let mut o = options(0.2, 1.0, ChargeMode::Factor(0.0));
    o.flight.stop_plane = Some(StopPlane {
        axis: 0,
        coordinate_mm: 0.5,
    });
    let report = fly_ensemble(&zero(), &particles, &o, &mut point()).unwrap();
    for r in report.results {
        assert_eq!(r.splat_reason, SplatReason::StopPlane);
        assert!((r.tof_us - 0.5).abs() < 1e-14);
        assert_eq!(r.final_state.position_mm[0], 0.5);
    }
    let particles = [
        particle([-1.0, 0.0, 0.0], [0.0; 3], 0.0),
        particle([1.0, 0.0, 0.0], [0.0; 3], 0.5),
    ];
    let o = options(0.1, 0.5, ChargeMode::Coulombic(2e-14));
    let report = fly_ensemble(&zero(), &particles, &o, &mut point()).unwrap();
    // The first source expires exactly when the second is born: no interaction.
    assert_eq!(report.results[0].final_state.position_mm, [-1.0, 0.0, 0.0]);
    assert_eq!(report.results[1].final_state.position_mm, [1.0, 0.0, 0.0]);
}

#[test]
fn ensemble_adapts_to_external_field_error() {
    let field = AnalyticField(|p: [f64; 3], _| FieldSample {
        electric_v_per_mm: [-p[0] / ELECTRIC_ACCELERATION, 0.0, 0.0],
        ..FieldSample::default()
    });
    let mut o = options(1.0, 1.0, ChargeMode::Factor(0.0));
    o.flight.relative_tolerance = 1e-10;
    o.flight.position_tolerance_mm = 1e-10;
    o.flight.velocity_tolerance_mm_per_us = 1e-10;
    let report = fly_ensemble(
        &field,
        &[particle([1.0, 0.0, 0.0], [0.0; 3], 0.0)],
        &o,
        &mut point(),
    )
    .unwrap();
    assert!(report.rejected_steps > 0);
    assert!((report.results[0].final_state.position_mm[0] - 1.0_f64.cos()).abs() < 5e-7);
    assert!((report.results[0].final_state.velocity_mm_per_us[0] + 1.0_f64.sin()).abs() < 5e-7);
}

#[test]
fn coupled_push_keeps_its_one_cell_ceiling_when_independent_defaults_are_looser() {
    struct GridScale;
    impl Field for GridScale {
        fn sample(&self, _: [f64; 3], _: f64) -> FieldSample {
            FieldSample::default()
        }
        fn spatial_scale_mm(&self) -> Option<f64> {
            Some(0.25)
        }
    }
    let particles = [
        particle([0.0, -1.0, 0.0], [10.0, 0.0, 0.0], 0.0),
        particle([0.0, 1.0, 0.0], [10.0, 0.0, 0.0], 0.0),
    ];
    let o = EnsembleOptions {
        flight: FlightOptions {
            duration_us: 1.0,
            ..Default::default()
        },
        charge_mode: ChargeMode::Factor(1e8),
        ..Default::default()
    };
    let actual = fly_ensemble(&GridScale, &particles, &o, &mut point()).unwrap();
    let mut previous = o;
    previous.flight.step_limits.max_grid_cells = 1.0;
    let reference = fly_ensemble(&GridScale, &particles, &previous, &mut point()).unwrap();
    assert!(actual.global_steps >= 40);
    assert_eq!(actual.global_steps, reference.global_steps);
    assert_eq!(actual.rejected_steps, reference.rejected_steps);
    for (actual, reference) in actual.results.iter().zip(&reference.results) {
        assert_eq!(actual.final_state, reference.final_state);
    }
    assert!(actual.results[1].final_state.position_mm[1] > 1.0);
}

#[test]
fn sub_ulp_crossing_does_not_underflow_the_global_clock() {
    let mut o = options(0.1, 1.0, ChargeMode::Factor(0.0));
    o.flight.stop_plane = Some(StopPlane {
        axis: 0,
        coordinate_mm: 1.0,
    });
    let report = fly_ensemble(
        &zero(),
        &[particle(
            [1.0_f64.next_down(), 0.0, 0.0],
            [1000.0, 0.0, 0.0],
            0.1,
        )],
        &o,
        &mut point(),
    )
    .unwrap();
    assert_eq!(report.results[0].splat_reason, SplatReason::StopPlane);
    assert_eq!(report.results[0].final_state.position_mm[0], 1.0);
    assert!(report.results[0].tof_us < 1e-15);
}

fn cube(n: usize, length: f64) -> PicMesh {
    let grid = Grid {
        dims: [n; 3],
        spacing: [length / (n - 1) as f64; 3],
        geometry: Geometry::Planar,
    };
    let fixed = (0..n * n * n)
        .map(|i| {
            let c = [i % n, (i / n) % n, i / (n * n)];
            c.into_iter().any(|v| v == 0 || v + 1 == n)
        })
        .collect();
    PicMesh::new(
        grid,
        [-length / 2.0; 3],
        fixed,
        Options {
            relative_tolerance: 1e-8,
            ..Options::default()
        },
    )
    .unwrap()
}

#[test]
fn cic_charge_conservation_cylindrical_jacobian_and_self_force() {
    let mut mesh = cube(17, 8.0);
    let p = [[0.1, 0.2, 0.3]];
    let rho = mesh.deposit(&p, &[1e-14]).unwrap();
    let sum: f64 = rho
        .iter()
        .enumerate()
        .map(|(i, r)| r * mesh.volume_mm3(i))
        .sum();
    assert!((sum / 1e-14 - 1.0).abs() < 1e-14);
    let e = mesh.field(&[[0.0; 3]], &[1e-14], &[1.0]).unwrap()[0];
    assert!(e.into_iter().all(|v| v.abs() < 1e-12), "{e:?}");
    let n = 33;
    let grid = Grid {
        dims: [n, 9, 1],
        spacing: [0.125, 0.25, 1.0],
        geometry: Geometry::Cylindrical { radial_origin: 0.0 },
    };
    let fixed = (0..n * 9).map(|i| i % n == n - 1).collect();
    let mesh = PicMesh::new(grid, [0.0; 3], fixed, Options::default()).unwrap();
    let rho = mesh
        .deposit(&[[0.0, 0.0, 0.7], [0.6, 0.8, 1.3]], &[1e-14, 2e-14])
        .unwrap();
    let sum: f64 = rho
        .iter()
        .enumerate()
        .map(|(i, r)| r * mesh.volume_mm3(i))
        .sum();
    assert!((sum / 3e-14 - 1.0).abs() < 1e-14);
    assert!((mesh.volume_mm3(0) - std::f64::consts::PI * 0.0625_f64.powi(2) * 0.125).abs() < 1e-16);
}

#[test]
fn pic_second_order_against_its_converged_time_reference() {
    let particles = [
        particle([-0.7, 0.0, 0.0], [0.0; 3], 0.0),
        particle([0.7, 0.0, 0.0], [0.0; 3], 0.0),
    ];
    let run = |dt| {
        let mut pic = SpaceCharge::Pic(Box::new(cube(17, 8.0)));
        fly_ensemble(
            &zero(),
            &particles,
            &options(dt, 0.16, ChargeMode::Coulombic(2e-15)),
            &mut pic,
        )
        .unwrap()
        .results[1]
            .final_state
            .position_mm[0]
    };
    let reference = run(0.00125);
    let errors: Vec<_> = [0.04, 0.02, 0.01]
        .into_iter()
        .map(|dt| (run(dt) - reference).abs())
        .collect();
    eprintln!("PIC dt halving errors: {errors:?}");
    assert!(errors[0] / errors[1] > 3.7 && errors[1] / errors[2] > 3.7);
}

fn sphere_particles() -> Vec<Particle> {
    let mut particles = Vec::new();
    for z in -4..=4 {
        for y in -4..=4 {
            for x in -4..=4 {
                if x * x + y * y + z * z <= 16 {
                    particles.push(particle(
                        [x as f64 / 4.0, y as f64 / 4.0, z as f64 / 4.0],
                        [0.0; 3],
                        0.0,
                    ));
                }
            }
        }
    }
    particles
}
fn rms_radius(states: impl Iterator<Item = [f64; 3]>) -> f64 {
    let r: Vec<_> = states
        .map(|p| p.into_iter().map(|v| v * v).sum::<f64>())
        .collect();
    (r.iter().sum::<f64>() / r.len() as f64).sqrt()
}

#[test]
fn uniformly_charged_sphere_envelope_and_direct_pic_agreement() {
    // Debug builds fly half the duration so the debug suite stays fast;
    // release keeps the full 0.3 us. The time step, mesh and particle cloud
    // are unchanged, and the analytic envelope is evaluated at the same
    // duration, so every assertion compares the same quantities.
    #[cfg(debug_assertions)]
    const DURATION: f64 = 0.15;
    #[cfg(not(debug_assertions))]
    const DURATION: f64 = 0.3;
    let particles = sphere_particles();
    let initial = rms_radius(particles.iter().map(|p| p.initial.position_mm));
    let q = 1e-14;
    let o = options(0.01, DURATION, ChargeMode::Coulombic(q));
    let direct = fly_ensemble(&zero(), &particles, &o, &mut point()).unwrap();
    let pic = fly_ensemble(
        &zero(),
        &particles,
        &o,
        &mut SpaceCharge::Pic(Box::new(cube(33, 8.0))),
    )
    .unwrap();
    let direct_radius =
        rms_radius(direct.results.iter().map(|r| r.final_state.position_mm)) / initial;
    let pic_radius = rms_radius(pic.results.iter().map(|r| r.final_state.position_mm)) / initial;
    let expected = sphere_radius(ELECTRIC_ACCELERATION * COULOMB_V_MM_PER_C * q, DURATION);
    eprintln!(
        "sphere analytic={expected:.8}, direct={direct_radius:.8}, PIC={pic_radius:.8}, N={}",
        particles.len()
    );
    assert!((direct_radius / expected - 1.0).abs() < 0.04);
    assert!((pic_radius / expected - 1.0).abs() < 0.04);
    assert!((pic_radius / direct_radius - 1.0).abs() < 0.025);
}

// Independent RK4 solve of the analytic uniform-cylinder envelope R''=C/R.
fn cylinder_radius(c: f64, duration: f64) -> f64 {
    let h = duration / 8192.0;
    let (mut r, mut v) = (1.0, 0.0);
    for _ in 0..8192 {
        let a = c / r;
        let v2 = v + 0.5 * h * a;
        let r2 = r + 0.5 * h * v;
        let v3 = v + 0.5 * h * c / r2;
        let r3 = r + 0.5 * h * v2;
        let v4 = v + h * c / r3;
        let r4 = r + h * v3;
        r += h * (v + 2.0 * v2 + 2.0 * v3 + v4) / 6.0;
        v += h * (a + 2.0 * c / r2 + 2.0 * c / r3 + c / r4) / 6.0;
    }
    r
}

#[test]
fn uniform_cylinder_pic_expansion_matches_envelope() {
    let grid = Grid {
        dims: [65, 9, 1],
        spacing: [0.0625, 0.25, 1.0],
        geometry: Geometry::Cylindrical { radial_origin: 0.0 },
    };
    let fixed = (0..65 * 9).map(|i| i % 65 == 64).collect();
    let mut pic = SpaceCharge::Pic(Box::new(
        PicMesh::new(
            grid,
            [0.0; 3],
            fixed,
            Options {
                relative_tolerance: 1e-9,
                ..Options::default()
            },
        )
        .unwrap(),
    ));
    let mut particles = Vec::new();
    let mut weights = Vec::new();
    for z in 0..9 {
        for k in 0..16 {
            for a in 0..4 {
                let r = (k as f64 + 0.5) / 16.0;
                let theta = a as f64 * std::f64::consts::FRAC_PI_2;
                particles.push(particle(
                    [r * theta.cos(), r * theta.sin(), z as f64 * 0.25],
                    [0.0; 3],
                    0.0,
                ));
                weights.push(r * if z == 0 || z == 8 { 0.5 } else { 1.0 });
            }
        }
    }
    let q = 1e-14;
    let mut o = options(0.005, 0.2, ChargeMode::Coulombic(q));
    o.charge_weights = weights.clone();
    let initial: f64 = particles
        .iter()
        .zip(&weights)
        .map(|(p, w)| w * (p.initial.position_mm[0].powi(2) + p.initial.position_mm[1].powi(2)))
        .sum();
    let report = fly_ensemble(&zero(), &particles, &o, &mut pic).unwrap();
    let final_r: f64 = report
        .results
        .iter()
        .zip(&weights)
        .map(|(p, w)| {
            w * (p.final_state.position_mm[0].powi(2) + p.final_state.position_mm[1].powi(2))
        })
        .sum();
    let radius = (final_r / initial).sqrt();
    let expected = cylinder_radius(
        ELECTRIC_ACCELERATION * q / 2.0 / (2.0 * std::f64::consts::PI * EPSILON_0_F_PER_MM),
        0.2,
    );
    eprintln!(
        "cylinder analytic={expected:.8}, PIC={radius:.8}, N={}",
        particles.len()
    );
    assert!((radius / expected - 1.0).abs() < 0.005);
}
