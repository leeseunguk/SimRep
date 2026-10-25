use simrep_solve::{
    dielectric::Permittivity,
    gpu::{Backends, Gpu, GpuResult},
    mg::{self, Geometry, Grid, Options},
};

fn main() -> GpuResult<()> {
    for backend in [Backends::DX12, Backends::VULKAN] {
        let Some(gpu) = pollster::block_on(Gpu::new(backend))? else {
            println!("SKIP physical Poisson {backend:?}: no adapter");
            continue;
        };
        for geometry in [
            Geometry::Planar,
            Geometry::Cylindrical { radial_origin: 0.0 },
            Geometry::Cylindrical { radial_origin: 1.7 },
        ] {
            let grid = Grid {
                dims: [17, 15, 1],
                spacing: [0.3, 0.5, 1.0],
                geometry,
            };
            let mut fixed = vec![false; 255];
            let mut initial = vec![3.0; 255];
            let source: Vec<_> = (0..255)
                .map(|i| 0.7 + ((i * 7 % 19) as f64) * 0.13)
                .collect();
            for i in 0..255 {
                if i % 17 == 16 || i / 17 == 0 || (i % 17 == 7 && i / 17 == 8) {
                    fixed[i] = true;
                    initial[i] = 3.0 + 0.01 * (i % 17) as f64;
                }
            }
            let options = Options {
                relative_tolerance: 1e-12,
                ..Default::default()
            };
            let mut expected = initial.clone();
            assert!(mg::solve_poisson(&mut expected, &fixed, &source, grid, options)?.converged);
            for native in [false, true] {
                if native && !gpu.has_f64() {
                    continue;
                }
                let mut actual = initial.clone();
                let report = gpu.solve_poisson_with_precision(
                    &mut actual,
                    &fixed,
                    &source,
                    grid,
                    options,
                    native,
                )?;
                assert!(
                    report.converged,
                    "{geometry:?} native={native}: {:?}",
                    report.residual_history.last()
                );
                let error = actual
                    .iter()
                    .zip(&expected)
                    .map(|(a, b)| (a - b).abs())
                    .fold(0.0, f64::max);
                assert!(error < 1e-8, "physical RHS error={error:e}");
                for (i, &m) in fixed.iter().enumerate() {
                    if m {
                        assert_eq!(actual[i], initial[i]);
                    }
                }
                println!(
                    "PASS physical Poisson {:?} {geometry:?} native={native}: error={error:e}, cycles={}",
                    gpu.info.backend, report.iterations
                );
                let relative: Vec<_> = (0..255)
                    .map(|i| if i % 17 < 8 { 1.0 } else { 7.0 })
                    .collect();
                let material = Permittivity::from_nodes(grid, &fixed, &relative)?;
                let mut expected_material = initial.clone();
                assert!(
                    material
                        .solve_poisson(&mut expected_material, &source, options)?
                        .converged
                );
                let mut actual_material = initial.clone();
                let mut prepared = simrep_solve::gpu::Poisson::new_prepared(
                    &gpu,
                    &actual_material,
                    material.prepared_stencil(),
                    options,
                    native,
                )?;
                let report = prepared.solve(&gpu, &mut actual_material, &source)?;
                assert!(
                    report.converged,
                    "{geometry:?} native={native}: {:?}",
                    report.residual_history
                );
                let error = actual_material
                    .iter()
                    .zip(&expected_material)
                    .map(|(a, b)| (a - b).abs())
                    .fold(0.0, f64::max);
                assert!(error < 1e-8, "prepared physical RHS error={error:e}");
                println!(
                    "PASS prepared Poisson {:?} {geometry:?} native={native}: error={error:e}",
                    gpu.info.backend
                );
            }
        }
    }
    Ok(())
}
