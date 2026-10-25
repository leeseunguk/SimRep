//! Original STEP/IGES fixtures, analytic solutions, and a fine ring reference.
use std::error::Error;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use simrep_cad::{ImportOptions, Mode, read_cad, write_fixture};
use simrep_geom::{Electrode, Point, Sdf, SignedDistance};
use simrep_solve::cutcell::CutCells;
use simrep_solve::mg::{Geometry, Grid, Options};

fn ring(p: Point) -> f64 {
    (p[0].hypot(p[1]) - 0.55).hypot(p[2]) - 0.19
}

fn outer(kind: u8) -> impl SignedDistance {
    move |p: Point| {
        if kind == 1 {
            0.83 - p[0].hypot(p[1])
        } else {
            let r = p.iter().map(|x| x * x).sum::<f64>().sqrt();
            if kind == 2 { 0.95 - r } else { 0.83 - r }
        }
    }
}

fn solve(
    shape: Arc<dyn SignedDistance>,
    kind: u8,
    n: usize,
) -> Result<(Vec<f64>, Duration, Duration), Box<dyn Error>> {
    let start = Instant::now();
    let grid = Grid {
        dims: [n, n, if kind == 1 { 1 } else { n }],
        spacing: [2.0 / (n - 1) as f64; 3],
        geometry: Geometry::Planar,
    };
    let electrode = Electrode {
        voltage: 1.0,
        shape,
    };
    let cuts = CutCells::new(
        grid,
        [-1.0, -1.0, if kind == 1 { 0.0 } else { -1.0 }],
        &[electrode, Electrode::new(0.0, outer(kind))],
    )?;
    let mut values = cuts.values.clone();
    let prep = start.elapsed();
    let report = cuts.solve(
        &mut values,
        Options {
            relative_tolerance: 1e-11,
            max_cycles: 150,
            ..Default::default()
        },
    )?;
    assert!(report.converged);
    Ok((values, prep + report.timings.setup, report.timings.solve))
}

fn main() -> Result<(), Box<dyn Error>> {
    let dir = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("target/cad-accuracy"));
    std::fs::create_dir_all(&dir)?;
    for (kind, name, n) in [(0, "sphere", 33), (1, "cylinder", 65), (2, "ring", 25)] {
        let analytic: Arc<dyn SignedDistance> = match kind {
            0 => Arc::new(Sdf::Sphere {
                center: [0.0; 3],
                radius: 0.31,
            }),
            1 => Arc::new(Sdf::Cylinder {
                center: [0.0; 3],
                radius: 0.31,
                axis: 2,
            }),
            _ => Arc::new(ring),
        };
        let reference_n = 4 * (n - 1) + 1;
        let fine = if kind == 2 {
            let (values, prep, compute) = solve(Arc::clone(&analytic), kind, reference_n)?;
            println!(
                "{name} reference n={reference_n} preparation_ms={:.3} compute_ms={:.3}",
                prep.as_secs_f64() * 1000.0,
                compute.as_secs_f64() * 1000.0
            );
            Some(values)
        } else {
            None
        };
        let same_grid = solve(Arc::clone(&analytic), kind, n)?.0;
        for extension in ["step", "iges"] {
            let path = dir.join(format!("{name}.{extension}"));
            write_fixture(&path, kind, "AP242DIS", "MM")?;
            let mut errors = Vec::new();
            for mode in [Mode::Exact, Mode::Tessellated] {
                let started = Instant::now();
                let part = read_cad(
                    &path,
                    ImportOptions {
                        mode,
                        band_mm: 4.0 / (n - 1) as f64,
                        // Deliberately visible chordal error at the same solve grid.
                        chordal_tolerance_mm: 0.02,
                        ..Default::default()
                    },
                )?
                .remove(0);
                let load = started.elapsed();
                let triangles = part.triangle_count;
                let (values, prep, compute) = solve(Arc::new(part), kind, n)?;
                let mut max = 0.0_f64;
                let mut same = 0.0_f64;
                let mut sum = 0.0;
                let mut count = 0;
                for (i, &value) in values.iter().enumerate() {
                    let c = [i % n, i / n % n, i / (n * n)];
                    let p = [
                        2.0 * c[0] as f64 / (n - 1) as f64 - 1.0,
                        2.0 * c[1] as f64 / (n - 1) as f64 - 1.0,
                        if kind == 1 {
                            0.0
                        } else {
                            2.0 * c[2] as f64 / (n - 1) as f64 - 1.0
                        },
                    ];
                    if analytic.distance(p) <= 0.0 || outer(kind).distance(p) <= 0.0 {
                        continue;
                    }
                    let r = if kind == 1 {
                        p[0].hypot(p[1])
                    } else {
                        p.iter().map(|x| x * x).sum::<f64>().sqrt()
                    };
                    let expected = if let Some(fine) = &fine {
                        fine[4 * c[0] + reference_n * (4 * c[1] + reference_n * 4 * c[2])]
                    } else if kind == 1 {
                        (0.83 / r).ln() / (0.83_f64 / 0.31).ln()
                    } else {
                        (1.0 / r - 1.0 / 0.83) / (1.0 / 0.31 - 1.0 / 0.83)
                    };
                    let error = (value - expected).abs();
                    max = max.max(error);
                    sum += error * error;
                    same = same.max((value - same_grid[i]).abs());
                    count += 1;
                }
                let rms = (sum / count as f64).sqrt();
                println!(
                    "{name}.{extension} {mode:?} n={n} triangles={triangles} max_v={max:.9e} rms_v={rms:.9e} vs_analytic_sdf_v={same:.9e} preparation_ms={:.3} compute_ms={:.3}",
                    (load + prep).as_secs_f64() * 1000.0,
                    compute.as_secs_f64() * 1000.0
                );
                if mode == Mode::Exact {
                    assert!(same < 2e-6, "B-rep differs from analytic SDF");
                }
                errors.push(rms);
            }
            assert!(
                errors[0] < errors[1],
                "{name}.{extension}: exact must beat tessellation: {errors:?}"
            );
        }
    }
    write_fixture(dir.join("pair.step"), 3, "AP242DIS", "MM")?;
    Ok(())
}
