//! Compute-only profile of exported SIMION problems; see magnetostatics.md.
//! Usage: <export-directory> <case> [cpu|vulkan|fallback|dx12] [tolerance].
use std::{path::Path, time::Instant};

use simrep_core::{PotentialArray, Symmetry};
use simrep_io::read_pa;
use simrep_solve::{
    gpu::{Backends, Gpu, GpuResult, Poisson},
    magnetostatics::{self, MagneticProblem},
    mg::{Geometry, Grid, Options},
};

fn ordered<T: Copy>(values: &[T], a: &PotentialArray) -> Vec<T> {
    if a.symmetry == Symmetry::Cylindrical {
        (0..values.len())
            .map(|i| values[i / a.ny + a.nx * (i % a.ny)])
            .collect()
    } else {
        values.to_vec()
    }
}

fn median(values: &mut [f64]) -> f64 {
    values.sort_by(f64::total_cmp);
    values[values.len() / 2]
}

fn maximum(a: &[f64], b: &[f64]) -> f64 {
    a.iter()
        .zip(b)
        .map(|(a, b)| (a - b).abs())
        .fold(0.0, f64::max)
}

fn main() -> GpuResult<()> {
    let args: Vec<_> = std::env::args().collect();
    let root = Path::new(args.get(1).ok_or("expected export directory")?);
    let name = args.get(2).ok_or("expected case")?;
    let device = args.get(3).map(String::as_str).unwrap_or("cpu");
    let tolerance = args.get(4).map_or(Ok(1e-10), |v| v.parse::<f64>())?;
    let gpu = match device {
        "cpu" => None,
        "vulkan" | "fallback" => {
            Some(pollster::block_on(Gpu::new(Backends::VULKAN))?.ok_or("no Vulkan adapter")?)
        }
        "dx12" => Some(pollster::block_on(Gpu::new(Backends::DX12))?.ok_or("no DX12 adapter")?),
        _ => return Err("expected cpu, vulkan, fallback or dx12".into()),
    };
    if let Some(gpu) = &gpu {
        println!("ADAPTER,{:?},{}", gpu.info.backend, gpu.info.name);
        if gpu.info.device_type == simrep_solve::gpu::DeviceType::Cpu {
            return Err("hardware profile requires a hardware adapter".into());
        }
    }
    let jobs = std::fs::read_to_string(root.join(format!("{name}.jobs.csv")))?;
    for line in jobs.lines() {
        let job: Vec<_> = line.split(',').collect();
        let suffix = job[0];
        let filename = |suffix: &str, extension: &str| {
            root.join(format!(
                "{name}{}.{extension}",
                if suffix == "base" {
                    String::new()
                } else {
                    format!("-{suffix}")
                }
            ))
        };
        let input = read_pa(filename(suffix, "input.pa"))?;
        let a = &input.array;
        let cylindrical = a.symmetry == Symmetry::Cylindrical;
        let grid = Grid {
            dims: if cylindrical {
                [a.ny, a.nx, 1]
            } else {
                [a.nx, a.ny, a.nz]
            },
            spacing: if cylindrical {
                [a.dy_mm, a.dx_mm, a.dz_mm]
            } else {
                [a.dx_mm, a.dy_mm, a.dz_mm]
            },
            geometry: if cylindrical {
                Geometry::Cylindrical { radial_origin: 0.0 }
            } else {
                Geometry::Planar
            },
        };
        let fixed = ordered(&a.electrodes, a);
        let initial: Vec<_> = ordered(&a.potentials, a)
            .into_iter()
            .zip(&fixed)
            .map(|(v, &fixed)| if fixed { v } else { 0.0 })
            .collect();
        let cells = grid
            .dims
            .map(|n| n.saturating_sub(1).max(1))
            .iter()
            .product();
        let mu = if job[3].is_empty() {
            vec![1.0; cells]
        } else {
            magnetostatics::load_cells(&filename(job[3], "input.pa"), grid, cylindrical)?
        };
        let source = if job[2].is_empty() {
            vec![0.0; cells]
        } else {
            magnetostatics::load_cells(&filename(job[2], "input.pa"), grid, cylindrical)?
        };
        let problem = match job[1] {
            "scalar" => MagneticProblem::scalar(
                grid,
                &fixed,
                &mu,
                &vec![[0.0; 3]; cells],
                Some(&source),
                input.ng as f64,
            )?,
            "cartesian_a" => MagneticProblem::cartesian_current_component(
                grid,
                &fixed,
                mu[0],
                &source,
                input.ng as f64,
            )?,
            _ => MagneticProblem::currents(grid, &fixed, &mu, &source, input.ng as f64)?,
        };
        let options = Options {
            relative_tolerance: tolerance,
            max_cycles: 2000,
            ..Default::default()
        };
        let mut reference = initial.clone();
        let r = problem.solve(&mut reference, options)?;
        if !r.converged {
            return Err(format!("CPU reference not converged: {:?}", r.residual_history).into());
        }
        if name == "uniform_2dp" {
            for (i, v) in reference.iter_mut().enumerate() {
                *v = if suffix == "base" {
                    100.0 * (a.nx - 1 - i % a.nx) as f64 / (a.nx - 1) as f64
                } else {
                    100.0 * (i / a.nx % a.ny) as f64 / (a.ny - 1) as f64
                };
            }
        }
        let default = read_pa(filename(suffix, "default.pa"))?.array;
        let mut setup = Vec::new();
        let mut solve = Vec::new();
        for repetition in 0..3 {
            let mut actual = initial.clone();
            let (iterations, residual, setup_s, solve_s) = if let Some(gpu) = &gpu {
                let started = Instant::now();
                let mut solver = Poisson::new_prepared(
                    gpu,
                    &actual,
                    problem.prepared_stencil(),
                    options,
                    device != "fallback" && gpu.has_f64(),
                )?;
                let setup_s = started.elapsed().as_secs_f64();
                let r = solver.solve(gpu, &mut actual, problem.source())?;
                if !r.converged {
                    return Err(format!("GPU not converged: {:?}", r.residual_history).into());
                }
                (
                    r.iterations,
                    *r.residual_history.last().unwrap(),
                    setup_s,
                    r.timings.solve.as_secs_f64(),
                )
            } else {
                let r = problem.solve(&mut actual, options)?;
                if !r.converged {
                    return Err("CPU not converged".into());
                }
                println!(
                    "PHASES,workers={},smoothing={:.9},restriction={:.9},prolongation={:.9},residual={:.9}",
                    r.worker_threads,
                    r.timings.smoothing.as_secs_f64(),
                    r.timings.restriction.as_secs_f64(),
                    r.timings.prolongation.as_secs_f64(),
                    r.timings.residual.as_secs_f64()
                );
                (
                    r.iterations,
                    *r.residual_history.last().unwrap(),
                    r.timings.setup.as_secs_f64(),
                    r.timings.solve.as_secs_f64(),
                )
            };
            setup.push(setup_s);
            solve.push(solve_s);
            println!(
                "PROFILE,{name},{suffix},{device},rep={repetition},dims={:?},fixed={},cycles={iterations},setup_s={setup_s:.9},compute_s={solve_s:.9},absolute={:.9e},relative={:.9e},error={:.9e},default_error={:.9e},auto_native_gpu={},auto_fallback_gpu={}",
                grid.dims,
                fixed.iter().filter(|&&f| f).count(),
                residual.absolute,
                residual.relative,
                maximum(&actual, &reference),
                maximum(&ordered(&default.potentials, a), &reference),
                magnetostatics::prefer_gpu(grid, true),
                magnetostatics::prefer_gpu(grid, false)
            );
        }
        println!(
            "MEDIAN,{name},{suffix},{device},setup_s={:.9},compute_s={:.9}",
            median(&mut setup),
            median(&mut solve)
        );
    }
    Ok(())
}
