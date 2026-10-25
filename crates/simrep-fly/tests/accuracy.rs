use std::f64::consts::PI;

use simrep_core::{MirrorFlags, PotentialArray, Symmetry};
use simrep_fly::{
    AnalyticField, BasisPotential, ELECTRIC_ACCELERATION, Field, FieldSample, FlightOptions,
    GridField, Integrator, MAGNETIC_FREQUENCY, Particle, Reconstruction, SPEED_OF_LIGHT,
    SplatReason, StopPlane, Superposition, fly, fly_many,
};

fn distance(a: [f64; 3], b: [f64; 3]) -> f64 {
    a.into_iter()
        .zip(b)
        .map(|(a, b)| (a - b).powi(2))
        .sum::<f64>()
        .sqrt()
}

fn grid(
    nx: usize,
    ny: usize,
    nz: usize,
    h: [f64; 3],
    potential: impl Fn([f64; 3]) -> f64,
) -> PotentialArray {
    let mut pa = PotentialArray::new(nx, ny, nz);
    [pa.dx_mm, pa.dy_mm, pa.dz_mm] = h;
    for z in 0..nz {
        for y in 0..ny {
            for x in 0..nx {
                let i = pa.index(x, y, z).unwrap();
                pa.potentials[i] = potential([x as f64 * h[0], y as f64 * h[1], z as f64 * h[2]]);
            }
        }
    }
    pa
}

#[test]
fn parallel_plate_tof_and_birth_time() {
    let field = AnalyticField(|_, _| FieldSample {
        electric_v_per_mm: [2.0, 0.0, 0.0],
        ..Default::default()
    });
    let ion = Particle::from_energy(40.0, 1.0, [0.0; 3], 0.0, [1.0, 0.0, 0.0], 7.25).unwrap();
    let expected = (20.0 / (ELECTRIC_ACCELERATION * 2.0 / 40.0)).sqrt();
    let result = fly(
        &field,
        &ion,
        &FlightOptions {
            duration_us: expected * 2.0,
            initial_step_us: expected * 0.4,
            max_step_us: expected,
            stop_plane: Some(StopPlane {
                axis: 0,
                coordinate_mm: 10.0,
            }),
            sample_every: Some(1),
            ..Default::default()
        },
    )
    .unwrap();
    let relative = (result.tof_us / expected - 1.0).abs();
    println!(
        "parallel plates: TOF {:.15e} us, relative error {relative:.3e}",
        result.tof_us
    );
    assert_eq!(result.splat_reason, SplatReason::StopPlane);
    assert!(relative < 1e-12);
    assert!((result.final_state.position_mm[0] - 10.0).abs() < 1e-11);
    assert_eq!(result.final_state.time_us, 7.25 + result.tof_us);
    let samples = result.trajectory.unwrap();
    assert_eq!(samples.first(), Some(&ion.initial));
    assert_eq!(samples.last(), Some(&result.final_state));
    assert!(samples.windows(2).all(|p| p[0].time_us < p[1].time_us));
}

#[test]
fn harmonic_oscillation_frequency_and_long_flight() {
    let omega: f64 = 0.8;
    let field = AnalyticField(|p: [f64; 3], _| FieldSample {
        potential_v: omega.powi(2) * p[0] * p[0] / (2.0 * ELECTRIC_ACCELERATION),
        electric_v_per_mm: [-omega.powi(2) * p[0] / ELECTRIC_ACCELERATION, 0.0, 0.0],
        ..Default::default()
    });
    let ion = Particle::new(1.0, 1.0, [1.0, 0.0, 0.0], [0.0; 3], 0.0).unwrap();
    let period = 2.0 * PI / omega;
    let options = FlightOptions {
        duration_us: period,
        max_step_us: period,
        stop_plane: Some(StopPlane {
            axis: 0,
            coordinate_mm: 0.0,
        }),
        ..Default::default()
    };
    let result = fly(&field, &ion, &options).unwrap();
    let frequency_error = ((PI / (2.0 * result.tof_us)) / omega - 1.0).abs();
    println!("harmonic frequency: relative error {frequency_error:.3e}");
    assert_eq!(result.splat_reason, SplatReason::StopPlane);
    assert!(frequency_error < 1e-9);
    let long = fly(
        &field,
        &ion,
        &FlightOptions {
            duration_us: 50.0 * period,
            stop_plane: None,
            ..options
        },
    )
    .unwrap();
    assert_eq!(long.splat_reason, SplatReason::TimeLimit);
    assert!(distance(long.final_state.position_mm, ion.initial.position_mm) < 2e-8);
    assert!(distance(long.final_state.velocity_mm_per_us, [0.0; 3]) < 2e-8);
}

#[test]
fn boris_cyclotron_radius_period_and_1000_orbit_energy() {
    let b = 100_000.0;
    let omega = MAGNETIC_FREQUENCY * b / 40.0;
    let period = 2.0 * PI / omega;
    let speed = 100.0;
    let radius = speed / omega;
    let field = AnalyticField(|_, _| FieldSample {
        magnetic_gauss: [0.0, 0.0, b],
        ..Default::default()
    });
    let ion = Particle::new(40.0, 1.0, [0.0; 3], [speed, 0.0, 0.0], 0.0).unwrap();
    let short = fly(
        &field,
        &ion,
        &FlightOptions {
            integrator: Integrator::Boris,
            duration_us: period,
            initial_step_us: period / 20_000.0,
            max_step_us: period,
            max_steps: 20_002,
            sample_every: Some(1_000),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(short.splat_reason, SplatReason::TimeLimit);
    let phase_error = distance(short.final_state.position_mm, [0.0; 3]) / radius;
    let period_error = phase_error / (2.0 * PI);
    assert!(period_error < 1e-7);
    let long = fly(
        &field,
        &ion,
        &FlightOptions {
            integrator: Integrator::Boris,
            duration_us: 1000.0 * period,
            initial_step_us: period / 1_000.0,
            max_step_us: period,
            max_steps: 1_000_002,
            sample_every: Some(1_000),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(long.splat_reason, SplatReason::TimeLimit);
    let mut max_energy_error: f64 = 0.0;
    let mut max_radius_error: f64 = 0.0;
    for state in long.trajectory.unwrap() {
        let speed2 = state
            .velocity_mm_per_us
            .into_iter()
            .map(|v| v * v)
            .sum::<f64>();
        max_energy_error = max_energy_error.max((speed2 / speed.powi(2) - 1.0).abs());
        let r = state.position_mm[0].hypot(state.position_mm[1] + radius);
        max_radius_error = max_radius_error.max((r / radius - 1.0).abs());
    }
    println!(
        "Boris: 1000 orbits, max relative energy {max_energy_error:.3e}, radius {max_radius_error:.3e}, one-orbit period {period_error:.3e}"
    );
    assert!(max_energy_error < 1e-10);
    assert!(max_radius_error < 1e-10);
    // Compensated accumulation also finishes exactly at the requested duration.
    assert_eq!(long.tof_us, 1000.0 * period);
}

#[test]
fn cubic_grid_is_interpolating_and_field_is_c1_at_cell_faces() {
    let potential =
        |p: [f64; 3]| 1.0 + p[0].powi(3) + 2.0 * p[1].powi(2) + 3.0 * p[2] + p[0] * p[1] * p[2];
    let field = GridField::new(grid(8, 7, 6, [0.3, 0.4, 0.7], potential)).unwrap();
    let p = [0.83, 1.31, 2.42];
    let s = field.sample(p, 0.0);
    let expected = [
        -3.0 * p[0] * p[0] - p[1] * p[2],
        -4.0 * p[1] - p[0] * p[2],
        -3.0 - p[0] * p[1],
    ];
    assert!((s.potential_v - potential(p)).abs() < 1e-11);
    assert!(distance(s.electric_v_per_mm, expected) < 1e-11);
    for z in 0..6 {
        for y in 0..7 {
            for x in 0..8 {
                let p = [x as f64 * 0.3, y as f64 * 0.4, z as f64 * 0.7];
                assert!((field.sample(p, 0.0).potential_v - potential(p)).abs() < 1e-11);
            }
        }
    }
    // Non-polynomial data ensures continuity is actually imposed by the spline,
    // rather than passing accidentally because every cell is the same cubic.
    let f = GridField::new(grid(12, 11, 10, [0.3, 0.4, 0.7], |p| {
        (p[0] + p[1] * p[2]).sin()
    }))
    .unwrap();
    let eps = 1e-6;
    for axis in 0..3 {
        let mut p = [1.2, 1.6, 2.8];
        let center = f.sample(p, 0.0).electric_v_per_mm;
        p[axis] -= eps;
        let left = f.sample(p, 0.0).electric_v_per_mm;
        p[axis] += 2.0 * eps;
        let right = f.sample(p, 0.0).electric_v_per_mm;
        assert!(distance(left, right) < 2e-4);
        let dl = std::array::from_fn(|a| (center[a] - left[a]) / eps);
        let dr = std::array::from_fn(|a| (right[a] - center[a]) / eps);
        assert!(distance(dl, dr) < 2e-4, "axis {axis}: {dl:?} != {dr:?}");
    }
}

#[test]
fn vacuum_boundary_derivatives_exclude_flat_metal_interior() {
    let potential = |p: [f64; 3]| {
        let x = p[0] - 5.0;
        x * (2.0 + 0.03 * p[1].powi(2) + 0.02 * p[2].powi(2)) + x.powi(3)
    };
    let mut pa = grid(9, 8, 7, [1.0; 3], potential);
    for z in 0..pa.nz {
        for y in 0..pa.ny {
            for x in 5..pa.nx {
                let i = pa.index(x, y, z).unwrap();
                pa.electrodes[i] = true;
                pa.potentials[i] = 0.0;
            }
        }
    }
    let legacy = GridField::with_reconstruction(pa.clone(), Reconstruction::GlobalCubic).unwrap();
    let field = GridField::new(pa).unwrap();
    for x in [0.0, 1.23, 3.12, 4.3, 5.0] {
        let p = [x, 2.37, 3.16];
        let sample = field.sample(p, 0.0);
        let expected = [
            -2.0 - 0.03 * p[1].powi(2) - 0.02 * p[2].powi(2) - 3.0 * (x - 5.0).powi(2),
            -0.06 * p[1] * (x - 5.0),
            -0.04 * p[2] * (x - 5.0),
        ];
        assert!((sample.potential_v - potential(p)).abs() < 1e-11);
        assert!(distance(sample.electric_v_per_mm, expected) < 1e-11);
    }
    let p = [4.3, 2.37, 3.16];
    assert!(
        distance(
            field.sample(p, 0.0).electric_v_per_mm,
            legacy.sample(p, 0.0).electric_v_per_mm
        ) > 0.01
    );
}

#[test]
fn boundary_corner_patches_share_c1_fields_and_interpolate_nodes() {
    let mut pa = grid(10, 9, 1, [1.0; 3], |p| {
        (0.3 * p[0] + 0.1 * p[1].powi(2)).sin()
    });
    for y in 6..pa.ny {
        for x in 5..pa.nx {
            let i = pa.index(x, y, 0).unwrap();
            pa.electrodes[i] = true;
            pa.potentials[i] = 0.0;
        }
    }
    let field = GridField::new(pa.clone()).unwrap();
    for y in 0..pa.ny {
        for x in 0..pa.nx {
            let i = pa.index(x, y, 0).unwrap();
            assert!(
                (field.sample([x as f64, y as f64, 0.0], 0.0).potential_v - pa.potentials[i]).abs()
                    < 1e-11
            );
        }
    }
    let eps = 1e-6;
    for (axis, mut p) in [
        (0, [5.0, 5.2, 0.0]),
        (1, [4.2, 6.0, 0.0]),
        (0, [4.0, 3.6, 0.0]),
    ] {
        let center = field.sample(p, 0.0).electric_v_per_mm;
        p[axis] -= eps;
        let left = field.sample(p, 0.0).electric_v_per_mm;
        p[axis] += 2.0 * eps;
        let right = field.sample(p, 0.0).electric_v_per_mm;
        assert!(distance(left, right) < 2e-4);
        let dl = std::array::from_fn(|a| (center[a] - left[a]) / eps);
        let dr = std::array::from_fn(|a| (right[a] - center[a]) / eps);
        assert!(distance(dl, dr) < 2e-4, "axis {axis}: {dl:?} != {dr:?}");
    }
}

#[test]
fn vacuum_boundary_reconstruction_preserves_axis_and_reflection_conditions() {
    let mut pa = grid(8, 9, 1, [1.0; 3], |p| {
        (1.0 + p[0].powi(2)) * (p[1].powi(2) - 36.0)
    });
    pa.symmetry = Symmetry::Cylindrical;
    pa.mirrors.x = true;
    for y in 6..pa.ny {
        for x in 0..pa.nx {
            let i = pa.index(x, y, 0).unwrap();
            pa.electrodes[i] = true;
            pa.potentials[i] = 0.0;
        }
    }
    let field = GridField::new(pa).unwrap();
    let sample = field.sample([-1.4, 1.2, 1.6], 0.0);
    assert!((sample.potential_v + 94.72).abs() < 1e-11);
    assert!(distance(sample.electric_v_per_mm, [-89.6, -7.104, -9.472]) < 1e-11);
    assert_eq!(
        &field.sample([1.2, 0.0, 0.0], 0.0).electric_v_per_mm[1..],
        &[0.0, 0.0]
    );
    let mut odd = grid(9, 1, 1, [1.0; 3], |p| p[0].powi(3));
    odd.mirrors.x = true;
    odd.anti_mirrors.x = true;
    for x in 6..odd.nx {
        odd.electrodes[x] = true;
        odd.potentials[x] = 216.0;
    }
    let odd = GridField::new(odd).unwrap();
    let sample = odd.sample([-2.3, 17.0, -90.0], 0.0);
    assert!((sample.potential_v + 12.167).abs() < 1e-11);
    assert!(distance(sample.electric_v_per_mm, [-15.87, 0.0, 0.0]) < 1e-11);
}

#[test]
fn three_dimensional_voxel_corner_patches_share_c1_fields() {
    let mut pa = grid(8, 9, 10, [1.0; 3], |p| {
        (0.2 * p[0] + 0.3 * p[1] + 0.1 * p[2].powi(2)).sin()
    });
    for z in 4..pa.nz {
        for y in 6..pa.ny {
            for x in 5..pa.nx {
                let i = pa.index(x, y, z).unwrap();
                pa.electrodes[i] = true;
                pa.potentials[i] = 0.0;
            }
        }
    }
    let field = GridField::new(pa.clone()).unwrap();
    for z in 0..pa.nz {
        for y in 0..pa.ny {
            for x in 0..pa.nx {
                let p = [x as f64, y as f64, z as f64];
                let i = pa.index(x, y, z).unwrap();
                assert!((field.sample(p, 0.0).potential_v - pa.potentials[i]).abs() < 1e-11);
            }
        }
    }
    let eps = 1e-6;
    for (axis, mut p) in [
        (0, [5.0, 5.2, 3.6]),
        (1, [4.2, 6.0, 4.3]),
        (2, [4.2, 6.3, 4.0]),
    ] {
        let center = field.sample(p, 0.0).electric_v_per_mm;
        p[axis] -= eps;
        let left = field.sample(p, 0.0).electric_v_per_mm;
        p[axis] += 2.0 * eps;
        let right = field.sample(p, 0.0).electric_v_per_mm;
        assert!(distance(left, right) < 2e-4);
        let dl = std::array::from_fn(|a| (center[a] - left[a]) / eps);
        let dr = std::array::from_fn(|a| (right[a] - center[a]) / eps);
        assert!(distance(dl, dr) < 2e-4, "axis {axis}: {dl:?} != {dr:?}");
    }
}

#[test]
fn three_dimensional_electrode_tips_preserve_vacuum_mirror_normals() {
    let mut pa = grid(10, 9, 10, [1.0; 3], |p| {
        0.1 * (p[1].powi(2) - p[2].powi(2)) + (0.2 * p[0]).sin()
    });
    pa.mirrors.y = true;
    for z in 0..pa.nz {
        for y in 0..pa.ny {
            for x in 3..7 {
                if (y as f64).powi(2) + (z as f64 - 6.0).powi(2) <= 4.0 {
                    let i = pa.index(x, y, z).unwrap();
                    pa.electrodes[i] = true;
                    pa.potentials[i] = 2.0;
                }
            }
        }
    }
    for shift in 0..3 {
        let dims = [pa.nx, pa.ny, pa.nz];
        let rotated_dims: [usize; 3] = std::array::from_fn(|a| dims[(a + shift) % 3]);
        let mut rotated = PotentialArray::new(rotated_dims[0], rotated_dims[1], rotated_dims[2]);
        let normal_axis = (4 - shift) % 3;
        rotated.mirrors = MirrorFlags {
            x: normal_axis == 0,
            y: normal_axis == 1,
            z: normal_axis == 2,
        };
        for z in 0..pa.nz {
            for y in 0..pa.ny {
                for x in 0..pa.nx {
                    let old = [x, y, z];
                    let p: [usize; 3] = std::array::from_fn(|a| old[(a + shift) % 3]);
                    let i = rotated.index(p[0], p[1], p[2]).unwrap();
                    let j = pa.index(x, y, z).unwrap();
                    rotated.potentials[i] = pa.potentials[j];
                    rotated.electrodes[i] = pa.electrodes[j];
                }
            }
        }
        let compact = GridField::new_compact(rotated.clone()).unwrap();
        let field = GridField::new(rotated).unwrap();
        for x in [2.1, 3.4, 5.7, 7.2] {
            for z in [2.3, 3.2, 4.3, 8.7] {
                let old = [x, 0.0, z];
                let p = std::array::from_fn(|a| old[(a + shift) % 3]);
                assert_eq!(field.status(p), None);
                let sample = field.sample(p, 0.0);
                assert_eq!(sample, compact.sample(p, 0.0));
                let normal = sample.electric_v_per_mm[normal_axis];
                assert!(normal.abs() < 1e-11, "{p:?}: mirror normal {normal}");
                for offset in [-0.37, 0.37] {
                    let mut reflected = p;
                    reflected[normal_axis] = offset;
                    assert_eq!(field.sample(reflected, 0.0), compact.sample(reflected, 0.0));
                }
            }
        }
    }
}

#[test]
fn planar_invariance_cylindrical_rotation_mirrors_and_anti_mirrors() {
    let mut planar = grid(9, 8, 1, [0.2, 0.3, 1.0], |p| {
        p[0].powi(2) + 2.0 * p[1].powi(2)
    });
    planar.mirrors = MirrorFlags {
        x: true,
        y: true,
        z: false,
    };
    let planar = GridField::new(planar).unwrap();
    let s = planar.sample([-0.47, -0.81, 123.0], 0.0);
    assert!(distance(s.electric_v_per_mm, [0.94, 3.24, 0.0]) < 1e-12);
    assert_eq!(planar.status([-0.47, -0.81, -500.0]), None);
    let eps = 1e-7;
    for axis in 0..2 {
        let mut p = [0.47, 0.81, 0.0];
        p[axis] = 0.0;
        let middle = planar.sample(p, 0.0).electric_v_per_mm;
        p[axis] = -eps;
        let left = planar.sample(p, 0.0).electric_v_per_mm;
        p[axis] = eps;
        let right = planar.sample(p, 0.0).electric_v_per_mm;
        assert!(middle[axis].abs() < 1e-13);
        assert!(
            ((right[axis] - middle[axis]) / eps - (middle[axis] - left[axis]) / eps).abs() < 1e-7
        );
    }
    let mut cylinder = grid(9, 8, 1, [0.2, 0.3, 1.0], |p| 3.0 * p[0] + p[1].powi(2));
    cylinder.symmetry = Symmetry::Cylindrical;
    let cylinder = GridField::new(cylinder).unwrap();
    let s = cylinder.sample([0.47, 0.3, 0.4], 0.0);
    assert!(distance(s.electric_v_per_mm, [-3.0, -0.6, -0.8]) < 1e-12);
    let axis = cylinder.sample([0.47, 0.0, 0.0], 0.0);
    assert!(distance(axis.electric_v_per_mm, [-3.0, 0.0, 0.0]) < 1e-12);
    let mut anti = grid(8, 1, 1, [0.2, 1.0, 1.0], |p| p[0].powi(3) + p[0]);
    anti.mirrors.x = true;
    anti.anti_mirrors.x = true;
    let anti = GridField::new(anti).unwrap();
    let s = anti.sample([-0.53, 0.0, 0.0], 0.0);
    assert!((s.potential_v - (-0.53f64).powi(3) + 0.53).abs() < 1e-12);
    assert!((s.electric_v_per_mm[0] + 3.0 * 0.53f64.powi(2) + 1.0).abs() < 1e-12);
}

#[test]
fn fast_adjust_voltages_and_absolute_time_drive_trajectories() {
    let linear = GridField::new(grid(8, 4, 1, [1.0; 3], |p| p[0])).unwrap();
    let quadratic = GridField::new(grid(8, 4, 1, [1.0; 3], |p| p[1].powi(2))).unwrap();
    let field = Superposition::new(vec![
        BasisPotential {
            grid: linear,
            voltage: Box::new(|t| 2.0 * t),
        },
        BasisPotential {
            grid: quadratic,
            voltage: Box::new(|_| -3.0),
        },
    ])
    .unwrap()
    .with_magnetic(|p, t| [p[0], t, 7.0]);
    let p = [2.3, 1.2, 9.0];
    let s = field.sample(p, 4.0);
    assert!((s.potential_v - (8.0 * 2.3 - 3.0 * 1.2f64.powi(2))).abs() < 1e-12);
    assert!(distance(s.electric_v_per_mm, [-8.0, 7.2, 0.0]) < 1e-12);
    assert_eq!(s.magnetic_gauss, [2.3, 4.0, 7.0]);
    let rf = Superposition::new(vec![BasisPotential {
        grid: GridField::new(grid(20, 1, 1, [1.0; 3], |p| -p[0])).unwrap(),
        voltage: Box::new(|t| t),
    }])
    .unwrap();
    let ion = Particle::new(100.0, 1.0, [2.0, 0.0, 0.0], [0.0; 3], 3.0).unwrap();
    let result = fly(
        &rf,
        &ion,
        &FlightOptions {
            duration_us: 0.7,
            ..Default::default()
        },
    )
    .unwrap();
    let t: f64 = 0.7;
    let expected = 2.0 + ELECTRIC_ACCELERATION / 100.0 * (3.0 * t * t / 2.0 + t.powi(3) / 6.0);
    assert!((result.final_state.position_mm[0] - expected).abs() < 1e-11);
}

#[test]
fn grid_flight_converges_at_least_cubically_against_analytic_field() {
    let k: f64 = 1.3;
    let analytic = AnalyticField(|p: [f64; 3], _| FieldSample {
        potential_v: 0.2 * (k * p[0]).sin(),
        electric_v_per_mm: [-0.2 * k * (k * p[0]).cos(), 0.0, 0.0],
        ..Default::default()
    });
    // Sample several launch phases within cells: a single trajectory can have
    // an accidentally cancelling leading interpolation error at one spacing.
    let particles: Vec<_> = (0..17)
        .map(|i| {
            Particle::new(
                40.0,
                1.0,
                [1.137 + i as f64 * 0.047_13, 0.3, 0.0],
                [1.7, 0.0, 0.0],
                0.0,
            )
            .unwrap()
        })
        .collect();
    let options = FlightOptions {
        duration_us: 2.1,
        max_step_us: 0.02,
        relative_tolerance: 1e-12,
        position_tolerance_mm: 1e-12,
        velocity_tolerance_mm_per_us: 1e-12,
        ..Default::default()
    };
    let reference = fly_many(&analytic, &particles, &options).unwrap();
    let mut errors = Vec::new();
    for cells in [32, 64, 128, 256] {
        let h = 8.0 / cells as f64;
        let field = GridField::new(grid(cells + 1, 1, 1, [h, 1.0, 1.0], |p| {
            0.2 * (k * p[0]).sin()
        }))
        .unwrap();
        let results = fly_many(&field, &particles, &options).unwrap();
        let mut sum2 = 0.0;
        for (result, reference) in results.iter().zip(&reference) {
            assert_eq!(result.splat_reason, SplatReason::TimeLimit);
            sum2 += distance(
                result.final_state.position_mm,
                reference.final_state.position_mm,
            )
            .powi(2);
        }
        let error = (sum2 / particles.len() as f64).sqrt();
        println!(
            "interpolated flight: h={h:.5} mm, RMS position error={error:.9e} mm (17 launch phases)"
        );
        errors.push(error);
    }
    for pair in errors.windows(2) {
        let order = (pair[0] / pair[1]).log2();
        println!("observed position convergence order {order:.4}");
        assert!(order > 2.9, "errors {errors:?}");
    }
}

#[test]
fn occupied_cells_require_every_corner_and_localize_integer_faces() {
    for nz in [1, 4] {
        let mut pa = PotentialArray::new(8, 7, nz);
        pa.mirrors.y = true;
        pa.mirrors.z = nz > 1;
        let zs: &[usize] = if nz == 1 { &[0] } else { &[1, 2] };
        let mut corners = Vec::new();
        for &z in zs {
            for y in [2, 3] {
                for x in [3, 4] {
                    let i = pa.index(x, y, z).unwrap();
                    pa.electrodes[i] = true;
                    corners.push(i);
                }
            }
        }
        let z = if nz == 1 { 0.0 } else { -1.4 };
        let inside = [3.2, -2.4, z];
        let field = GridField::new(pa.clone()).unwrap();
        let legacy = GridField::new(pa.clone())
            .unwrap()
            .with_nearest_node_collisions();
        assert!(matches!(
            field.status(inside),
            Some(SplatReason::Electrode { .. })
        ));
        assert_eq!(field.status([2.75, -2.4, z]), None);
        assert!(matches!(
            legacy.status([2.75, -2.4, z]),
            Some(SplatReason::Electrode { .. })
        ));
        for corner in corners {
            let mut missing = pa.clone();
            missing.electrodes[corner] = false;
            assert_eq!(GridField::new(missing).unwrap().status(inside), None);
        }
        let ion = Particle::new(1.0, 1.0, [1.0, -2.4, z], [10.0, 0.0, 0.0], 0.0).unwrap();
        for integrator in [Integrator::Rk45, Integrator::Boris] {
            let result = fly(
                &field,
                &ion,
                &FlightOptions {
                    duration_us: 2.0,
                    initial_step_us: 1.0,
                    max_step_us: 1.0,
                    integrator,
                    ..Default::default()
                },
            )
            .unwrap();
            assert!(matches!(result.splat_reason, SplatReason::Electrode { .. }));
            assert!((result.tof_us - 0.2).abs() < 1e-12);
            assert!((result.final_state.position_mm[0] - 3.0).abs() < 1e-11);
        }
    }
    let mut cyl = PotentialArray::new(12, 8, 1);
    cyl.symmetry = Symmetry::Cylindrical;
    for y in [3, 4] {
        for x in 0..cyl.nx {
            cyl.electrodes[y * cyl.nx + x] = true;
        }
    }
    let field = GridField::new(cyl).unwrap();
    let crossing = field
        .intersection([1.0, -5.0, 0.0], [1.0, 5.0, 0.0])
        .unwrap();
    assert!((crossing.0 - 0.1).abs() < 1e-12);
    assert!(matches!(crossing.1, SplatReason::Electrode { .. }));
}

#[test]
fn electrodes_splat_without_tunneling_and_exit_times_are_localized() {
    let mut pa = PotentialArray::new(12, 1, 1);
    pa.electrodes[5] = true;
    let field = GridField::new(pa).unwrap().with_nearest_node_collisions();
    let ion = Particle::new(1.0, 1.0, [1.0, 0.0, 0.0], [10.0, 0.0, 0.0], 0.0).unwrap();
    let options = FlightOptions {
        duration_us: 2.0,
        initial_step_us: 1.0,
        max_step_us: 1.0,
        ..Default::default()
    };
    for integrator in [Integrator::Rk45, Integrator::Boris] {
        let result = fly(
            &field,
            &ion,
            &FlightOptions {
                integrator,
                ..options.clone()
            },
        )
        .unwrap();
        assert_eq!(result.splat_reason, SplatReason::Electrode { index: 5 });
        assert!((result.tof_us - 0.35).abs() < 1e-12);
        assert!((result.final_state.position_mm[0] - 4.5).abs() < 1e-11);
    }
    let mut cyl = PotentialArray::new(12, 8, 1);
    cyl.symmetry = Symmetry::Cylindrical;
    for x in 0..12 {
        cyl.electrodes[3 * 12 + x] = true;
    }
    let cyl = GridField::new(cyl).unwrap().with_nearest_node_collisions();
    // Same free endpoint radius, but the chord crosses the radial electrode.
    let ion = Particle::new(1.0, 1.0, [1.0, -5.0, 0.0], [0.0, 10.0, 0.0], 0.0).unwrap();
    let result = fly(&cyl, &ion, &options).unwrap();
    assert!(matches!(result.splat_reason, SplatReason::Electrode { .. }));
    assert!((result.tof_us - 0.15).abs() < 1e-12);
    let empty = GridField::new(PotentialArray::new(12, 1, 1)).unwrap();
    let ion = Particle::new(1.0, 1.0, [1.0, 0.0, 0.0], [10.0, 0.0, 0.0], 0.0).unwrap();
    let result = fly(&empty, &ion, &options).unwrap();
    assert_eq!(result.splat_reason, SplatReason::OutsideGrid);
    assert!((result.tof_us - 1.0).abs() < 1e-12);
    let on_electrode = Particle::new(1.0, 1.0, [5.0, 0.0, 0.0], [0.0; 3], 0.0).unwrap();
    assert_eq!(fly(&field, &on_electrode, &options).unwrap().tof_us, 0.0);
}

#[test]
fn localized_curved_flight_reports_the_first_physical_event() {
    // The full-step chord misses the electrode and crosses the detector.
    // Shortening the step resolves the parabola into the electrode first.
    let mut pa = grid(13, 13, 1, [1.0; 3], |p| {
        -20.0 * p[1] / ELECTRIC_ACCELERATION
    });
    let electrode = pa.index(5, 3, 0).unwrap();
    pa.electrodes[electrode] = true;
    let field = GridField::new(pa).unwrap().with_nearest_node_collisions();
    let ion = Particle::new(1.0, 1.0, [1.0, 1.0, 0.0], [10.0, 0.0, 0.0], 0.0).unwrap();
    let result = fly(
        &field,
        &ion,
        &FlightOptions {
            duration_us: 2.0,
            initial_step_us: 1.0,
            max_step_us: 1.0,
            stop_plane: Some(StopPlane {
                axis: 0,
                coordinate_mm: 8.0,
            }),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        result.splat_reason,
        SplatReason::Electrode { index: electrode }
    );
    assert!((result.tof_us - 0.15f64.sqrt()).abs() < 1e-12);
}

#[test]
fn rayon_preserves_particle_order_and_matches_serial() {
    let field = AnalyticField(|_, t| FieldSample {
        electric_v_per_mm: [0.1 * t, 0.0, 0.0],
        ..Default::default()
    });
    let particles: Vec<_> = (0..80)
        .map(|i| {
            Particle::from_energy(
                40.0,
                1.0,
                [i as f64, 0.0, 0.0],
                10.0,
                [1.0, 0.2, 0.0],
                i as f64 * 0.01,
            )
            .unwrap()
        })
        .collect();
    let options = FlightOptions {
        duration_us: 0.3,
        sample_every: Some(3),
        ..Default::default()
    };
    let parallel = fly_many(&field, &particles, &options).unwrap();
    for (particle, result) in particles.iter().zip(parallel) {
        let serial = fly(&field, particle, &options).unwrap();
        assert_eq!(serial.final_state, result.final_state);
        assert_eq!(serial.tof_us, result.tof_us);
        assert_eq!(serial.trajectory, result.trajectory);
    }
}

#[test]
fn relativistic_acceleration_and_boris_energy() {
    let field = AnalyticField(|_, _| FieldSample {
        electric_v_per_mm: [1e9, 0.0, 0.0],
        ..Default::default()
    });
    let ion =
        Particle::from_relativistic_energy(1.0, 1.0, [0.0; 3], 0.0, [1.0, 0.0, 0.0], 0.0).unwrap();
    let t = 1e-4;
    let options = FlightOptions {
        relativistic: true,
        duration_us: t,
        initial_step_us: t / 100.0,
        max_step_us: t / 10.0,
        ..Default::default()
    };
    let result = fly(&field, &ion, &options).unwrap();
    let proper = ELECTRIC_ACCELERATION * 1e9 * t;
    let expected_v = proper / (1.0 + (proper / SPEED_OF_LIGHT).powi(2)).sqrt();
    let expected_x = SPEED_OF_LIGHT.powi(2) / (ELECTRIC_ACCELERATION * 1e9)
        * ((1.0 + (proper / SPEED_OF_LIGHT).powi(2)).sqrt() - 1.0);
    assert!((result.final_state.velocity_mm_per_us[0] / expected_v - 1.0).abs() < 1e-10);
    assert!((result.final_state.position_mm[0] / expected_x - 1.0).abs() < 1e-9);
    let magnet = AnalyticField(|_, _| FieldSample {
        magnetic_gauss: [0.0, 0.0, 1e8],
        ..Default::default()
    });
    let ion =
        Particle::from_relativistic_energy(1.0, 1.0, [0.0; 3], 1e9, [1.0, 0.0, 0.0], 0.0).unwrap();
    let result = fly(
        &magnet,
        &ion,
        &FlightOptions {
            integrator: Integrator::Boris,
            ..options
        },
    )
    .unwrap();
    assert!(
        (distance(result.final_state.velocity_mm_per_us, [0.0; 3])
            / distance(ion.initial.velocity_mm_per_us, [0.0; 3])
            - 1.0)
            .abs()
            < 1e-12
    );
}

#[test]
fn rejects_invalid_inputs_and_reports_limits() {
    assert!(Particle::new(0.0, 1.0, [0.0; 3], [0.0; 3], 0.0).is_err());
    assert!(Particle::from_energy(1.0, 1.0, [0.0; 3], -1.0, [1.0; 3], 0.0).is_err());
    assert!(Particle::from_energy(1.0, 1.0, [0.0; 3], 1.0, [0.0; 3], 0.0).is_err());
    let particle = Particle::new(1.0, 1.0, [0.0; 3], [0.0; 3], 0.0).unwrap();
    let zero = AnalyticField(|_, _| FieldSample::default());
    assert!(
        fly(
            &zero,
            &particle,
            &FlightOptions {
                sample_every: Some(0),
                ..Default::default()
            }
        )
        .is_err()
    );
    let result = fly(
        &zero,
        &particle,
        &FlightOptions {
            max_steps: 1,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(result.splat_reason, SplatReason::StepLimit);
    let bad = AnalyticField(|_, _| FieldSample {
        electric_v_per_mm: [f64::NAN; 3],
        ..Default::default()
    });
    assert_eq!(
        fly(&bad, &particle, &FlightOptions::default())
            .unwrap()
            .splat_reason,
        SplatReason::NonFinite
    );
    let mut invalid = PotentialArray::new(3, 1, 1);
    invalid.dx_mm = 0.0;
    assert!(GridField::new(invalid).is_err());
    let bases = vec![
        BasisPotential {
            grid: GridField::new(PotentialArray::new(3, 1, 1)).unwrap(),
            voltage: Box::new(|_| 1.0),
        },
        BasisPotential {
            grid: GridField::new(PotentialArray::new(4, 1, 1)).unwrap(),
            voltage: Box::new(|_| 1.0),
        },
    ];
    assert!(Superposition::new(bases).is_err());
}
