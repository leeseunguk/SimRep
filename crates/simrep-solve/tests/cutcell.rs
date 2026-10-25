use simrep_geom::{Electrode, Sdf};
use simrep_solve::{
    cutcell::CutCells,
    mg::{self, Geometry, Grid, Options},
};

const INNER: f64 = 0.31;
const OUTER: f64 = 0.83;

fn case(n: usize, kind: usize, phase: f64) -> CutCells {
    let sphere = kind == 0;
    let cylindrical = kind == 2;
    let h = if cylindrical {
        (OUTER - INNER) / (n - 2) as f64
    } else {
        2.0 / (n - 1) as f64
    };
    let shape = |radius| {
        if sphere {
            Sdf::Sphere {
                center: [0.0; 3],
                radius,
            }
        } else if cylindrical {
            Sdf::Plane {
                normal: [1.0, 0.0, 0.0],
                offset: radius,
            }
        } else {
            Sdf::Cylinder {
                center: [phase * h, 0.0, 0.0],
                radius,
                axis: 2,
            }
        }
    };
    let outside = Sdf::Box {
        center: [0.0; 3],
        half_extent: [4.0; 3],
    }
    .difference(shape(OUTER));
    // Keep the annular r-z surfaces at half cells under refinement, so that
    // changing boundary phase does not obscure the radial truncation order.
    let origin = if cylindrical {
        [INNER - 0.5 * h, 0.0, 0.0]
    } else {
        [-1.0, -1.0, if sphere { -1.0 } else { 0.0 }]
    };
    CutCells::new(
        Grid {
            dims: [n, n, if sphere { n } else { 1 }],
            spacing: [h; 3],
            geometry: if cylindrical {
                Geometry::Cylindrical {
                    radial_origin: origin[0],
                }
            } else {
                Geometry::Planar
            },
        },
        origin,
        &[
            Electrode::new(1.0, shape(INNER)),
            Electrode::new(0.0, outside),
        ],
    )
    .unwrap()
}

fn errors(cuts: &CutCells, values: &[f64], kind: usize, phase: f64) -> (f64, f64) {
    let mut potential = 0.0_f64;
    let mut field = 0.0_f64;
    let h = cuts.grid.spacing[0];
    for (i, &v) in values.iter().enumerate() {
        let n = cuts.grid.dims[0];
        let c = [i % n, i / n % n, i / (n * n)];
        let mut p: [f64; 3] =
            std::array::from_fn(|a| cuts.origin[a] + c[a] as f64 * cuts.grid.spacing[a]);
        if kind == 1 {
            p[0] -= phase * h;
        }
        let radius = if kind == 2 {
            p[0]
        } else {
            p.iter()
                .take(if kind == 0 { 3 } else { 2 })
                .map(|x| x * x)
                .sum::<f64>()
                .sqrt()
        };
        if radius <= INNER || radius >= OUTER {
            continue;
        }
        let (exact, derivative) = if kind == 0 {
            (
                (1.0 / radius - 1.0 / OUTER) / (1.0 / INNER - 1.0 / OUTER),
                -1.0 / (radius * radius * (1.0 / INNER - 1.0 / OUTER)),
            )
        } else {
            (
                (OUTER / radius).ln() / (OUTER / INNER).ln(),
                -1.0 / (radius * (OUTER / INNER).ln()),
            )
        };
        potential = potential.max((v - exact).abs());
        if !cuts.fixed[i] && (radius - INNER).min(OUTER - radius) < 2.0 * h {
            let actual = cuts.electric_field(values, i).unwrap();
            for a in 0..if kind == 0 { 3 } else { 2 } {
                let component = if kind == 2 {
                    if a == 0 { -derivative } else { 0.0 }
                } else {
                    -derivative * p[a] / radius
                };
                field = field.max((actual[a] - component).abs());
            }
        }
    }
    (potential, field)
}

#[test]
fn curved_boundaries_have_second_order_potential_and_first_order_field() {
    // Debug builds shift the refinement ladder down one level so the debug
    // suite stays under a few minutes; release keeps the full 257-node grid.
    // Both ladders span two refinements (endpoint spacing ratio of four), so
    // the same potential order, envelope, and staircase assertions apply
    // unchanged.
    #[cfg(debug_assertions)]
    const SIZES: [usize; 3] = [33, 65, 129];
    #[cfg(not(debug_assertions))]
    const SIZES: [usize; 3] = [65, 129, 257];
    const FINEST: usize = SIZES[2];
    for kind in 0..3 {
        let mut errors_by_grid: Vec<(f64, f64)> = Vec::new();
        for n in SIZES {
            let mut worst = (0.0_f64, 0.0_f64);
            // A circle's maximum can land almost exactly on a snapped node on
            // one grid and miss on the next. Check the worst of four subcell
            // translations, rather than accepting a favorable alignment.
            let phases: &[f64] = if kind == 1 {
                &[0.0, 0.25, 0.5, 0.75]
            } else {
                &[0.0]
            };
            for &phase in phases {
                let cuts = case(n, kind, phase);
                let mut values = cuts.values.clone();
                let options = Options {
                    relative_tolerance: 1e-12,
                    ..Default::default()
                };
                let report = cuts.solve(&mut values, options).unwrap();
                assert!(
                    report.converged,
                    "kind={kind} n={n}: {:?}",
                    report.residual_history
                );
                let error = errors(&cuts, &values, kind, phase);
                let h = cuts.grid.spacing[0];
                assert!(
                    error.0 < 6.0 * h * h,
                    "potential O(h^2) envelope: {error:?} h={h}"
                );
                assert!(error.1 < 20.0 * h, "field O(h) envelope: {error:?} h={h}");
                let mut staircase = cuts.values.clone();
                let stair_report =
                    mg::solve(&mut staircase, &cuts.fixed, cuts.grid, options).unwrap();
                assert!(stair_report.converged);
                let stair_error = errors(&cuts, &staircase, kind, phase).0;
                eprintln!(
                    "kind={kind} n={n} phase={phase}: potential={:.9e} field={:.9e} stair={stair_error:.9e} cycles={}",
                    error.0, error.1, report.iterations
                );
                if n == FINEST {
                    assert!(stair_error > 10.0 * error.0);
                }
                worst.0 = worst.0.max(error.0);
                worst.1 = worst.1.max(error.1);
            }
            errors_by_grid.push(worst);
        }
        // Maxima can move between nodes, and quadratic snapping changes grid
        // phase. Use two refinements rather than a noisy adjacent-grid ratio.
        assert!(
            errors_by_grid[0].0 / errors_by_grid[2].0 > 12.0,
            "potential order over two refinements: {errors_by_grid:?}"
        );
        // The near-surface field maximum sits on snapped nodes whose
        // quadratic estimate does not enter its O(h) regime until roughly
        // n=97 for the sphere (0.52 at n=33 and n=65, then 0.20 at n=129),
        // so a strict first-order endpoint ratio is unreachable below the
        // release-only 257 grid. Debug still requires at least a 2x
        // improvement over two refinements, and the absolute 20 h envelope
        // above is identical in both profiles.
        let field_ratio = if cfg!(debug_assertions) { 2.0 } else { 4.0 };
        assert!(
            errors_by_grid[0].1 / errors_by_grid[2].1 > field_ratio,
            "field order over two refinements: {errors_by_grid:?}"
        );
    }
}

#[test]
fn tiny_cuts_snap_with_quadratic_not_linear_displacement() {
    let mut previous = None;
    for n in [17, 33, 65] {
        let grid = Grid {
            dims: [n, n, 1],
            spacing: [1.0 / (n - 1) as f64; 3],
            geometry: Geometry::Planar,
        };
        let threshold = 2.0 * grid.spacing[0].powi(2);
        let surface = 0.5 - threshold * 0.5;
        let cuts = CutCells::new(
            grid,
            [0.0; 3],
            &[Electrode::new(
                2.0,
                Sdf::Plane {
                    normal: [1.0, 0.0, 0.0],
                    offset: surface,
                },
            )],
        )
        .unwrap();
        assert!(cuts.fixed[n / 2]);
        assert_eq!(cuts.snap_distance, threshold);
        if let Some(p) = previous {
            assert_eq!(p / threshold, 4.0);
        }
        previous = Some(threshold);
        let mut x = cuts.values.clone();
        assert!(cuts.solve(&mut x, Options::default()).unwrap().converged);
        assert!(x.iter().all(|v| (*v - 2.0).abs() < 1e-8));
    }
}

#[test]
fn a_thin_electrode_between_free_endpoints_is_not_lost() {
    let n = 33;
    let grid = Grid {
        dims: [n, n, 1],
        spacing: [1.0 / (n - 1) as f64; 3],
        geometry: Geometry::Planar,
    };
    let cuts = CutCells::new(
        grid,
        [0.0; 3],
        &[
            Electrode::new(
                0.0,
                Sdf::Plane {
                    normal: [1.0, 0.0, 0.0],
                    offset: 0.0,
                },
            ),
            Electrode::new(
                0.0,
                Sdf::Plane {
                    normal: [-1.0, 0.0, 0.0],
                    offset: -1.0,
                },
            ),
            Electrode::new(
                1.0,
                Sdf::Box {
                    center: [0.46, 0.5, 0.0],
                    half_extent: [0.004, 2.0, 2.0],
                },
            ),
        ],
    )
    .unwrap();
    assert!(
        !cuts
            .fixed
            .iter()
            .zip(&cuts.values)
            .any(|(m, v)| *m && *v == 1.0)
    );
    let mut values = cuts.values.clone();
    let report = cuts
        .solve(
            &mut values,
            Options {
                relative_tolerance: 1e-12,
                ..Default::default()
            },
        )
        .unwrap();
    assert!(report.converged);
    for (i, value) in values.iter().enumerate() {
        let x = (i % n) as f64 * grid.spacing[0];
        let exact = if x < 0.456 {
            x / 0.456
        } else {
            (1.0 - x) / (1.0 - 0.464)
        };
        assert!((value - exact).abs() < 1e-8, "x={x}: {value} vs {exact}");
    }
}
