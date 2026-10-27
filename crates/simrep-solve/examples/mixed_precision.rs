//! Reproduce mixed-precision stopping behavior on software or hardware Vulkan.
use simrep_solve::{
    dielectric::Permittivity,
    gpu::{Backends, Gpu, GpuResult},
    mg::{Geometry, Grid, Options},
};

fn main() -> GpuResult<()> {
    let gpu = pollster::block_on(Gpu::new(Backends::VULKAN))?.ok_or("no Vulkan adapter")?;
    println!("Adapter: {} ({:?})", gpu.info.name, gpu.info.device_type);
    for (name, dims, spacing, contrast) in [
        ("cube64", [64, 64, 64], [1.0; 3], 1.0),
        ("tight", [33, 31, 1], [1.0; 3], 1.0),
        ("contrast", [65, 63, 1], [1.0; 3], 1e8),
        ("anisotropic", [65, 63, 1], [1e-5, 1.0, 1.0], 1.0),
    ] {
        let count = dims.iter().product();
        let grid = Grid {
            dims,
            spacing,
            geometry: Geometry::Planar,
        };
        let fixed: Vec<_> = (0..count)
            .map(|i| i % dims[0] == 0 || i % dims[0] == dims[0] - 1)
            .collect();
        let initial: Vec<_> = (0..count)
            .map(|i| {
                if i % dims[0] == dims[0] - 1 {
                    if name == "tight" {
                        1.0 + (i / dims[0] % dims[1]) as f64 * 0.17
                    } else {
                        1.0
                    }
                } else {
                    0.0
                }
            })
            .collect();
        let eps: Vec<_> = (0..count)
            .map(|i| {
                if i % dims[0] < dims[0] / 2 {
                    1.0
                } else {
                    contrast
                }
            })
            .collect();
        let material = Permittivity::from_nodes(grid, &fixed, &eps)?;
        for native in [false, true] {
            if native && !gpu.has_f64() {
                continue;
            }
            let mut values = initial.clone();
            let report = material.solve_gpu_with_precision(
                &gpu,
                &mut values,
                Options {
                    relative_tolerance: if name == "tight" { 1e-30 } else { 1e-12 },
                    max_cycles: 200,
                    ..Default::default()
                },
                native,
            )?;
            let last = report.residual_history.last().unwrap();
            println!(
                "{name} native={native} converged={} cycles={} outer={} relative={:.12e} solve_ms={:.3} reason={:?}",
                report.converged,
                report.iterations,
                report.outer_iterations,
                last.relative,
                report.timings.solve.as_secs_f64() * 1000.0,
                report.failure_reason,
            );
            println!(
                "history={:?}",
                report
                    .residual_history
                    .iter()
                    .map(|r| r.relative)
                    .collect::<Vec<_>>()
            );
            if let Some(directory) = std::env::args().nth(1) {
                std::fs::create_dir_all(&directory)?;
                std::fs::write(
                    format!("{directory}/{name}-{native}.bin"),
                    bytemuck::cast_slice(&values),
                )?;
            }
        }
    }
    Ok(())
}
