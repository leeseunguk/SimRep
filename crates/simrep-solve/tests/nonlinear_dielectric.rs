use simrep_solve::{
    dielectric::{FieldLaw, FieldSampler, NonlinearOptions, NonlinearPermittivity},
    gpu::{Backends, Gpu},
    mg::{Geometry, Grid, Options},
};

fn options() -> Options {
    Options {
        relative_tolerance: 1e-12,
        max_cycles: 200,
        ..Options::default()
    }
}

fn outer() -> NonlinearOptions {
    NonlinearOptions {
        field_tolerance: 1e-11,
        ..NonlinearOptions::default()
    }
}

#[test]
fn nonlinear_layered_capacitor_preserves_flux_and_both_convergence_criteria() {
    for nz in [1, 4] {
        let grid = Grid {
            dims: [33, 5, nz],
            spacing: [1.0 / 32.0, 0.25, 0.2],
            geometry: Geometry::Planar,
        };
        let fixed: Vec<_> = (0..33 * 5 * nz)
            .map(|i| i % 33 == 0 || i % 33 == 32)
            .collect();
        let initial: Vec<_> = (0..fixed.len())
            .map(|i| if i % 33 == 32 { 1.0 } else { 0.0 })
            .collect();
        let eps: Vec<_> = (0..32 * 4 * (nz - 1).max(1))
            .map(|i| if i % 32 < 16 { 1.0 } else { 3.0 })
            .collect();
        let material = NonlinearPermittivity::from_cells(
            grid,
            &fixed,
            &eps,
            vec![(
                3.0,
                FieldLaw::SaturatingQuadratic {
                    lambda: 0.02,
                    minimum: 1.0,
                },
            )],
            FieldSampler::CellCenter,
        )
        .unwrap();
        let mut values = initial.clone();
        let report = material
            .solve(&mut values, options(), outer(), None)
            .unwrap();
        assert!(report.converged, "{report:?}");
        assert!(report.residual_history.last().unwrap().relative <= 1e-12);
        assert!(*report.field_history.last().unwrap() <= 1e-11);
        let (mut lo, mut hi) = (0.0, 1.0);
        for _ in 0..60 {
            let e = (lo + hi) * 0.5;
            if 3.0 * (1.0 - 0.02 * e * e) * e < 2.0 - e {
                lo = e;
            } else {
                hi = e;
            }
        }
        let e2 = (lo + hi) * 0.5;
        let d = 2.0 - e2;
        let error = values
            .iter()
            .enumerate()
            .map(|(i, v)| {
                let x = (i % 33) as f64 / 32.0;
                (v - (d * x.min(0.5) + e2 * (x - 0.5).max(0.0))).abs()
            })
            .fold(0.0_f64, f64::max);
        let flux_error = (0..32)
            .map(|i| {
                let e = (values[i + 1] - values[i]) * 32.0;
                let eps = if i < 16 {
                    1.0
                } else {
                    3.0 * (1.0 - 0.02 * e * e)
                };
                (eps * e - d).abs()
            })
            .fold(0.0_f64, f64::max);
        println!(
            "nonlinear layered nz={nz} max_V={error:.9e}, flux_error={flux_error:.9e}, iterations={}",
            report.iterations
        );
        assert!(error < 2e-10 && flux_error < 2e-9);
        for (i, &is_fixed) in fixed.iter().enumerate() {
            if is_fixed {
                assert_eq!(values[i], initial[i]);
            }
        }
        let mut limited = initial;
        let report = material
            .solve(
                &mut limited,
                options(),
                NonlinearOptions {
                    max_iterations: 1,
                    ..outer()
                },
                None,
            )
            .unwrap();
        assert!(!report.converged);
    }
}

fn integral(c: f64, lo: f64, hi: f64) -> f64 {
    // Fixed Simpson quadrature is independent of the finite-volume solver.
    let n = 4096;
    let h = (hi - lo) / n as f64;
    (0..=n)
        .map(|i| {
            let r = lo + i as f64 * h;
            let e = 2.0 * c / r / (1.0 + (1.0 + 4.0 * c / r).sqrt());
            e * if i == 0 || i == n {
                1.0
            } else if i % 2 == 0 {
                2.0
            } else {
                4.0
            }
        })
        .sum::<f64>()
        * h
        / 3.0
}

#[test]
fn nonlinear_coaxial_quadrature_reference_has_second_order_grid_convergence() {
    let (mut lo, mut hi) = (0.0, 10.0);
    for _ in 0..60 {
        let c = (lo + hi) * 0.5;
        if integral(c, 1.0, 3.0) < 1.0 {
            lo = c;
        } else {
            hi = c;
        }
    }
    let c = (lo + hi) * 0.5;
    let errors = [17, 33, 65].map(|n| {
        let grid = Grid {
            dims: [n, 5, 1],
            spacing: [2.0 / (n - 1) as f64, 0.25, 1.0],
            geometry: Geometry::Cylindrical { radial_origin: 1.0 },
        };
        let fixed: Vec<_> = (0..n * 5).map(|i| i % n == 0 || i % n == n - 1).collect();
        let mut values: Vec<_> = (0..n * 5)
            .map(|i| if i % n == 0 { 1.0 } else { 0.0 })
            .collect();
        let material = NonlinearPermittivity::from_cells(
            grid,
            &fixed,
            &vec![2.0; (n - 1) * 4],
            vec![(
                2.0,
                FieldLaw::Table {
                    points: vec![[0.0, 1.0], [10.0, 11.0]],
                },
            )],
            FieldSampler::CellCenter,
        )
        .unwrap();
        let report = material
            .solve(
                &mut values,
                options(),
                NonlinearOptions {
                    relaxation: 0.5,
                    ..outer()
                },
                None,
            )
            .unwrap();
        assert!(report.converged, "{report:?}");
        let error = (0..n)
            .map(|i| (values[i] - integral(c, 1.0 + i as f64 * grid.spacing[0], 3.0)).abs())
            .fold(0.0_f64, f64::max);
        println!(
            "nonlinear coax n={n}, max_V={error:.9e}, iterations={}",
            report.iterations
        );
        error
    });
    let order = (errors[1] / errors[2]).log2();
    println!("nonlinear coax last_pair_order={order:.6}");
    assert!(errors[0] > errors[1] && errors[1] > errors[2] && order > 1.8);
}

#[test]
fn invalid_constitutive_laws_are_rejected() {
    let grid = Grid {
        dims: [3, 3, 1],
        spacing: [1.0; 3],
        geometry: Geometry::Planar,
    };
    let mut fixed = vec![false; 9];
    fixed[0] = true;
    for law in [
        FieldLaw::Table {
            points: vec![[1.0, 2.0], [0.0, 1.0]],
        },
        FieldLaw::Table {
            points: vec![[0.0, 2.0], [1.0, 0.0]],
        },
        FieldLaw::SaturatingQuadratic {
            lambda: f64::NAN,
            minimum: 1.0,
        },
        FieldLaw::SaturatingQuadratic {
            lambda: 1.0,
            minimum: 0.0,
        },
    ] {
        assert!(
            NonlinearPermittivity::from_cells(
                grid,
                &fixed,
                &[2.0; 4],
                vec![(2.0, law)],
                FieldSampler::CellCenter
            )
            .is_err()
        );
    }
}

#[test]
fn nonlinear_gpu_inner_solver_matches_cpu_reference() {
    let Some(gpu) = pollster::block_on(Gpu::new(Backends::VULKAN)).unwrap() else {
        println!("SKIP nonlinear GPU: no Vulkan adapter");
        return;
    };
    let grid = Grid {
        dims: [17, 7, 3],
        spacing: [0.1; 3],
        geometry: Geometry::Planar,
    };
    let fixed: Vec<_> = (0..17 * 7 * 3)
        .map(|i| i % 17 == 0 || i % 17 == 16)
        .collect();
    let mut cpu: Vec<_> = (0..fixed.len())
        .map(|i| if i % 17 == 16 { 1.0 } else { 0.0 })
        .collect();
    let material = NonlinearPermittivity::from_cells(
        grid,
        &fixed,
        &vec![3.0; 16 * 6 * 2],
        vec![(
            3.0,
            FieldLaw::SaturatingQuadratic {
                lambda: 0.01,
                minimum: 1.0,
            },
        )],
        FieldSampler::CellCenter,
    )
    .unwrap();
    let mut actual = cpu.clone();
    assert!(
        material
            .solve(&mut cpu, options(), outer(), None)
            .unwrap()
            .converged
    );
    let report = material
        .solve(&mut actual, options(), outer(), Some(&gpu))
        .unwrap();
    assert!(report.converged, "{report:?}");
    let error = actual
        .iter()
        .zip(cpu)
        .map(|(a, b)| (a - b).abs())
        .fold(0.0_f64, f64::max);
    println!("nonlinear GPU {:?}: max_V={error:.9e}", gpu.info.name);
    assert!(error < 1e-9);
}
