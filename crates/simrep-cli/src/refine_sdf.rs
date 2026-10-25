use std::time::Instant;

use clap::{Args, ValueEnum};
use simrep_geom::{Electrode, Sdf};
use simrep_solve::{
    cutcell::CutCells,
    gpu::{Gpu, GpuResult},
    mg::{self, Geometry, Grid, Options},
};

use crate::{Backend, Device, parse_grid_size, parse_tolerance};

#[derive(Clone, Copy, Debug, ValueEnum)]
enum TestGeometry {
    Sphere,
    Cylinder,
    Cylindrical,
}

#[derive(Args)]
pub struct SdfOptions {
    #[arg(long, value_enum, default_value = "sphere")]
    geometry: TestGeometry,
    #[arg(long, default_value_t = 65, value_parser = parse_grid_size)]
    n: usize,
    #[arg(long, value_enum, default_value = "cpu")]
    device: Device,
    #[arg(long, value_enum, default_value = "vulkan")]
    backend: Backend,
    #[arg(long, default_value = "1e-10", value_parser = parse_tolerance)]
    tolerance: f64,
    #[arg(long, default_value_t = 100)]
    max_cycles: usize,
    /// Also solve the same nodal electrode mask without subcell boundaries.
    #[arg(long)]
    compare_staircase: bool,
}

pub fn run(args: SdfOptions) -> GpuResult<()> {
    let prepared = Instant::now();
    let sphere = matches!(args.geometry, TestGeometry::Sphere);
    let cylindrical = matches!(args.geometry, TestGeometry::Cylindrical);
    let inner = 0.31;
    let outer = 0.83;
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
                center: [0.0; 3],
                radius,
                axis: 2,
            }
        }
    };
    let h = if cylindrical {
        (outer - inner) / (args.n - 2) as f64
    } else {
        2.0 / (args.n - 1) as f64
    };
    let origin = if cylindrical {
        [inner - 0.5 * h, 0.0, 0.0]
    } else {
        [-1.0, -1.0, if sphere { -1.0 } else { 0.0 }]
    };
    let grid = Grid {
        dims: [args.n, args.n, if sphere { args.n } else { 1 }],
        spacing: [h; 3],
        geometry: if cylindrical {
            Geometry::Cylindrical {
                radial_origin: origin[0],
            }
        } else {
            Geometry::Planar
        },
    };
    let cuts = CutCells::new(
        grid,
        origin,
        &[
            Electrode::new(1.0, shape(inner)),
            Electrode::new(
                0.0,
                Sdf::Box {
                    center: [0.0; 3],
                    half_extent: [4.0; 3],
                }
                .difference(shape(outer)),
            ),
        ],
    )?;
    println!(
        "Geometry: {:?}, grid {:?}, {} nodes, {} cut nodes",
        args.geometry,
        grid.dims,
        cuts.values.len(),
        cuts.cut_nodes()
    );
    println!(
        "Geometry preparation: {:.3} ms, snap distance {:.9e}",
        prepared.elapsed().as_secs_f64() * 1000.0,
        cuts.snap_distance
    );
    let gpu = if args.device == Device::Gpu {
        let start = Instant::now();
        let gpu =
            pollster::block_on(Gpu::new(args.backend.backends()))?.ok_or("no GPU adapter found")?;
        crate::require_hardware_gpu(&gpu.info)?;
        crate::report_gpu_residual(gpu.has_f64());
        println!(
            "Adapter: {} ({:?}, {:?}), native f64: {}, initialization: {:.3} ms",
            gpu.info.name,
            gpu.info.backend,
            gpu.info.device_type,
            gpu.has_f64(),
            start.elapsed().as_secs_f64() * 1000.0
        );
        Some(gpu)
    } else {
        None
    };
    let options = Options {
        relative_tolerance: args.tolerance,
        max_cycles: args.max_cycles,
        ..Default::default()
    };
    let mut cut_error = 0.0;
    let mut cut_time = 0.0;
    for staircase in [false, true] {
        if staircase && !args.compare_staircase {
            break;
        }
        let mut values = cuts.values.clone();
        let (converged, cycles, residual, setup, compute) = if let Some(gpu) = &gpu {
            let report = if staircase {
                gpu.solve(&mut values, &cuts.fixed, grid, options)?
            } else {
                gpu.solve_sdf(&mut values, &cuts, options)?
            };
            println!(
                "{} GPU upload: {:.3} ms, readback: {:.3} ms",
                if staircase { "Staircase" } else { "SDF" },
                report.timings.upload.as_secs_f64() * 1000.0,
                report.timings.readback.as_secs_f64() * 1000.0
            );
            (
                report.converged,
                report.iterations,
                report.residual_history.last().unwrap().relative,
                report.timings.setup,
                report.timings.solve,
            )
        } else {
            let report = if staircase {
                mg::solve(&mut values, &cuts.fixed, grid, options)?
            } else {
                cuts.solve(&mut values, options)?
            };
            (
                report.converged,
                report.iterations,
                report.residual_history.last().unwrap().relative,
                report.timings.setup,
                report.timings.solve,
            )
        };
        let mut error = 0.0_f64;
        let mut field_error = 0.0_f64;
        for (i, &v) in values.iter().enumerate() {
            let c = [i % args.n, i / args.n % args.n, i / (args.n * args.n)];
            let p: [f64; 3] = std::array::from_fn(|a| origin[a] + c[a] as f64 * h);
            let r = if cylindrical {
                p[0]
            } else {
                p.iter()
                    .take(if sphere { 3 } else { 2 })
                    .map(|x| x * x)
                    .sum::<f64>()
                    .sqrt()
            };
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
            error = error.max((v - exact).abs());
            if !staircase
                && (r - inner).min(outer - r) < 2.0 * h
                && let Some(field) = cuts.electric_field(&values, i)
            {
                for a in 0..if sphere { 3 } else { 2 } {
                    let exact = if cylindrical {
                        if a == 0 { -derivative } else { 0.0 }
                    } else {
                        -derivative * p[a] / r
                    };
                    field_error = field_error.max((field[a] - exact).abs());
                }
            }
        }
        let name = if staircase { "Staircase" } else { "SDF" };
        println!(
            "{name}: setup {:.3} ms, compute {:.3} ms, cycles {cycles}, relative residual {residual:.9e}, max potential error {error:.9e}",
            setup.as_secs_f64() * 1000.0,
            compute.as_secs_f64() * 1000.0
        );
        if staircase {
            println!(
                "Staircase/SDF potential error: {:.3}x, SDF/staircase compute: {:.3}x",
                error / cut_error,
                cut_time / compute.as_secs_f64()
            );
        } else {
            println!("Near-surface max field component error: {field_error:.9e}");
            cut_error = error;
            cut_time = compute.as_secs_f64();
        }
        if !converged {
            return Err(
                format!("{name} did not converge within {} cycles", args.max_cycles).into(),
            );
        }
    }
    Ok(())
}
