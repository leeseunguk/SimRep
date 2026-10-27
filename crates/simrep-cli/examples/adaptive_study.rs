//! Reproducible CPU-only adaptive study, without external fixture files.
//! cargo run --release -p simrep-cli --example adaptive_study
use simrep_geom::{Electrode, Sdf};
use simrep_solve::adaptive::{AdaptiveGrid, Region, Solution, interpolate};
use simrep_solve::cutcell::CutCells;
use simrep_solve::mg::{Geometry, Grid, Options};
use std::error::Error;

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
    for p in probes {
        println!(
            "einzel_probe,{p:?},adaptive_v={:.9e},reference_v={:.9e}",
            sample(&adaptive, &result, p),
            interpolate(&reference_values, reference.grid, origin, p)
        );
    }
    Ok(())
}

fn main() -> Result<()> {
    sphere()?;
    einzel()
}
