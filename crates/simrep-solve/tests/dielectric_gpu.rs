use simrep_solve::{
    dielectric::Permittivity,
    gpu::{Backends, Gpu, GpuResult},
    mg::{Geometry, Grid, Options},
};

fn main() -> GpuResult<()> {
    for backend in [Backends::DX12, Backends::VULKAN] {
        let Some(gpu) = pollster::block_on(Gpu::new(backend))? else {
            println!("SKIP dielectric {backend:?}: no adapter");
            continue;
        };
        println!(
            "dielectric GPU: {} ({:?}, {:?})",
            gpu.info.name, gpu.info.backend, gpu.info.device_type
        );
        for (dims, geometry) in [
            ([17, 15, 13], Geometry::Planar),
            ([19, 17, 1], Geometry::Planar),
            ([19, 17, 1], Geometry::Cylindrical { radial_origin: 0.0 }),
            ([19, 17, 1], Geometry::Cylindrical { radial_origin: 2.0 }),
        ] {
            let count = dims.iter().product();
            let grid = Grid {
                dims,
                spacing: [0.7, 0.9, 1.1],
                geometry,
            };
            let fixed: Vec<_> = (0..count)
                .map(|i| i % dims[0] == 0 || i % dims[0] == dims[0] - 1 || i == count / 2)
                .collect();
            let initial: Vec<_> = (0..count)
                .map(|i| {
                    if fixed[i] {
                        1000.0 + (i % 13) as f64
                    } else {
                        1000.0
                    }
                })
                .collect();
            for cell_centered in [false, true] {
                let material = if cell_centered {
                    let d = dims.map(|n| (n - 1).max(1));
                    let eps: Vec<_> = (0..d.iter().product())
                        .map(|i| if i % d[0] < d[0] / 2 { 1.0 } else { 10.0 })
                        .collect();
                    Permittivity::from_cells(grid, &fixed, &eps)?
                } else {
                    let eps: Vec<_> = (0..count)
                        .map(|i| if i % dims[0] < dims[0] / 2 { 1.0 } else { 10.0 })
                        .collect();
                    Permittivity::from_nodes(grid, &fixed, &eps)?
                };
                let options = Options {
                    relative_tolerance: 1e-12,
                    max_cycles: 200,
                    ..Default::default()
                };
                let mut expected = initial.clone();
                assert!(material.solve(&mut expected, options)?.converged);
                for native in [false, true] {
                    if native && !gpu.has_f64() {
                        continue;
                    }
                    let mut actual = initial.clone();
                    let report =
                        material.solve_gpu_with_precision(&gpu, &mut actual, options, native)?;
                    assert!(
                        report.converged,
                        "{geometry:?}: {:?}",
                        report.residual_history
                    );
                    let error = actual
                        .iter()
                        .zip(&expected)
                        .map(|(a, b)| (a - b).abs())
                        .fold(0.0_f64, f64::max);
                    assert!(
                        error < 1e-8,
                        "{geometry:?} cells={cell_centered} native={native} error={error:e}"
                    );
                    for (i, &e) in fixed.iter().enumerate() {
                        if e {
                            assert_eq!(actual[i], initial[i]);
                        }
                    }
                    println!(
                        "PASS dielectric {geometry:?} dims={dims:?} cells={cell_centered} native={native}, cycles={}, CPU difference={error:.9e}",
                        report.iterations
                    );
                }
            }
        }
    }
    Ok(())
}
