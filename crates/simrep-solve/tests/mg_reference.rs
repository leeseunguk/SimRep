use simrep_solve::mg::{Geometry, Grid, Options, Report, solve};

fn at(x: usize, y: usize, z: usize, dims: [usize; 3]) -> usize {
    x + dims[0] * (y + dims[1] * z)
}

fn cells(dims: [usize; 3], mut visit: impl FnMut(usize, [usize; 3])) {
    for z in 0..dims[2] {
        for y in 0..dims[1] {
            for x in 0..dims[0] {
                visit(at(x, y, z, dims), [x, y, z]);
            }
        }
    }
}

fn accurate(values: &mut [f64], fixed: &[bool], grid: Grid) -> Report {
    let report = solve(
        values,
        fixed,
        grid,
        Options {
            relative_tolerance: 2e-13,
            max_cycles: 180,
            ..Options::default()
        },
    )
    .unwrap();
    assert!(report.converged, "{:?}", report.residual_history);
    assert_eq!(report.residual_history.len(), report.iterations + 1);
    assert!(report.residual_history.last().unwrap().relative <= 2e-13);
    report
}

/// Independent, serial SOR reference using the physical (unweighted) stencil.
/// This deliberately does not use implementation helpers from the MG module.
fn sor(values: &mut [f64], fixed: &[bool], grid: Grid) -> f64 {
    for _ in 0..40_000 {
        cells(grid.dims, |i, c| {
            if fixed[i] {
                return;
            }
            let (diagonal, neighbors) = stencil(values, grid, i, c);
            values[i] += 1.6 * (neighbors / diagonal - values[i]);
        });
        let mut residual = 0.0_f64;
        cells(grid.dims, |i, c| {
            if !fixed[i] {
                let (diagonal, neighbors) = stencil(values, grid, i, c);
                residual = residual.max((neighbors - diagonal * values[i]).abs());
            }
        });
        if residual < 2e-11 {
            return residual;
        }
    }
    panic!("independent SOR reference did not converge");
}

fn stencil(values: &[f64], grid: Grid, i: usize, c: [usize; 3]) -> (f64, f64) {
    let mut diagonal = 0.0;
    let mut neighbors = 0.0;
    let strides = [1, grid.dims[0], grid.dims[0] * grid.dims[1]];
    for a in 0..3 {
        if grid.dims[a] == 1 {
            continue;
        }
        let (mut minus, mut plus) = (1.0, 1.0);
        if a == 0
            && let Geometry::Cylindrical { radial_origin } = grid.geometry
        {
            let r = radial_origin / grid.spacing[0] + c[0] as f64;
            if r == 0.0 {
                (minus, plus) = (0.0, 4.0);
            } else {
                (minus, plus) = (1.0 - 0.5 / r, 1.0 + 0.5 / r);
            }
        }
        if c[a] == 0 {
            minus = 0.0;
            if plus != 4.0 {
                plus = 2.0;
            }
        }
        if c[a] + 1 == grid.dims[a] {
            plus = 0.0;
            minus = 2.0;
        }
        let h2 = grid.spacing[a] * grid.spacing[a];
        diagonal += (minus + plus) / h2;
        if minus != 0.0 {
            neighbors += minus / h2 * values[i - strides[a]];
        }
        if plus != 0.0 {
            neighbors += plus / h2 * values[i + strides[a]];
        }
    }
    (diagonal, neighbors)
}

fn compare_sor(initial: &[f64], fixed: &[bool], grid: Grid) -> Vec<f64> {
    let mut mg = initial.to_vec();
    let report = accurate(&mut mg, fixed, grid);
    let mut reference = initial.to_vec();
    let residual = sor(&mut reference, fixed, grid);
    let error = mg
        .iter()
        .zip(&reference)
        .map(|(a, b)| (a - b).abs())
        .fold(0.0_f64, f64::max);
    eprintln!(
        "{:?} discrete: MG cycles={}, relative residual={:.3e}, SOR absolute residual={:.3e}, max difference={:.3e}",
        grid.dims,
        report.iterations,
        report.residual_history.last().unwrap().relative,
        residual,
        error,
    );
    assert!(
        error < 2e-9,
        "MG differs from discrete SOR solution: {error}"
    );
    for i in 0..mg.len() {
        if fixed[i] {
            assert_eq!(mg[i].to_bits(), initial[i].to_bits());
        }
    }
    mg
}

#[test]
fn parallel_plates_are_linear_with_mirror_faces_and_anisotropic_spacing() {
    for dims in [[25, 18, 1], [18, 11, 9], [50, 2, 1]] {
        let grid = Grid {
            dims,
            spacing: [0.13, 0.2, 0.31],
            geometry: Geometry::Planar,
        };
        let count = dims.iter().product();
        let mut potential = vec![127.0; count];
        let mut fixed = vec![false; count];
        cells(dims, |i, [x, _, _]| {
            if x == 0 || x == dims[0] - 1 {
                fixed[i] = true;
                potential[i] = 127.0 + 3.0 * x as f64 / (dims[0] - 1) as f64;
            }
        });
        let result = compare_sor(&potential, &fixed, grid);
        cells(dims, |i, [x, _, _]| {
            let exact = 127.0 + 3.0 * x as f64 / (dims[0] - 1) as f64;
            assert!((result[i] - exact).abs() < 2e-9);
        });
    }
}

#[test]
fn planar_quadrupole_is_discretely_exact() {
    let dims = [35, 26, 1];
    let grid = Grid {
        dims,
        spacing: [2.0 / 34.0, 1.6 / 25.0, 1.0],
        geometry: Geometry::Planar,
    };
    let mut potential = vec![0.0; dims.iter().product()];
    let mut fixed = vec![false; potential.len()];
    let exact = |[x, y, _]: [usize; 3]| {
        let x = -1.0 + x as f64 * grid.spacing[0];
        let y = -0.8 + y as f64 * grid.spacing[1];
        x * x - y * y
    };
    cells(dims, |i, c| {
        if c[0] == 0 || c[1] == 0 || c[0] == dims[0] - 1 || c[1] == dims[1] - 1 {
            fixed[i] = true;
            potential[i] = exact(c);
        }
    });
    accurate(&mut potential, &fixed, grid);
    cells(dims, |i, c| assert!((potential[i] - exact(c)).abs() < 1e-9));
}

#[test]
fn cylindrical_harmonic_polynomial_includes_the_axis() {
    let dims = [33, 24, 1];
    let grid = Grid {
        dims,
        spacing: [1.0 / 32.0, 2.0 / 23.0, 1.0],
        geometry: Geometry::Cylindrical { radial_origin: 0.0 },
    };
    let exact = |[r, z, _]: [usize; 3]| {
        let r = r as f64 * grid.spacing[0];
        let z = -1.0 + z as f64 * grid.spacing[1];
        r * r - 2.0 * z * z
    };
    let mut potential = vec![0.0; dims.iter().product()];
    let mut fixed = vec![false; potential.len()];
    cells(dims, |i, c| {
        if c[0] == dims[0] - 1 || c[1] == 0 || c[1] == dims[1] - 1 {
            fixed[i] = true;
            potential[i] = exact(c);
        }
    });
    let result = compare_sor(&potential, &fixed, grid);
    cells(dims, |i, c| assert!((result[i] - exact(c)).abs() < 2e-9));
}

#[test]
fn coaxial_log_solution_has_second_order_radial_error() {
    let mut errors = Vec::new();
    for n in [17, 33, 65] {
        let dims = [n, 5, 1];
        let h = 1.0 / (n - 1) as f64;
        let grid = Grid {
            dims,
            spacing: [h, 0.2, 1.0],
            geometry: Geometry::Cylindrical { radial_origin: 1.0 },
        };
        let mut potential = vec![0.0; dims.iter().product()];
        let mut fixed = vec![false; potential.len()];
        cells(dims, |i, [r, _, _]| {
            if r == 0 || r == n - 1 {
                fixed[i] = true;
                potential[i] = if r == 0 { 0.0 } else { 1.0 };
            }
        });
        let result = compare_sor(&potential, &fixed, grid);
        let mut error = 0.0_f64;
        cells(dims, |i, [r, _, _]| {
            let exact = (1.0 + r as f64 * h).ln() / 2.0_f64.ln();
            error = error.max((result[i] - exact).abs());
        });
        eprintln!("coaxial r-z n={n}: analytic discretization error={error:.6e}");
        errors.push(error);
    }
    for pair in errors.windows(2) {
        assert!(pair[0] / pair[1] > 3.8, "{errors:?}");
        assert!(pair[0] / pair[1] < 4.2, "{errors:?}");
    }
}

#[test]
fn smooth_planar_solution_has_second_order_error() {
    let mut errors = Vec::new();
    for n in [17, 33, 65] {
        let dims = [n, n, 1];
        let h = 1.0 / (n - 1) as f64;
        let grid = Grid {
            dims,
            spacing: [h, h, 1.0],
            geometry: Geometry::Planar,
        };
        let exact = |[x, y, _]: [usize; 3]| {
            let pi = std::f64::consts::PI;
            (pi * x as f64 * h).sin() * (pi * y as f64 * h).sinh() / pi.sinh()
        };
        let mut potential = vec![0.0; n * n];
        let mut fixed = vec![false; n * n];
        cells(dims, |i, c| {
            if c[0] == 0 || c[1] == 0 || c[0] == n - 1 || c[1] == n - 1 {
                fixed[i] = true;
                potential[i] = exact(c);
            }
        });
        accurate(&mut potential, &fixed, grid);
        let mut error = 0.0_f64;
        cells(dims, |i, c| {
            error = error.max((potential[i] - exact(c)).abs())
        });
        eprintln!("planar n={n}: analytic discretization error={error:.6e}");
        errors.push(error);
    }
    for pair in errors.windows(2) {
        assert!(pair[0] / pair[1] > 3.8, "{errors:?}");
    }
}

#[test]
fn stair_stepped_coaxial_cylinders_match_discrete_sor_not_exact_geometry() {
    let dims = [37, 37, 1];
    let h = 2.0 / 36.0;
    let grid = Grid {
        dims,
        spacing: [h, h, 1.0],
        geometry: Geometry::Planar,
    };
    let mut potential = vec![0.0; dims.iter().product()];
    let mut fixed = vec![false; potential.len()];
    let mut exact = vec![0.0; potential.len()];
    cells(dims, |i, [x, y, _]| {
        let r = ((-1.0 + x as f64 * h).powi(2) + (-1.0 + y as f64 * h).powi(2)).sqrt();
        if r <= 0.22 || r >= 0.8 {
            fixed[i] = true;
            potential[i] = if r <= 0.22 { 0.0 } else { 1.0 };
        } else {
            exact[i] = (r / 0.22).ln() / (0.8_f64 / 0.22).ln();
        }
    });
    let result = compare_sor(&potential, &fixed, grid);
    let error = result
        .iter()
        .zip(&exact)
        .zip(&fixed)
        .filter(|(_, fixed)| !**fixed)
        .map(|((a, b), _)| (a - b).abs())
        .fold(0.0_f64, f64::max);
    eprintln!("stair-stepped coaxial cylinders: analytic geometry error={error:.6e}");
    assert!(error < 0.16);
}

#[test]
fn concentric_spheres_match_the_discrete_solution_and_report_geometry_error() {
    let dims = [33, 33, 33];
    let h = 2.0 / 32.0;
    let grid = Grid {
        dims,
        spacing: [h; 3],
        geometry: Geometry::Planar,
    };
    let mut potential = vec![0.0; dims.iter().product()];
    let mut fixed = vec![false; potential.len()];
    let mut exact = vec![0.0; potential.len()];
    cells(dims, |i, c| {
        let r = c
            .iter()
            .map(|&v| (-1.0 + v as f64 * h).powi(2))
            .sum::<f64>()
            .sqrt();
        if r <= 0.25 || r >= 0.85 {
            fixed[i] = true;
            potential[i] = if r <= 0.25 { 0.0 } else { 1.0 };
        } else {
            exact[i] = (1.0 / 0.25 - 1.0 / r) / (1.0 / 0.25 - 1.0 / 0.85);
        }
    });
    let result = compare_sor(&potential, &fixed, grid);
    let error = result
        .iter()
        .zip(&exact)
        .zip(&fixed)
        .filter(|(_, fixed)| !**fixed)
        .map(|((a, b), _)| (a - b).abs())
        .fold(0.0_f64, f64::max);
    eprintln!("stair-stepped spheres: analytic geometry error={error:.6e}");
    assert!(error < 0.16);
}

#[test]
fn arbitrary_sizes_and_single_node_electrodes_survive_coarsening() {
    let dims = [22, 17, 10];
    let grid = Grid {
        dims,
        spacing: [0.7, 0.9, 1.1],
        geometry: Geometry::Planar,
    };
    let mut potential = vec![0.0; dims.iter().product()];
    let mut fixed = vec![false; potential.len()];
    cells(dims, |i, [x, y, z]| {
        if x == 0 || x == dims[0] - 1 || (x == 11 && y % 3 == 1 && z % 2 == 1) {
            fixed[i] = true;
            potential[i] = if x == 0 {
                -0.4
            } else if x == dims[0] - 1 {
                1.0
            } else {
                0.3
            };
        }
    });
    compare_sor(&potential, &fixed, grid);
}

#[test]
fn convergence_budget_and_input_validation_are_explicit() {
    let grid = Grid {
        dims: [4, 3, 1],
        spacing: [1.0; 3],
        geometry: Geometry::Planar,
    };
    let mut values = vec![0.0; 12];
    let mut mask = vec![false; 12];
    assert!(solve(&mut values, &mask, grid, Options::default()).is_err());
    mask[0] = true;
    values[0] = 1.0;
    let report = solve(
        &mut values,
        &mask,
        grid,
        Options {
            max_cycles: 0,
            ..Options::default()
        },
    )
    .unwrap();
    assert!(!report.converged);
    assert_eq!(report.iterations, 0);
    assert_eq!(report.residual_history.len(), 1);
    assert_eq!(values[0], 1.0);
    assert!(solve(&mut values[..11], &mask, grid, Options::default()).is_err());
    assert!(
        solve(
            &mut values,
            &mask,
            Grid {
                spacing: [0.0; 3],
                ..grid
            },
            Options::default()
        )
        .is_err()
    );
    assert!(
        solve(
            &mut values,
            &mask,
            grid,
            Options {
                relative_tolerance: f64::NAN,
                ..Options::default()
            }
        )
        .is_err()
    );
    values[1] = f64::INFINITY;
    assert!(solve(&mut values, &mask, grid, Options::default()).is_err());
}

#[test]
fn fixed_and_constant_solutions_do_not_need_a_cycle() {
    let grid = Grid {
        dims: [5, 3, 1],
        spacing: [0.3; 3],
        geometry: Geometry::Planar,
    };
    for mask in [vec![true; 15], {
        let mut mask = vec![false; 15];
        mask[7] = true;
        mask
    }] {
        let mut values = vec![17.0; 15];
        let report = solve(&mut values, &mask, grid, Options::default()).unwrap();
        assert!(report.converged);
        assert_eq!(report.iterations, 0);
        assert_eq!(report.residual_history[0].absolute, 0.0);
        assert_eq!(values, vec![17.0; 15]);
    }
}

#[test]
fn skinny_grid_retains_tight_sor_agreement() {
    let grid = Grid {
        dims: [50, 2, 1],
        spacing: [0.13, 0.2, 0.31],
        geometry: Geometry::Planar,
    };
    let mut values = vec![127.0; grid.dims.iter().product()];
    let mut fixed = vec![false; values.len()];
    cells(grid.dims, |i, [x, _, _]| {
        if x == 0 || x == grid.dims[0] - 1 {
            fixed[i] = true;
            values[i] = 127.0 + 3.0 * x as f64 / (grid.dims[0] - 1) as f64;
        }
    });
    let mut reference = values.clone();
    accurate(&mut values, &fixed, grid);
    sor(&mut reference, &fixed, grid);
    let error = values
        .iter()
        .zip(&reference)
        .map(|(a, b)| (a - b).abs())
        .fold(0.0_f64, f64::max);
    assert!(error < 7e-11, "skinny-grid SOR difference: {error}");
}
