use simrep_geom::Sdf;
use simrep_solve::{
    dielectric::{Permittivity, Region},
    mg::{self, Geometry, Grid, Options},
};

fn options() -> Options {
    Options {
        relative_tolerance: 1e-12,
        max_cycles: 200,
        ..Default::default()
    }
}

fn plate(n: usize, nz: usize, cells: bool) -> f64 {
    let grid = Grid {
        dims: [n, 5, nz],
        spacing: [1.0 / (n - 1) as f64, 0.25, 0.2],
        geometry: Geometry::Planar,
    };
    let count = grid.dims.iter().product();
    let fixed: Vec<_> = (0..count).map(|i| i % n == 0 || i % n == n - 1).collect();
    let mut values: Vec<_> = (0..count)
        .map(|i| if i % n == n - 1 { 1.0 } else { 0.0 })
        .collect();
    let interface = if cells { 0.5 } else { 1.0 / 3.0 };
    let shape = Sdf::Plane {
        normal: [1.0, 0.0, 0.0],
        offset: interface,
    };
    let material = if cells {
        let d = grid.dims.map(|n| (n - 1).max(1));
        let relative: Vec<_> = (0..d.iter().product())
            .map(|i| {
                if (i % d[0]) as f64 + 0.5 < (n - 1) as f64 * interface {
                    3.0
                } else {
                    1.0
                }
            })
            .collect();
        Permittivity::from_cells(grid, &fixed, &relative).unwrap()
    } else {
        Permittivity::from_regions(
            grid,
            &fixed,
            [0.0; 3],
            1.0,
            &[Region {
                relative_permittivity: 3.0,
                shape: &shape,
            }],
        )
        .unwrap()
    };
    let report = material.solve(&mut values, options()).unwrap();
    assert!(report.converged, "{:?}", report.residual_history);
    let error = values
        .iter()
        .enumerate()
        .map(|(i, &v)| {
            let x = (i % n) as f64 * grid.spacing[0];
            let exact = (x.min(interface) / 3.0 + (x - interface).max(0.0))
                / (interface / 3.0 + 1.0 - interface);
            (v - exact).abs()
        })
        .fold(0.0_f64, f64::max);
    if cells {
        // One V at the interface, and a constant normal displacement on both
        // sides. The electric field itself has the required factor-three jump.
        let flux: Vec<_> = (0..n - 1)
            .map(|x| {
                let epsilon = if x < (n - 1) / 2 { 3.0 } else { 1.0 };
                epsilon * (values[x + 1] - values[x]) / grid.spacing[0]
            })
            .collect();
        assert!(flux.iter().all(|d| (*d - 1.5).abs() < 1e-8));
    }
    println!(
        "plate nz={nz} cells={cells} n={n}: max={error:.9e}, compute={:.6}s",
        report.timings.solve.as_secs_f64()
    );
    error
}

#[test]
fn parallel_plate_interface_flux_and_grid_convergence() {
    for nz in [1, 4] {
        assert!(plate(33, nz, true) < 1e-9);
        let errors = [17, 33, 65].map(|n| plate(n, nz, false));
        let order = (errors[1] / errors[2]).log2();
        println!("plate nz={nz} nodal-interface order={order:.6}");
        assert!(errors[2] < errors[1] && errors[1] < errors[0]);
        assert!(order > 0.8 && order < 1.2);
    }
}

fn coax(n: usize) -> f64 {
    let grid = Grid {
        dims: [n, 5, 1],
        spacing: [2.0 / (n - 1) as f64, 0.25, 1.0],
        geometry: Geometry::Cylindrical { radial_origin: 1.0 },
    };
    let fixed: Vec<_> = (0..n * 5).map(|i| i % n == 0 || i % n == n - 1).collect();
    let mut values: Vec<_> = (0..n * 5)
        .map(|i| if i % n == 0 { 1.0 } else { 0.0 })
        .collect();
    let eps: Vec<_> = (0..(n - 1) * 4)
        .map(|i| if i % (n - 1) < (n - 1) / 2 { 4.0 } else { 1.0 })
        .collect();
    let material = Permittivity::from_cells(grid, &fixed, &eps).unwrap();
    let report = material.solve(&mut values, options()).unwrap();
    assert!(report.converged);
    let resistance = 2.0_f64.ln() / 4.0 + 1.5_f64.ln();
    let error = values
        .iter()
        .enumerate()
        .map(|(i, v)| {
            let r = 1.0 + (i % n) as f64 * grid.spacing[0];
            let exact = if r <= 2.0 {
                ((2.0 / r).ln() / 4.0 + 1.5_f64.ln()) / resistance
            } else {
                (3.0 / r).ln() / resistance
            };
            (v - exact).abs()
        })
        .fold(0.0_f64, f64::max);
    println!(
        "coax n={n}: max={error:.9e}, compute={:.6}s",
        report.timings.solve.as_secs_f64()
    );
    error
}

#[test]
fn coaxial_dielectric_sleeve_is_second_order() {
    let errors = [17, 33, 65].map(coax);
    let order = (errors[1] / errors[2]).log2();
    println!("coax order={order:.6}");
    assert!(errors[1] < errors[0] / 3.5 && errors[2] < errors[1] / 3.5);
    assert!(order > 1.9 && order < 2.1);
    simrep_core::validation::record_order(
        "comparison-dielectric-coax",
        "dielectric.rs coaxial_dielectric_sleeve_is_second_order",
        errors,
        2.0,
    );
}

fn sphere(n: usize) -> f64 {
    let h = 2.0 / (n - 1) as f64;
    let grid = Grid {
        dims: [n; 3],
        spacing: [h; 3],
        geometry: Geometry::Planar,
    };
    let radius = 0.37;
    let epsilon = 8.0;
    let shape = Sdf::Sphere {
        center: [0.0; 3],
        radius,
    };
    let exact = |p: [f64; 3]| {
        let r = p.iter().map(|x| x * x).sum::<f64>().sqrt();
        if r <= radius {
            -3.0 * p[0] / (epsilon + 2.0)
        } else {
            -p[0] + (epsilon - 1.0) / (epsilon + 2.0) * radius.powi(3) * p[0] / r.powi(3)
        }
    };
    let point = |i: usize| {
        [
            (i % n) as f64 * h - 1.0,
            (i / n % n) as f64 * h - 1.0,
            (i / (n * n)) as f64 * h - 1.0,
        ]
    };
    let fixed: Vec<_> = (0..n * n * n)
        .map(|i| {
            [i % n, i / n % n, i / (n * n)]
                .iter()
                .any(|&c| c == 0 || c == n - 1)
        })
        .collect();
    let mut values: Vec<_> = fixed
        .iter()
        .enumerate()
        .map(|(i, &e)| if e { exact(point(i)) } else { 0.0 })
        .collect();
    let material = Permittivity::from_regions(
        grid,
        &fixed,
        [-1.0; 3],
        1.0,
        &[Region {
            relative_permittivity: epsilon,
            shape: &shape,
        }],
    )
    .unwrap();
    let report = material.solve(&mut values, options()).unwrap();
    assert!(report.converged, "{:?}", report.residual_history);
    let rms = (values
        .iter()
        .enumerate()
        .map(|(i, &v)| (v - exact(point(i))).powi(2))
        .sum::<f64>()
        / values.len() as f64)
        .sqrt();
    let center = n * n * (n / 2) + n * (n / 2) + n / 2;
    let field = -(values[center + 1] - values[center - 1]) / (2.0 * h);
    println!(
        "sphere n={n}: rms={rms:.9e}, Ecenter={field:.9e}, theory=0.3, compute={:.6}s",
        report.timings.solve.as_secs_f64()
    );
    rms
}

#[test]
fn dielectric_sphere_converges_to_uniform_field_solution() {
    let errors = [17, 33, 65].map(sphere);
    let order = (errors[1] / errors[2]).log2();
    println!("sphere RMS order={order:.6}");
    assert!(errors[2] < errors[1] && errors[1] < errors[0]);
    assert!(order > 0.5);
}

#[test]
fn variable_dielectric_free_charge_and_uniform_laplace_regression() {
    let grid = Grid {
        dims: [33, 5, 1],
        spacing: [1.0 / 32.0, 0.25, 1.0],
        geometry: Geometry::Planar,
    };
    let fixed: Vec<_> = (0..165).map(|i| i % 33 == 0 || i % 33 == 32).collect();
    let exact: Vec<_> = (0..165)
        .map(|i| {
            let x = (i % 33) as f64 / 32.0;
            if x <= 0.5 {
                x * x
            } else {
                (x * x - 0.25) / 3.0 + 0.25
            }
        })
        .collect();
    let initial: Vec<_> = exact
        .iter()
        .zip(&fixed)
        .map(|(&v, &e)| if e { v } else { 0.0 })
        .collect();
    let relative: Vec<_> = (0..128)
        .map(|i| if i % 32 < 16 { 1.0 } else { 3.0 })
        .collect();
    let material = Permittivity::from_cells(grid, &fixed, &relative).unwrap();
    let mut values = initial.clone();
    assert!(
        material
            .solve_poisson(&mut values, &[-2.0; 165], options())
            .unwrap()
            .converged
    );
    assert!(values.iter().zip(&exact).all(|(v, e)| (v - e).abs() < 1e-9));
    assert!(
        material
            .solve_poisson(&mut values, &[-2.0; 165], options())
            .unwrap()
            .iterations
            == 0
    );
    let unit = Permittivity::from_nodes(grid, &fixed, &[1.0; 165]).unwrap();
    let mut ordinary = initial.clone();
    let mut dielectric = initial;
    mg::solve(&mut ordinary, &fixed, grid, options()).unwrap();
    unit.solve(&mut dielectric, options()).unwrap();
    assert_eq!(ordinary, dielectric);
}

#[test]
fn invalid_material_inputs_are_rejected_without_mutation() {
    let grid = Grid {
        dims: [5, 5, 1],
        spacing: [1.0; 3],
        geometry: Geometry::Planar,
    };
    let mut fixed = vec![false; 25];
    fixed[0] = true;
    for bad in [0.0, -1.0, f64::NAN, f64::INFINITY] {
        assert!(Permittivity::from_nodes(grid, &fixed, &[bad; 25]).is_err());
        assert!(Permittivity::from_cells(grid, &fixed, &[bad; 16]).is_err());
    }
    assert!(Permittivity::from_cells(grid, &fixed, &[1.0; 25]).is_err());
    assert!(Permittivity::from_nodes(grid, &fixed, &[1.0; 24]).is_err());
    let material = Permittivity::from_nodes(grid, &fixed, &[2.0; 25]).unwrap();
    let mut values = vec![7.0; 24];
    assert!(material.solve(&mut values, options()).is_err());
    assert_eq!(values, vec![7.0; 24]);
}

#[test]
fn dielectric_free_charge_at_cylindrical_axis_uses_physical_not_weighted_source() {
    let grid = Grid {
        dims: [17, 19, 1],
        spacing: [0.125, 0.2, 1.0],
        geometry: Geometry::Cylindrical { radial_origin: 0.0 },
    };
    let fixed: Vec<_> = (0..17 * 19)
        .map(|i| i % 17 == 16 || i / 17 == 0 || i / 17 == 18)
        .collect();
    let exact: Vec<_> = (0..17 * 19)
        .map(|i| 7.0 - ((i % 17) as f64 * 0.125).powi(2) - 2.0 * ((i / 17) as f64 * 0.2).powi(2))
        .collect();
    let mut values: Vec<_> = exact
        .iter()
        .zip(&fixed)
        .map(|(&v, &e)| if e { v } else { 0.0 })
        .collect();
    let material = Permittivity::from_nodes(grid, &fixed, &vec![2.0; values.len()]).unwrap();
    assert!(
        material
            .solve_poisson(&mut values, &vec![16.0; exact.len()], options())
            .unwrap()
            .converged
    );
    assert!(values.iter().zip(&exact).all(|(v, e)| (v - e).abs() < 1e-9));
}
