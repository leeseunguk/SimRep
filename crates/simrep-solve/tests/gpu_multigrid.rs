use simrep_geom::{Electrode, Sdf};
use simrep_solve::{
    cutcell::CutCells,
    gpu::{Backends, Gpu, GpuResult},
    mg::{self, Geometry, Grid, Options},
};

fn main() -> GpuResult<()> {
    for backend in [Backends::DX12, Backends::VULKAN] {
        let Some(gpu) = pollster::block_on(Gpu::new(backend))? else {
            println!("SKIP gpu_multigrid {backend:?}: no adapter");
            continue;
        };
        check(&gpu)?;
        check_sdf(&gpu)?;
    }
    Ok(())
}

fn check_sdf(gpu: &Gpu) -> GpuResult<()> {
    for kind in 0..3 {
        let cylindrical = kind == 2;
        let sphere = kind == 0;
        let dims = [17, 15, if sphere { 13 } else { 1 }];
        let spacing = [0.125, 2.0 / 14.0, 2.0 / 12.0];
        let origin = if cylindrical {
            [0.0; 3]
        } else {
            [-1.0, -1.0, if sphere { -1.0 } else { 0.0 }]
        };
        let shape = |radius| {
            if cylindrical {
                Sdf::Plane {
                    normal: [1.0, 0.0, 0.0],
                    offset: radius,
                }
            } else if sphere {
                Sdf::Sphere {
                    center: [0.0; 3],
                    radius,
                }
            } else {
                Sdf::Cylinder {
                    center: [0.0; 3],
                    radius,
                    axis: 2,
                }
            }
        };
        let cuts = CutCells::new(
            Grid {
                dims,
                spacing,
                geometry: if cylindrical {
                    Geometry::Cylindrical { radial_origin: 0.0 }
                } else {
                    Geometry::Planar
                },
            },
            origin,
            &[
                Electrode::new(1003.0, shape(0.31)),
                Electrode::new(
                    1000.0,
                    Sdf::Box {
                        center: [0.0; 3],
                        half_extent: [4.0; 3],
                    }
                    .difference(shape(0.83)),
                ),
            ],
        )?;
        let options = Options {
            relative_tolerance: 1e-12,
            ..Default::default()
        };
        let mut expected = cuts.values.clone();
        assert!(cuts.solve(&mut expected, options)?.converged);
        for native in [false, true] {
            if native && !gpu.has_f64() {
                continue;
            }
            let mut actual = cuts.values.clone();
            let report = gpu.solve_sdf_with_precision(&mut actual, &cuts, options, native)?;
            assert!(
                report.converged,
                "SDF kind={kind} native={native}: {:?}",
                report.residual_history
            );
            let error = actual
                .iter()
                .zip(&expected)
                .map(|(a, b)| (a - b).abs())
                .fold(0.0_f64, f64::max);
            assert!(
                error < 1e-9,
                "SDF kind={kind} native={native} error={error:e}"
            );
            for (i, &fixed) in cuts.fixed.iter().enumerate() {
                if fixed {
                    assert_eq!(actual[i], cuts.values[i]);
                }
            }
            println!(
                "PASS SDF kind={kind} native={native}: {} cycles, relative={:.3e}, CPU difference={error:.3e}",
                report.iterations,
                report.residual_history.last().unwrap().relative
            );
        }
    }
    Ok(())
}

fn check(gpu: &Gpu) -> GpuResult<()> {
    println!(
        "gpu_multigrid: {} ({:?}, {:?}), SHADER_F64={}",
        gpu.info.name,
        gpu.info.backend,
        gpu.info.device_type,
        gpu.has_f64()
    );
    let cases = [
        ([12, 11, 9], Geometry::Planar, [0.7, 1.0, 1.3]),
        ([14, 13, 1], Geometry::Planar, [0.6, 1.1, 1.0]),
        (
            [13, 12, 1],
            Geometry::Cylindrical { radial_origin: 0.0 },
            [0.7, 0.9, 1.0],
        ),
        (
            [12, 11, 1],
            Geometry::Cylindrical { radial_origin: 2.0 },
            [0.8, 1.0, 1.0],
        ),
        ([3, 35, 2], Geometry::Planar, [0.8, 1.0, 1.2]),
    ];
    for (dims, geometry, spacing) in cases {
        let count = dims.iter().product();
        let mut initial = vec![3.0; count];
        let mut fixed = vec![false; count];
        // Mirror faces, a one-node electrode, and a thin internal wire which
        // does not coincide with selected coarse nodes.
        for i in 0..count {
            let x = i % dims[0];
            let y = i / dims[0] % dims[1];
            let z = i / (dims[0] * dims[1]);
            if x == 0
                || (x == dims[0] - 1 && y > dims[1] / 3)
                || (x == 3 && y == dims[1] / 2)
                || (x == dims[0] / 2 && y == 2 && z == 0)
            {
                fixed[i] = true;
                initial[i] = 3.0 + (x + 2 * y + z) as f64 * 0.17;
            }
        }
        let grid = Grid {
            dims,
            spacing,
            geometry,
        };
        let options = Options {
            relative_tolerance: 1e-12,
            max_cycles: 200,
            ..Default::default()
        };
        let mut reference = initial.clone();
        let cpu = mg::solve(&mut reference, &fixed, grid, options)?;
        assert!(cpu.converged, "CPU reference failed {dims:?}");
        for native in [false, true] {
            if native && !gpu.has_f64() {
                continue;
            }
            let mut actual = initial.clone();
            let report = gpu.solve_with_precision(&mut actual, &fixed, grid, options, native)?;
            let residual = report.residual_history.last().unwrap();
            assert!(report.converged, "{dims:?} native={native}: {residual:?}");
            assert!(residual.relative <= options.relative_tolerance);
            let scale = reference.iter().copied().map(f64::abs).fold(1.0, f64::max);
            let error = actual
                .iter()
                .zip(&reference)
                .map(|(a, b)| (a - b).abs())
                .fold(0.0, f64::max);
            assert!(
                error < scale * 1e-9,
                "{dims:?} native={native}: max error {error:e}"
            );
            for (i, &m) in fixed.iter().enumerate() {
                if m {
                    assert_eq!(actual[i].to_bits(), initial[i].to_bits());
                }
            }
            println!(
                "PASS {dims:?} {geometry:?} native={native}: {} cycles, rel={:.3e}, CPU difference={error:.3e}",
                report.iterations, residual.relative
            );
            // Budget exhaustion is a report, not a false convergence claim.
            let mut values = initial.clone();
            let report = gpu.solve_with_precision(
                &mut values,
                &fixed,
                grid,
                Options {
                    max_cycles: 0,
                    ..options
                },
                native,
            )?;
            assert!(!report.converged);
            assert_eq!(report.iterations, 0);
            assert_eq!(values, initial);
        }
    }
    Ok(())
}
