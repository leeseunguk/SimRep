//! Node-centred MMS sources are differentiated analytically, not by the
//! discrete operator under test. All ladders preserve endpoints and have r=2.
use std::f64::consts::PI;

use simrep_core::{
    uncertainty::{Behaviour, Level, observed_order},
    validation::{Gate, record_order},
};
use simrep_geom::{Electrode, Sdf};
use simrep_solve::{
    cutcell::CutCells,
    dielectric::Permittivity,
    mg::{self, Geometry, Grid, Options},
};

fn options() -> Options {
    Options {
        relative_tolerance: 0.0,
        absolute_tolerance: 1e-10,
        max_cycles: 200,
        ..Default::default()
    }
}

fn smooth(n: usize, dimensions: usize, cylindrical: bool, poisson: bool) -> (f64, f64) {
    let h = 1.0 / (n - 1) as f64;
    let grid = Grid {
        dims: [n, n, if dimensions == 3 { n } else { 1 }],
        spacing: [h; 3],
        geometry: if cylindrical {
            Geometry::Cylindrical { radial_origin: 0.0 }
        } else {
            Geometry::Planar
        },
    };
    let point = |i: usize| {
        [
            (i % n) as f64 * h,
            (i / n % n) as f64 * h,
            (i / (n * n)) as f64 * h,
        ]
    };
    let exact = |p: [f64; 3]| {
        if cylindrical {
            // r^4 + sin(pi z) is regular at r=0; -L V = -16 r^2 + pi^2 sin(pi z).
            if poisson {
                p[0].powi(4) + (PI * p[1]).sin()
            } else {
                // Axisymmetric solid harmonic of degree four, L V = 0.
                p[1].powi(4) - 3.0 * p[1].powi(2) * p[0].powi(2) + 0.375 * p[0].powi(4)
            }
        } else if poisson {
            p.into_iter()
                .take(dimensions)
                .map(|x| (PI * x).sin())
                .product()
        } else if dimensions == 2 {
            p[0].exp() * p[1].cos()
        } else {
            (2.0_f64.sqrt() * p[0]).exp() * p[1].cos() * p[2].cos()
        }
    };
    let len = grid.dims.iter().product();
    let fixed: Vec<_> = (0..len)
        .map(|i| {
            [i % n, i / n % n, i / (n * n)]
                .into_iter()
                .take(dimensions)
                .enumerate()
                .any(|(a, c)| c + 1 == n || (c == 0 && !(cylindrical && a == 0)))
        })
        .collect();
    let mut values: Vec<_> = fixed
        .iter()
        .enumerate()
        .map(|(i, &e)| if e { exact(point(i)) } else { 0.0 })
        .collect();
    let source: Vec<_> = (0..len)
        .map(|i| {
            if !poisson {
                0.0
            } else if cylindrical {
                let p = point(i);
                -16.0 * p[0].powi(2) + PI.powi(2) * (PI * p[1]).sin()
            } else {
                dimensions as f64 * PI.powi(2) * exact(point(i))
            }
        })
        .collect();
    let report = if poisson {
        mg::solve_poisson(&mut values, &fixed, &source, grid, options())
    } else {
        mg::solve(&mut values, &fixed, grid, options())
    }
    .unwrap();
    assert!(report.converged, "n={n}: {:?}", report.residual_history);
    let error = values
        .iter()
        .enumerate()
        .map(|(i, v)| (v - exact(point(i))).abs())
        .fold(0.0_f64, f64::max);
    let center = n / 2 + n * (n / 2) + if dimensions == 3 { n * n * (n / 2) } else { 0 };
    (error, values[center] - exact(point(center)))
}

fn smooth_studies(sizes: [usize; 3]) {
    for (dimensions, cylindrical, poisson, name) in [
        (2, false, false, "mms-laplace-2d"),
        (3, false, false, "mms-laplace-3d"),
        (2, true, false, "mms-laplace-cylindrical"),
        (2, false, true, "mms-poisson-2d"),
        (3, false, true, "mms-poisson-3d"),
        (2, true, true, "mms-poisson-cylindrical"),
    ] {
        let results = sizes.map(|n| smooth(n, dimensions, cylindrical, poisson));
        let errors = results.map(|r| r.0);
        record_order(name, "equations (1), analytic nodal solution", errors, 2.0);
        for pair in errors.windows(2) {
            let order = (pair[0] / pair[1]).log2();
            assert!((1.9..2.1).contains(&order), "{name}: {errors:?}, p={order}");
        }
        let levels = std::array::from_fn(|i| Level {
            h: 1.0 / (sizes[2 - i] - 1) as f64,
            // Signed centre error is the same physical observable on each grid.
            value: results[2 - i].1,
        });
        let triplet = observed_order(&levels).unwrap();
        assert_eq!(triplet.behaviour, Behaviour::Monotone);
        Gate {
            name: &format!("{name}-center"),
            reference: "analytic center potential",
            metric: "center potential error (V)",
            value: levels[0].value,
            datum: 0.0,
            u_num: triplet.gci_fine.map(|g| g * levels[0].value.abs()),
            u_input: 0.0,
            u_datum: 0.0,
            band: "u_val from three-grid Celik GCI",
        }
        .record()
        .unwrap();
    }
}

#[test]
fn multigrid_mms_orders() {
    smooth_studies([9, 17, 33]);
}

fn dielectric(n: usize) -> f64 {
    let h = 1.0 / (n - 1) as f64;
    let grid = Grid {
        dims: [n, n, 1],
        spacing: [h; 3],
        geometry: Geometry::Planar,
    };
    let mut exact = Vec::new();
    let mut eps = Vec::new();
    let mut source = Vec::new();
    let mut fixed = Vec::new();
    for y in 0..n {
        for x in 0..n {
            let (x, y) = (x as f64 * h, y as f64 * h);
            let v = (PI * x).sin() * (PI * y).sin();
            exact.push(v);
            eps.push(1.0 + x);
            // -div((1+x) grad V) = 2 pi^2 (1+x) V - dV/dx.
            source.push(2.0 * PI.powi(2) * (1.0 + x) * v - PI * (PI * x).cos() * (PI * y).sin());
            fixed.push(x == 0.0 || x == 1.0 || y == 0.0 || y == 1.0);
        }
    }
    let material = Permittivity::from_nodes(grid, &fixed, &eps).unwrap();
    let mut values: Vec<_> = exact
        .iter()
        .zip(&fixed)
        .map(|(&v, &e)| if e { v } else { 0.0 })
        .collect();
    assert!(
        material
            .solve_poisson(&mut values, &source, options())
            .unwrap()
            .converged
    );
    values
        .iter()
        .zip(exact)
        .map(|(v, e)| (v - e).abs())
        .fold(0.0_f64, f64::max)
}

#[test]
fn linear_dielectric_mms_order() {
    let errors = [9, 17, 33].map(dielectric);
    record_order(
        "mms-linear-dielectric",
        "equations (17)-(18), epsilon=1+x",
        errors,
        2.0,
    );
    for pair in errors.windows(2) {
        assert!(
            (1.9..2.1).contains(&(pair[0] / pair[1]).log2()),
            "{errors:?}"
        );
    }
}

fn curved(n: usize, sphere: bool, phase: f64) -> (f64, f64) {
    let (inner, outer) = (0.31_f64, 0.83_f64);
    let h = 2.0 / (n - 1) as f64;
    let center = [phase * h, 0.0, 0.0];
    let shape = |radius| {
        if sphere {
            Sdf::Sphere { center, radius }
        } else {
            Sdf::Cylinder {
                center,
                radius,
                axis: 2,
            }
        }
    };
    let outside = Sdf::Box {
        center: [0.0; 3],
        half_extent: [4.0; 3],
    }
    .difference(shape(outer));
    let cuts = CutCells::new(
        Grid {
            dims: [n, n, if sphere { n } else { 1 }],
            spacing: [h; 3],
            geometry: Geometry::Planar,
        },
        [-1.0, -1.0, if sphere { -1.0 } else { 0.0 }],
        &[
            Electrode::new(1.0, shape(inner)),
            Electrode::new(0.0, outside),
        ],
    )
    .unwrap();
    let mut values = cuts.values.clone();
    assert!(cuts.solve(&mut values, options()).unwrap().converged);
    let mut error = (0.0_f64, 0.0_f64);
    for (i, &v) in values.iter().enumerate() {
        // Shift the analytic solution AND its field into the geometry frame.
        let p: [f64; 3] = std::array::from_fn(|a| {
            cuts.origin[a] + [i % n, i / n % n, i / (n * n)][a] as f64 * h - center[a]
        });
        let r = p
            .iter()
            .take(if sphere { 3 } else { 2 })
            .map(|x| x * x)
            .sum::<f64>()
            .sqrt();
        if r <= inner || r >= outer {
            continue;
        }
        let (exact, derivative) = if sphere {
            (
                (1.0 / r - 1.0 / outer) / (1.0 / inner - 1.0 / outer),
                -1.0 / (r * r * (1.0 / inner - 1.0 / outer)),
            )
        } else {
            (
                (outer / r).ln() / (outer / inner).ln(),
                -1.0 / (r * (outer / inner).ln()),
            )
        };
        error.0 = error.0.max((v - exact).abs());
        if !cuts.fixed[i] && (r - inner).min(outer - r) < 2.0 * h {
            let field = cuts.electric_field(&values, i).unwrap();
            for a in 0..if sphere { 3 } else { 2 } {
                error.1 = error.1.max((field[a] + derivative * p[a] / r).abs());
            }
        }
    }
    error
}

fn curved_studies(sizes: [usize; 3]) {
    for (sphere, name) in [(true, "mms-cut-sphere"), (false, "mms-cut-cylinder")] {
        let errors = sizes.map(|n| {
            // Worst of four phases prevents choosing a favorable grid alignment.
            [0.0, 0.25, 0.5, 0.75]
                .map(|phase| curved(n, sphere, phase))
                .into_iter()
                .fold((0.0_f64, 0.0_f64), |a, b| (a.0.max(b.0), a.1.max(b.1)))
        });
        record_order(
            &format!("{name}-potential"),
            "equation (3), concentric conductors",
            errors.map(|e| e.0),
            2.0,
        );
        record_order(
            &format!("{name}-field"),
            "equation (3), near-surface field",
            errors.map(|e| e.1),
            1.0,
        );
        let levels = std::array::from_fn(|i| Level {
            h: 2.0 / (sizes[2 - i] - 1) as f64,
            value: errors[2 - i].0,
        });
        let triplet = observed_order(&levels).unwrap();
        Gate {
            name: if sphere {
                "exact-laplace-spheres"
            } else {
                "exact-laplace-coax"
            },
            reference: "exact_solutions.json; concentric-conductor Laplace solution",
            metric: "maximum potential error over four grid phases (V)",
            value: levels[0].value,
            datum: 0.0,
            u_num: triplet.gci_fine.map(|g| g * levels[0].value.abs()),
            u_input: 0.0,
            u_datum: 0.0,
            band: "existing potential envelope 6h^2 V; three-grid Celik GCI of error norm",
        }
        .record()
        .unwrap();
        // No reduced-order acceptance threshold: any pre-asymptotic order is
        // published as a finding. Check the documented absolute envelopes.
        for (n, e) in sizes.into_iter().zip(errors) {
            let h = 2.0 / (n - 1) as f64;
            assert!(e.0 < 6.0 * h * h && e.1 < 20.0 * h, "{name} n={n}: {e:?}");
        }
    }
}

#[test]
fn exact_surface_mms_orders_and_shifted_reference_frame() {
    curved_studies([17, 33, 65]);
}

#[test]
#[ignore = "larger CPU MMS study; run separately without SIMREP_GATE_DIR"]
fn larger_mms_study() {
    smooth_studies([33, 65, 129]);
    curved_studies([65, 129, 257]);
}
