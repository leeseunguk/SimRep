use std::f64::consts::PI;

use simrep_solve::{
    magnetostatics::{MagneticProblem, cell_source},
    mg::{Geometry, Grid, Options},
};

fn options() -> Options {
    Options {
        relative_tolerance: 1e-12,
        max_cycles: 300,
        ..Default::default()
    }
}

fn coords(i: usize, n: usize, h: f64) -> [f64; 3] {
    [
        (i % n) as f64 * h - 1.0,
        (i / n % n) as f64 * h - 1.0,
        (i / (n * n)) as f64 * h - 1.0,
    ]
}

// Primal-cell averages of one fixed physical sphere/cylinder. Integrate the
// clipped x chord, not a center-label geometry whose volume can oscillate
// non-monotonically when the grid is refined.
fn fraction(lo: [f64; 3], h: f64, radius: f64, sphere: bool) -> f64 {
    let dimensions = if sphere { 3 } else { 2 };
    let min2: f64 = (0..dimensions)
        .map(|a| {
            if lo[a] <= 0.0 && lo[a] + h >= 0.0 {
                0.0
            } else {
                lo[a].powi(2).min((lo[a] + h).powi(2))
            }
        })
        .sum();
    let max2: f64 = (0..dimensions)
        .map(|a| lo[a].powi(2).max((lo[a] + h).powi(2)))
        .sum();
    if min2 >= radius * radius {
        return 0.0;
    }
    if max2 <= radius * radius {
        return 1.0;
    }
    let samples = 32;
    let nz = if sphere { samples } else { 1 };
    let mut sum = 0.0;
    for z in 0..nz {
        let zz = if sphere {
            lo[2] + (z as f64 + 0.5) * h / samples as f64
        } else {
            0.0
        };
        for y in 0..samples {
            let yy = lo[1] + (y as f64 + 0.5) * h / samples as f64;
            let square = radius * radius - yy * yy - zz * zz;
            if square > 0.0 {
                let extent = square.sqrt();
                sum += ((lo[0] + h).min(extent) - lo[0].max(-extent)).max(0.0) / h;
            }
        }
    }
    sum / (samples * nz) as f64
}

fn magnet(n: usize, sphere: bool) -> f64 {
    let h = 2.0 / (n - 1) as f64;
    let grid = Grid {
        dims: [n, n, if sphere { n } else { 1 }],
        spacing: [h; 3],
        geometry: Geometry::Planar,
    };
    let count = grid.dims.iter().product();
    let radius: f64 = 0.43;
    let ng = 100.0;
    let exact = |p: [f64; 3]| {
        let r = (p[0] * p[0] + p[1] * p[1] + if sphere { p[2] * p[2] } else { 0.0 }).sqrt();
        let dimension = if sphere { 3.0 } else { 2.0 };
        4.0 * PI / (dimension * ng)
            * p[0]
            * if r <= radius {
                1.0
            } else {
                (radius / r).powf(dimension)
            }
    };
    let mut values = vec![0.0; count];
    let fixed: Vec<_> = (0..count)
        .map(|i| {
            let p = coords(i, n, h);
            let boundary = (0..if sphere { 3 } else { 2 }).any(|a| p[a].abs() > 1.0 - h / 2.0);
            if boundary {
                values[i] = exact(p);
            }
            boundary
        })
        .collect();
    let d = grid.dims.map(|n| n.saturating_sub(1).max(1));
    let m: Vec<_> = (0..d.iter().product())
        .map(|i| {
            let p = [
                (i % d[0]) as f64 * h + h / 2.0 - 1.0,
                (i / d[0] % d[1]) as f64 * h + h / 2.0 - 1.0,
                (i / (d[0] * d[1])) as f64 * h + h / 2.0 - 1.0,
            ];
            [
                fraction(p.map(|x| x - h / 2.0), h, radius, sphere),
                0.0,
                0.0,
            ]
        })
        .collect();
    let problem = MagneticProblem::scalar(grid, &fixed, &vec![1.0; m.len()], &m, None, ng).unwrap();
    let report = problem.solve(&mut values, options()).unwrap();
    assert!(report.converged);
    let error = (values
        .iter()
        .enumerate()
        .map(|(i, v)| (v - exact(coords(i, n, h))).powi(2))
        .sum::<f64>()
        / count as f64)
        .sqrt();
    println!(
        "magnet sphere={sphere} n={n}: rms_p={error:.9e}, compute_s={:.6}",
        report.timings.solve.as_secs_f64()
    );
    error
}

#[test]
fn uniformly_magnetized_sphere_and_transverse_cylinder_converge() {
    for sphere in [false, true] {
        let errors = [17, 33, 65].map(|n| magnet(n, sphere));
        let order = (errors[1] / errors[2]).log2();
        println!("magnet sphere={sphere}: last_pair_order={order:.6}");
        assert!(errors[2] < errors[1] && errors[1] < errors[0], "{errors:?}");
        assert!(order > 0.5, "{order}");
    }
}

#[test]
fn iron_slab_has_exact_continuous_normal_flux_and_is_solid() {
    let n = 33;
    let grid = Grid {
        dims: [n, 5, 1],
        spacing: [1.0 / 32.0, 0.25, 1.0],
        geometry: Geometry::Planar,
    };
    let fixed: Vec<_> = (0..n * 5).map(|i| i % n == 0 || i % n == n - 1).collect();
    let mu: Vec<_> = (0..32 * 4)
        .map(|i| if i % 32 < 16 { 1.0 } else { 100.0 })
        .collect();
    let problem =
        MagneticProblem::scalar(grid, &fixed, &mu, &vec![[0.0; 3]; mu.len()], None, 100.0).unwrap();
    let mut values: Vec<_> = (0..n * 5)
        .map(|i| if i % n == n - 1 { 1.0 } else { 0.0 })
        .collect();
    assert!(problem.solve(&mut values, options()).unwrap().converged);
    for (i, v) in values.iter().enumerate() {
        let x = (i % n) as f64 / 32.0;
        let exact = (x.min(0.5) + (x - 0.5).max(0.0) / 100.0) / 0.505;
        assert!((v - exact).abs() < 1e-9);
    }
    for x in 0..32 {
        let flux = mu[x] * (values[x + 1] - values[x]) * 32.0;
        assert!((flux - 1.0 / 0.505).abs() < 1e-7);
        assert_eq!(problem.solid_cells[x], x >= 16);
    }
}

fn wire(n: usize, coax: bool) -> f64 {
    let h = 2.0 / (n - 1) as f64;
    let grid = Grid {
        dims: [n, n, 1],
        spacing: [h, h, 1.0],
        geometry: Geometry::Planar,
    };
    let inner: f64 = 0.3;
    let outer: f64 = 0.7;
    let ng = 100.0;
    // I=1 A forward inside a, -1 A in a<r<b for coax.
    let primitive = |r: f64| {
        if r <= inner {
            r * r / (inner * inner)
        } else if !coax {
            1.0 + 2.0 * (r / inner).ln()
        } else if r < outer {
            1.0 + (2.0 * outer * outer * (r / inner).ln() - (r * r - inner * inner))
                / (outer * outer - inner * inner)
        } else {
            1.0 + (2.0 * outer * outer * (outer / inner).ln() - (outer * outer - inner * inner))
                / (outer * outer - inner * inner)
        }
    };
    let exact = |p: [f64; 3]| -primitive(p[0].hypot(p[1])) / ng;
    let mut values = vec![0.0; n * n];
    let fixed: Vec<_> = (0..n * n)
        .map(|i| {
            let p = coords(i, n, h);
            let boundary = p[0].abs() > 1.0 - h / 2.0 || p[1].abs() > 1.0 - h / 2.0;
            if boundary {
                values[i] = exact(p);
            }
            boundary
        })
        .collect();
    let j: Vec<_> = (0..(n - 1) * (n - 1))
        .map(|i| {
            let lo = [
                (i % (n - 1)) as f64 * h - 1.0,
                (i / (n - 1)) as f64 * h - 1.0,
                0.0,
            ];
            let inside = fraction(lo, h, inner, false);
            inside / (PI * inner * inner)
                - if coax {
                    (fraction(lo, h, outer, false) - inside)
                        / (PI * (outer * outer - inner * inner))
                } else {
                    0.0
                }
        })
        .collect();
    let problem = MagneticProblem::currents(grid, &fixed, &vec![1.0; j.len()], &j, ng).unwrap();
    assert!(problem.solve(&mut values, options()).unwrap().converged);
    let error = (values
        .iter()
        .enumerate()
        .map(|(i, v)| (v - exact(coords(i, n, h))).powi(2))
        .sum::<f64>()
        / (n * n) as f64)
        .sqrt();
    println!("wire coax={coax} n={n}: rms_Az={error:.9e}");
    error
}

#[test]
fn infinite_wire_and_return_coax_converge() {
    for coax in [false, true] {
        let errors = [33, 65, 129].map(|n| wire(n, coax));
        let order = (errors[1] / errors[2]).log2();
        println!("wire coax={coax}: last_pair_order={order:.6}");
        assert!(errors[2] < errors[1] && errors[1] < errors[0], "{errors:?}");
        assert!(order > 0.5);
    }
}

fn psi(n: usize) -> f64 {
    let h = 1.0 / (n - 1) as f64;
    let grid = Grid {
        dims: [n, n, 1],
        spacing: [h, h, 1.0],
        geometry: Geometry::Cylindrical { radial_origin: 0.0 },
    };
    let exact = |r: f64, z: f64| r * r * (3.0 - z * z - r * r);
    let mut values = vec![0.0; n * n];
    let fixed: Vec<_> = (0..n * n)
        .map(|i| {
            let r = (i % n) as f64 * h;
            let z = (i / n) as f64 * h;
            let boundary = i % n == n - 1 || i / n == 0 || i / n == n - 1;
            if boundary {
                values[i] = exact(r, z);
            }
            boundary
        })
        .collect();
    let j: Vec<_> = (0..(n - 1) * (n - 1))
        .map(|i| 10.0 * ((i % (n - 1)) as f64 + 0.5) * h / (4.0 * PI))
        .collect();
    let problem = MagneticProblem::currents(grid, &fixed, &vec![1.0; j.len()], &j, 1.0).unwrap();
    assert_eq!(problem.prepared_stencil().grid.geometry, Geometry::Planar);
    assert!(problem.solve(&mut values, options()).unwrap().converged);
    let error = values
        .iter()
        .enumerate()
        .map(|(i, v)| (v - exact((i % n) as f64 * h, (i / n) as f64 * h)).abs())
        .fold(0.0, f64::max);
    println!("psi n={n}: max={error:.9e}");
    error
}

#[test]
fn axisymmetric_current_uses_planar_one_over_r_operator() {
    let errors = [17, 33, 65].map(psi);
    let order = (errors[1] / errors[2]).log2();
    println!("psi last_pair_order={order:.6}");
    assert!(order > 1.7 && order < 2.3, "{errors:?} order={order}");
}

#[test]
fn invalid_magnetic_inputs_and_nonregular_axis_are_rejected() {
    let grid = Grid {
        dims: [5, 5, 1],
        spacing: [1.0; 3],
        geometry: Geometry::Cylindrical { radial_origin: 0.0 },
    };
    let mut fixed = vec![false; 25];
    fixed[24] = true;
    let problem = MagneticProblem::currents(grid, &fixed, &[1.0; 16], &[0.0; 16], 100.0).unwrap();
    assert!(problem.solve(&mut [1.0; 25], options()).is_err());
    assert!(MagneticProblem::currents(grid, &fixed, &[-1.0; 16], &[0.0; 16], 100.0).is_err());
    assert!(MagneticProblem::scalar(grid, &fixed, &[1.0; 16], &[[0.0; 3]; 16], None, 0.0).is_err());
    assert!(
        MagneticProblem::cartesian_current_component(grid, &fixed, 1.0, &[0.0; 16], 100.0).is_err()
    );
}

#[test]
fn cylindrical_cell_source_conserves_axis_adjacent_annular_volume() {
    let grid = Grid {
        dims: [5, 5, 1],
        spacing: [1.0; 3],
        geometry: Geometry::Cylindrical { radial_origin: 0.0 },
    };
    let mut cells = vec![0.0; 16];
    cells[4] = 1.0; // r=0..1, z=1..2: volume / (2*pi) = 1/2.
    let source = cell_source(grid, &cells).unwrap();
    let integral: f64 = source
        .iter()
        .enumerate()
        .map(|(i, &v)| {
            let r = i % 5;
            let radial_weight = if r == 0 { 0.125 } else { r as f64 };
            radial_weight * v
        })
        .sum();
    assert!(
        (integral - 0.5).abs() < 1e-15,
        "integrated source={integral}"
    );
    assert!((source[6] - 0.1875).abs() < 1e-15);
}

#[test]
fn uniform_vacuum_planes_are_solved_without_cycles_at_original_tolerance() {
    for dims in [[33, 17, 1], [9, 17, 5]] {
        for axis in 0..3 {
            if dims[axis] == 1 {
                continue;
            }
            let grid = Grid {
                dims,
                spacing: [0.5, 0.25, 0.75],
                geometry: Geometry::Planar,
            };
            let strides = [1, dims[0], dims[0] * dims[1]];
            let count = dims.iter().product();
            let cells = dims.map(|n| (n - 1).max(1)).iter().product();
            let fixed: Vec<_> = (0..count)
                .map(|i| {
                    let c = i / strides[axis] % dims[axis];
                    c == 0 || c + 1 == dims[axis]
                })
                .collect();
            let initial: Vec<_> = (0..count)
                .map(|i| {
                    if i / strides[axis] % dims[axis] == dims[axis] - 1 {
                        107.0
                    } else {
                        7.0
                    }
                })
                .collect();
            let problem = MagneticProblem::scalar(
                grid,
                &fixed,
                &vec![1.0; cells],
                &vec![[0.0; 3]; cells],
                None,
                100.0,
            )
            .unwrap();
            let options = Options {
                relative_tolerance: 1e-15,
                ..options()
            };
            let mut values = initial.clone();
            let query = problem
                .solve(
                    &mut values,
                    Options {
                        max_cycles: 0,
                        ..options
                    },
                )
                .unwrap();
            assert!(!query.converged);
            assert_eq!(values, initial);
            let report = problem.solve(&mut values, options).unwrap();
            assert!(report.converged);
            assert_eq!(report.iterations, 0);
            assert_eq!(report.levels, 1);
            for (i, &v) in values.iter().enumerate() {
                let exact =
                    7.0 + 100.0 * (i / strides[axis] % dims[axis]) as f64 / (dims[axis] - 1) as f64;
                assert_eq!(v, exact);
            }
        }
    }
}

#[test]
fn uniform_plane_initialization_does_not_replace_a_physical_source() {
    let grid = Grid {
        dims: [17, 9, 1],
        spacing: [1.0; 3],
        geometry: Geometry::Planar,
    };
    let fixed: Vec<_> = (0..153).map(|i| i % 17 == 0 || i % 17 == 16).collect();
    let mut values: Vec<_> = (0..153)
        .map(|i| if i % 17 == 16 { 100.0 } else { 0.0 })
        .collect();
    let problem =
        MagneticProblem::currents(grid, &fixed, &[1.0; 128], &[1.0; 128], 4.0 * PI).unwrap();
    let report = problem.solve(&mut values, options()).unwrap();
    assert!(report.converged);
    assert!(report.iterations > 0);
    for (i, &value) in values.iter().enumerate() {
        let x = (i % 17) as f64;
        assert!((value - (100.0 * x / 16.0 + x * (16.0 - x) / 2.0)).abs() < 1e-8);
    }
}

#[test]
fn magnetic_device_crossover_uses_the_measured_native_and_fallback_brackets() {
    for (dims, native, fallback) in [
        ([201, 201, 1], false, false),
        ([65, 65, 65], false, false),
        ([101, 101, 51], true, false),
        ([101, 101, 101], true, true),
        ([201, 201, 201], true, true),
    ] {
        let grid = Grid {
            dims,
            spacing: [1.0; 3],
            geometry: Geometry::Planar,
        };
        assert_eq!(simrep_solve::magnetostatics::prefer_gpu(grid, true), native);
        assert_eq!(
            simrep_solve::magnetostatics::prefer_gpu(grid, false),
            fallback
        );
    }
}

fn shield(n: usize) -> f64 {
    let h = 2.0 / (n - 1) as f64;
    let grid = Grid {
        dims: [n, n, 1],
        spacing: [h, h, 1.0],
        geometry: Geometry::Planar,
    };
    let inner: f64 = 0.3;
    let outer: f64 = 0.6;
    let mu: f64 = 20.0;
    let factor = 4.0 * mu / ((mu + 1.0).powi(2) - (inner / outer).powi(2) * (mu - 1.0).powi(2));
    let c = (mu + 1.0) * factor / (2.0 * mu);
    let d = (mu - 1.0) * factor * inner * inner / (2.0 * mu);
    let e = (c - 1.0) * outer * outer + d;
    let exact = |p: [f64; 3]| {
        let r = p[0].hypot(p[1]);
        if r < inner {
            -factor * p[0]
        } else if r < outer {
            -(c + d / (r * r)) * p[0]
        } else {
            -(1.0 + e / (r * r)) * p[0]
        }
    };
    let mut values = vec![0.0; n * n];
    let fixed: Vec<_> = (0..n * n)
        .map(|i| {
            let p = coords(i, n, h);
            let boundary = p[0].abs() > 1.0 - h / 2.0 || p[1].abs() > 1.0 - h / 2.0;
            if boundary {
                values[i] = exact(p);
            }
            boundary
        })
        .collect();
    let relative: Vec<_> = (0..(n - 1) * (n - 1))
        .map(|i| {
            let lo = [
                (i % (n - 1)) as f64 * h - 1.0,
                (i / (n - 1)) as f64 * h - 1.0,
                0.0,
            ];
            1.0 + (mu - 1.0) * (fraction(lo, h, outer, false) - fraction(lo, h, inner, false))
        })
        .collect();
    let problem = MagneticProblem::scalar(
        grid,
        &fixed,
        &relative,
        &vec![[0.0; 3]; relative.len()],
        None,
        1.0,
    )
    .unwrap();
    assert!(problem.solve(&mut values, options()).unwrap().converged);
    let center = n / 2 + n * (n / 2);
    let field = -(values[center + 1] - values[center - 1]) / (2.0 * h);
    let error = (values
        .iter()
        .enumerate()
        .map(|(i, v)| (v - exact(coords(i, n, h))).powi(2))
        .sum::<f64>()
        / (n * n) as f64)
        .sqrt();
    println!("iron shield n={n}: rms_p={error:.9e}, center_B={field:.12}, analytic_B={factor:.12}");
    error
}

#[test]
fn iron_cylindrical_shield_converges_to_analytic_attenuation() {
    let errors = [33, 65, 129].map(shield);
    let order = (errors[1] / errors[2]).log2();
    println!("iron shield last_pair_order={order:.6}");
    assert!(errors[2] < errors[1] && errors[1] < errors[0], "{errors:?}");
    assert!(order > 0.5);
}
