//! Reproducible CPU-only adaptive study, without external fixture files.
//! cargo run --release -p simrep-cli --example adaptive_study
use simrep_geom::{Electrode, Sdf};
use simrep_solve::adaptive::{AdaptiveGrid, Region, Solution, interpolate};
use simrep_solve::cutcell::CutCells;
use simrep_solve::mg::{Geometry, Grid, Options};
use std::error::Error;

#[path = "../src/auto_refine.rs"]
mod auto;

type Result<T> = std::result::Result<T, Box<dyn Error>>;
const INNER: f64 = 0.31;
const OUTER: f64 = 0.83;

fn options() -> Options {
    Options {
        relative_tolerance: 1e-11,
        max_cycles: 400,
        ..Default::default()
    }
}

fn point(cuts: &CutCells, i: usize) -> [f64; 3] {
    let d = cuts.grid.dims;
    let c = [i % d[0], i / d[0] % d[1], i / (d[0] * d[1])];
    std::array::from_fn(|a| cuts.origin[a] + c[a] as f64 * cuts.grid.spacing[a])
}

fn sample(adaptive: &AdaptiveGrid, solution: &Solution, p: [f64; 3]) -> f64 {
    for (patch, values) in adaptive.patches.iter().zip(&solution.patches) {
        if (0..3).all(|a| {
            p[a] >= patch.cuts.origin[a]
                && p[a]
                    <= patch.cuts.origin[a]
                        + (patch.cuts.grid.dims[a] - 1) as f64 * patch.cuts.grid.spacing[a]
        }) {
            return interpolate(values, patch.cuts.grid, patch.cuts.origin, p);
        }
    }
    interpolate(&solution.base, adaptive.base.grid, adaptive.base.origin, p)
}

fn profile(case: &str, label: &str, adaptive: &AdaptiveGrid, result: &Solution) -> Result<()> {
    println!(
        "profile,{case},{label},iterations={},setup_ms={:.6},base_snap_mm={:.9e},patch_snap_mm={:?},phases={}",
        result.report.iterations,
        result.report.timings.setup.as_secs_f64() * 1000.0,
        adaptive.base.snap_distance,
        adaptive
            .patches
            .iter()
            .map(|p| p.cuts.snap_distance)
            .collect::<Vec<_>>(),
        serde_json::to_string(&result.profile)?
    );
    Ok(())
}

fn sphere_errors(cuts: &CutCells, values: &[f64]) -> (f64, f64, f64) {
    let mut potential = 0.0_f64;
    let mut field = 0.0_f64;
    for (i, &v) in values.iter().enumerate() {
        let p = point(cuts, i);
        let r = p.iter().map(|v| v * v).sum::<f64>().sqrt();
        if r <= INNER || r >= OUTER {
            continue;
        }
        let exact = (1.0 / r - 1.0 / OUTER) / (1.0 / INNER - 1.0 / OUTER);
        potential = potential.max((v - exact).abs());
        if let Some(actual) = cuts.electric_field(values, i) {
            for a in 0..3 {
                let exact = p[a] / (r.powi(3) * (1.0 / INNER - 1.0 / OUTER));
                field = field.max((actual[a] - exact).abs());
            }
        }
    }
    let i = (((INNER + OUTER) * 0.5) / cuts.grid.spacing[0]).round() as usize;
    let r = cuts.origin[0] + i as f64 * cuts.grid.spacing[0];
    let e = cuts.electric_field(values, i).unwrap()[0];
    let passband = e * r * r / (INNER + OUTER);
    (potential, field, passband)
}

fn sphere() -> Result<()> {
    let electrodes = [
        Electrode::new(
            1.0,
            Sdf::Sphere {
                center: [0.0; 3],
                radius: INNER,
            },
        ),
        Electrode::new(
            0.0,
            Sdf::Box {
                center: [0.0; 3],
                half_extent: [4.0; 3],
            }
            .difference(Sdf::Sphere {
                center: [0.0; 3],
                radius: OUTER,
            }),
        ),
    ];
    let make = |n| {
        CutCells::new(
            Grid {
                dims: [n; 3],
                spacing: [1.0 / (n - 1) as f64; 3],
                geometry: Geometry::Planar,
            },
            [0.0; 3],
            &electrodes,
        )
    };
    println!(
        "case,method,stored_nodes,free_unknowns,scalar_bytes,matrix_bytes,compute_ms,potential_error_v,field_error_v_per_mm,passband_ev_per_e"
    );
    for n in [17, 33, 65] {
        let cuts = make(n)?;
        let mut values = cuts.values.clone();
        let report = cuts.solve(&mut values, options())?;
        assert!(report.converged);
        let (v, e, pass) = sphere_errors(&cuts, &values);
        println!(
            "sphere,uniform{n},{},{},{},0,{:.6},{v:.9e},{e:.9e},{pass:.9e}",
            values.len(),
            cuts.fixed.iter().filter(|&&f| !f).count(),
            values.len() * 8,
            report.timings.solve.as_secs_f64() * 1000.0
        );
        println!("iterations,sphere,uniform{n},{}", report.iterations);
    }
    let adaptive = AdaptiveGrid::new(
        make(17)?,
        &[Region {
            bounds_mm: [[0.0; 3], [0.875; 3]],
            factor: 2,
        }],
        &electrodes,
    )?;
    let result = adaptive.solve(options())?;
    assert!(
        result.report.converged,
        "{:?}",
        result.report.residual_history
    );
    let (v, e, pass) = sphere_errors(&adaptive.patches[0].cuts, &result.patches[0]);
    let nodes = result.base.len() + result.patches.iter().map(Vec::len).sum::<usize>();
    println!(
        "sphere,adaptive17x2,{nodes},{},{},{},{:.6},{v:.9e},{e:.9e},{pass:.9e}",
        result.unknowns,
        nodes * 8,
        result.matrix_bytes,
        result.report.timings.solve.as_secs_f64() * 1000.0
    );
    profile("sphere", "adaptive17x2", &adaptive, &result)?;
    let base = make(17)?;
    let selection = auto::select(
        base.grid,
        base.origin,
        &electrodes
            .iter()
            .map(|e| e.shape.clone())
            .collect::<Vec<_>>(),
        &[auto::GapHint {
            electrodes: [1, 2],
            bracket_mm: [Some(OUTER - INNER); 2],
            points_mm: Some([[INNER, 0.0, 0.0], [OUTER, 0.0, 0.0]]),
            resolved: true,
        }],
        2,
    )?;
    println!(
        "auto_selection,sphere,{}",
        serde_json::to_string(&selection)?
    );
    let adaptive = AdaptiveGrid::new(base, &selection.regions(), &electrodes)?;
    let result = adaptive.solve(options())?;
    assert!(result.report.converged);
    assert_eq!(adaptive.patches.len(), 1);
    let (v, e, pass) = sphere_errors(&adaptive.patches[0].cuts, &result.patches[0]);
    println!(
        "sphere,auto17x2,{},{},{},{},{:.6},{v:.9e},{e:.9e},{pass:.9e}",
        selection.stored_nodes,
        result.unknowns,
        selection.stored_nodes * 8,
        result.matrix_bytes,
        result.report.timings.solve.as_secs_f64() * 1000.0
    );
    profile("sphere", "auto17x2", &adaptive, &result)?;
    println!(
        "sphere_exact_passband,{:.12e}",
        INNER * OUTER / (OUTER * OUTER - INNER * INNER)
    );
    Ok(())
}

fn einzel() -> Result<()> {
    // The original axial ranges and radius are represented as 3D Cartesian
    // cylindrical electrodes. The square outer domain is not the r=19 mirror.
    let tube = |lo: f64, hi: f64| {
        Sdf::Box {
            center: [0.5 * (lo + hi), 0.0, 0.0],
            half_extent: [0.5 * (hi - lo), 100.0, 100.0],
        }
        .difference(Sdf::Cylinder {
            center: [0.0; 3],
            radius: 18.0,
            axis: 0,
        })
    };
    let electrodes = [
        Electrode::new(0.0, tube(-2.0, 28.0)),
        Electrode::new(110.0, tube(30.0, 56.0)),
        Electrode::new(0.0, tube(58.0, 92.0)),
    ];
    let origin = [0.0, -20.0, -20.0];
    let make = |factor: usize| {
        CutCells::new(
            Grid {
                dims: [45 * factor + 1, 20 * factor + 1, 20 * factor + 1],
                spacing: [2.0 / factor as f64; 3],
                geometry: Geometry::Planar,
            },
            origin,
            &electrodes,
        )
    };
    let reference = make(4)?;
    let mut reference_values = reference.values.clone();
    let rr = reference.solve(&mut reference_values, options())?;
    assert!(rr.converged);
    let probes = [
        [29.0, 0.0, 0.0],
        [29.0, 10.0, 0.0],
        [29.0, 16.0, 0.0],
        [31.0, 16.0, 0.0],
        [43.0, 0.0, 0.0],
        [57.0, 0.0, 0.0],
        [57.0, 16.0, 0.0],
    ];
    let error = |sample: &dyn Fn([f64; 3]) -> f64| {
        probes
            .iter()
            .map(|&p| (sample(p) - interpolate(&reference_values, reference.grid, origin, p)).abs())
            .fold(0.0_f64, f64::max)
    };
    println!(
        "einzel,reference4,{},{},{},0,{:.6},0,NA,NA",
        reference_values.len(),
        reference.fixed.iter().filter(|&&f| !f).count(),
        reference_values.len() * 8,
        rr.timings.solve.as_secs_f64() * 1000.0
    );
    for factor in [1, 2] {
        let cuts = make(factor)?;
        let mut values = cuts.values.clone();
        let report = cuts.solve(&mut values, options())?;
        assert!(report.converged);
        let err = error(&|p| interpolate(&values, cuts.grid, origin, p));
        println!(
            "einzel,uniform{factor},{},{},{},0,{:.6},{err:.9e},NA,NA",
            values.len(),
            cuts.fixed.iter().filter(|&&f| !f).count(),
            values.len() * 8,
            report.timings.solve.as_secs_f64() * 1000.0
        );
        println!("iterations,einzel,uniform{factor},{}", report.iterations);
        for p in probes {
            println!(
                "uniform_probe,einzel,uniform{factor},{p:?},v={:.9e},reference_v={:.9e}",
                interpolate(&values, cuts.grid, origin, p),
                interpolate(&reference_values, reference.grid, origin, p)
            );
        }
    }
    let regions = [
        Region {
            bounds_mm: [[24.0, -20.0, -20.0], [34.0, 20.0, 20.0]],
            factor: 2,
        },
        Region {
            bounds_mm: [[52.0, -20.0, -20.0], [62.0, 20.0, 20.0]],
            factor: 2,
        },
    ];
    let adaptive = AdaptiveGrid::new(make(1)?, &regions, &electrodes)?;
    let result = adaptive.solve(options())?;
    assert!(
        result.report.converged,
        "{:?}",
        result.report.residual_history
    );
    let err = error(&|p| sample(&adaptive, &result, p));
    let nodes = result.base.len() + result.patches.iter().map(Vec::len).sum::<usize>();
    println!(
        "einzel,adaptive1x2,{nodes},{},{},{},{:.6},{err:.9e},NA,NA",
        result.unknowns,
        nodes * 8,
        result.matrix_bytes,
        result.report.timings.solve.as_secs_f64() * 1000.0
    );
    profile("einzel", "adaptive1x2", &adaptive, &result)?;
    for p in probes {
        println!(
            "einzel_probe,{p:?},adaptive_v={:.9e},reference_v={:.9e}",
            sample(&adaptive, &result, p),
            interpolate(&reference_values, reference.grid, origin, p)
        );
    }
    // Move only the internal interfaces, preserving fine h and transverse
    // extent, to distinguish placement error from the fine surface mask.
    let wider = [
        Region {
            bounds_mm: [[20.0, -20.0, -20.0], [38.0, 20.0, 20.0]],
            factor: 2,
        },
        Region {
            bounds_mm: [[48.0, -20.0, -20.0], [66.0, 20.0, 20.0]],
            factor: 2,
        },
    ];
    let wider_grid = AdaptiveGrid::new(make(1)?, &wider, &electrodes)?;
    let wider_result = wider_grid.solve(options())?;
    assert!(wider_result.report.converged);
    println!(
        "einzel,wider1x2,{},{},{},{},{:.6},{:.9e},NA,NA",
        wider_result.base.len() + wider_result.patches.iter().map(Vec::len).sum::<usize>(),
        wider_result.unknowns,
        (wider_result.base.len() + wider_result.patches.iter().map(Vec::len).sum::<usize>()) * 8,
        wider_result.matrix_bytes,
        wider_result.report.timings.solve.as_secs_f64() * 1000.0,
        error(&|p| sample(&wider_grid, &wider_result, p))
    );
    profile("einzel", "wider1x2", &wider_grid, &wider_result)?;
    let base = make(1)?;
    let gaps = [28.0, 56.0].map(|x| auto::GapHint {
        electrodes: [1 + usize::from(x > 28.0), 2 + usize::from(x > 28.0)],
        bracket_mm: [Some(2.0); 2],
        points_mm: Some([[x, 18.0, 0.0], [x + 2.0, 18.0, 0.0]]),
        resolved: true,
    });
    let selection = auto::select(
        base.grid,
        origin,
        &electrodes
            .iter()
            .map(|e| e.shape.clone())
            .collect::<Vec<_>>(),
        &gaps,
        2,
    )?;
    println!(
        "auto_selection,einzel,{}",
        serde_json::to_string(&selection)?
    );
    let auto_grid = AdaptiveGrid::new(base, &selection.regions(), &electrodes)?;
    let auto_result = auto_grid.solve(options())?;
    assert!(auto_result.report.converged);
    println!(
        "einzel,auto1x2,{},{},{},{},{:.6},{:.9e},NA,NA",
        selection.stored_nodes,
        auto_result.unknowns,
        selection.stored_nodes * 8,
        auto_result.matrix_bytes,
        auto_result.report.timings.solve.as_secs_f64() * 1000.0,
        error(&|p| sample(&auto_grid, &auto_result, p))
    );
    profile("einzel", "auto1x2", &auto_grid, &auto_result)?;
    // Same surfaces, axial extent and fine h; only the outer boundary differs.
    let cylindrical: Vec<_> = electrodes
        .iter()
        .map(|e| {
            let shape = e.shape.clone();
            Electrode::new(e.voltage, move |p: [f64; 3]| {
                shape.distance([p[1], p[0], 0.0])
            })
        })
        .collect();
    let cyl = CutCells::new(
        Grid {
            dims: [39, 181, 1],
            spacing: [0.5; 3],
            geometry: Geometry::Cylindrical { radial_origin: 0.0 },
        },
        [0.0; 3],
        &cylindrical,
    )?;
    let mut values = cyl.values.clone();
    assert!(cyl.solve(&mut values, options())?.converged);
    println!(
        "einzel_boundary,cartesian_square_vs_cylindrical_r19,max_probe_difference_v={:.9e}",
        error(&|p| interpolate(&values, cyl.grid, cyl.origin, [p[1].hypot(p[2]), p[0], 0.0]))
    );
    Ok(())
}

fn main() -> Result<()> {
    sphere()?;
    einzel()
}
