use simrep_solve::mg::{Geometry, Grid, Options, solve_poisson};

#[test]
fn manufactured_planar_and_cylindrical_poisson() {
    for geometry in [
        Geometry::Planar,
        Geometry::Cylindrical { radial_origin: 0.0 },
    ] {
        let dims = [19, 23, 1];
        let spacing = [0.13, 0.17, 1.0];
        let grid = Grid {
            dims,
            spacing,
            geometry,
        };
        let mut exact = vec![0.0; dims[0] * dims[1]];
        let mut fixed = vec![false; exact.len()];
        for y in 0..dims[1] {
            for x in 0..dims[0] {
                let i = x + dims[0] * y;
                exact[i] =
                    7.0 - (x as f64 * spacing[0]).powi(2) - 2.0 * (y as f64 * spacing[1]).powi(2);
                // The radial axis is a regularity boundary, not an electrode.
                fixed[i] = x + 1 == dims[0]
                    || y == 0
                    || y + 1 == dims[1]
                    || (x == 0 && geometry == Geometry::Planar);
            }
        }
        let mut values: Vec<_> = exact
            .iter()
            .zip(&fixed)
            .map(|(&v, &e)| if e { v } else { 0.0 })
            .collect();
        let source = vec![
            if geometry == Geometry::Planar {
                6.0
            } else {
                8.0
            };
            values.len()
        ];
        let report = solve_poisson(
            &mut values,
            &fixed,
            &source,
            grid,
            Options {
                relative_tolerance: 1e-12,
                ..Options::default()
            },
        )
        .unwrap();
        assert!(report.converged, "{:?}", report.residual_history);
        for ((&value, &expected), &e) in values.iter().zip(&exact).zip(&fixed) {
            if e {
                assert_eq!(value, expected);
            }
            assert!(
                (value - expected).abs() < 1e-9,
                "{geometry:?}: {value} != {expected}"
            );
        }
        simrep_core::validation::Gate {
            name: if geometry == Geometry::Planar {
                "comparison-poisson-planar"
            } else {
                "comparison-poisson-cylindrical"
            },
            reference: "poisson.rs manufactured_planar_and_cylindrical_poisson",
            metric: "maximum potential error (V)",
            value: values
                .iter()
                .zip(&exact)
                .map(|(v, e)| (v - e).abs())
                .fold(0.0_f64, f64::max),
            datum: 0.0,
            u_num: None,
            u_input: 0.0,
            u_datum: 0.0,
            band: "existing assertion: max error < 1e-9 V; not an uncertainty",
        }
        .record()
        .unwrap();
        // A warm start must measure the nonzero physical RHS as well.
        let warm = solve_poisson(
            &mut values,
            &fixed,
            &source,
            grid,
            Options {
                relative_tolerance: 1e-12,
                ..Options::default()
            },
        )
        .unwrap();
        assert!(warm.converged);
        assert_eq!(warm.iterations, 0);
    }
}

#[test]
fn poisson_source_sign_units_and_validation() {
    let grid = Grid {
        dims: [9, 9, 1],
        spacing: [0.25, 0.25, 1.0],
        geometry: Geometry::Planar,
    };
    let mut values = vec![0.0; 81];
    let fixed: Vec<_> = (0..81)
        .map(|i| i % 9 == 0 || i % 9 == 8 || i / 9 == 0 || i / 9 == 8)
        .collect();
    let source = vec![1.0; 81];
    assert!(solve_poisson(&mut values, &fixed, &source[..80], grid, Options::default()).is_err());
    let report = solve_poisson(&mut values, &fixed, &source, grid, Options::default()).unwrap();
    assert!(report.converged);
    assert!(values[40] > 0.0);
}
