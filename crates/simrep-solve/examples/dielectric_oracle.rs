//! Read-only validation against the locally installed SIMION dielectric inputs.
//! Usage: dielectric_oracle <dielectric directory> [cpu|vulkan|dx12] [relative_tolerance]
use std::path::Path;

use simrep_core::Symmetry;
use simrep_solve::{
    dielectric::Permittivity,
    gpu::{Backends, Gpu, GpuResult},
    mg::{Geometry, Grid, Options},
};

fn transpose<T: Copy>(values: &[T], nx: usize, ny: usize) -> Vec<T> {
    (0..values.len())
        .map(|i| values[i / ny + nx * (i % ny)])
        .collect()
}

fn median(values: &[f64]) -> f64 {
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    sorted[sorted.len() / 2]
}

fn main() -> GpuResult<()> {
    let args: Vec<_> = std::env::args().collect();
    let root = Path::new(args.get(1).ok_or("expected dielectric directory")?);
    let tolerance = args
        .get(3)
        .map(|v| v.parse::<f64>())
        .transpose()?
        .unwrap_or(1e-12);
    let gpu = match args.get(2).map(String::as_str).unwrap_or("cpu") {
        "cpu" => None,
        backend @ ("vulkan" | "dx12") => Some(
            pollster::block_on(Gpu::new(if backend == "dx12" {
                Backends::DX12
            } else {
                Backends::VULKAN
            }))?
            .ok_or("no GPU adapter found")?,
        ),
        _ => return Err("expected cpu, vulkan or dx12".into()),
    };
    if let Some(gpu) = &gpu {
        println!(
            "Adapter: {} ({:?}, {:?})",
            gpu.info.name, gpu.info.backend, gpu.info.device_type
        );
    }
    for (name, simion_field, simion_compute) in [
        ("plates_2dp", None, 0.32),
        ("sphere_2dc", Some(0.25654121405188), 0.15),
        ("sphere_3dp", Some(0.24873766211532), 0.57),
    ] {
        let pa = simrep_io::read_pa(root.join(format!("{name}.pa")))?.array;
        let material = simrep_io::read_pa(root.join(format!("{name}-dielectric.pa")))?.array;
        let cylindrical = pa.symmetry == Symmetry::Cylindrical;
        let mut dims = [pa.nx, pa.ny, pa.nz];
        let mut spacing = [pa.dx_mm, pa.dy_mm, pa.dz_mm];
        let mut fixed = pa.electrodes;
        let mut initial = pa.potentials;
        let mut epsilon = material.potentials;
        for (i, v) in initial.iter_mut().enumerate() {
            *v = if i % pa.nx == 0 { 100.0 } else { 0.0 };
        }
        // The example's Lua replaces the GEM's material label 5 with kinside=10.
        if simion_field.is_some() {
            for e in &mut epsilon {
                if *e != 1.0 {
                    *e = 10.0;
                }
            }
        }
        if cylindrical {
            initial = transpose(&initial, pa.nx, pa.ny);
            fixed = transpose(&fixed, pa.nx, pa.ny);
            epsilon = transpose(&epsilon, material.nx, material.ny);
            dims.swap(0, 1);
            spacing.swap(0, 1);
        }
        let grid = Grid {
            dims,
            spacing,
            geometry: if cylindrical {
                Geometry::Cylindrical { radial_origin: 0.0 }
            } else {
                Geometry::Planar
            },
        };
        let prepared = std::time::Instant::now();
        let material = Permittivity::from_cells(grid, &fixed, &epsilon)?;
        let preparation_s = prepared.elapsed().as_secs_f64();
        let options = Options {
            relative_tolerance: tolerance,
            max_cycles: 300,
            ..Default::default()
        };
        let mut actual = initial.clone();
        let mut setup = Vec::new();
        let mut compute = Vec::new();
        let mut upload = Vec::new();
        let mut readback = Vec::new();
        let mut cycles = 0;
        let mut residual = 0.0;
        for repetition in 0..3 {
            actual.copy_from_slice(&initial);
            let converged = if let Some(gpu) = &gpu {
                let r = material.solve_gpu(gpu, &mut actual, options)?;
                setup.push(r.timings.setup.as_secs_f64());
                compute.push(r.timings.solve.as_secs_f64());
                upload.push(r.timings.upload.as_secs_f64());
                readback.push(r.timings.readback.as_secs_f64());
                cycles = r.iterations;
                residual = r.residual_history.last().unwrap().relative;
                r.converged
            } else {
                let r = material.solve(&mut actual, options)?;
                setup.push(r.timings.setup.as_secs_f64());
                compute.push(r.timings.solve.as_secs_f64());
                upload.push(0.0);
                readback.push(0.0);
                cycles = r.iterations;
                residual = r.residual_history.last().unwrap().relative;
                r.converged
            };
            assert!(converged, "{name} did not converge: {residual:e}");
            println!(
                "{name}: repetition={}, setup_s={:.6}, solve_s={:.6}, upload_s={:.6}, readback_s={:.6}",
                repetition + 1,
                setup[repetition],
                compute[repetition],
                upload[repetition],
                readback[repetition]
            );
        }
        let setup_s = median(&setup);
        let compute_s = median(&compute);
        let upload_s = median(&upload);
        let readback_s = median(&readback);
        let mut reference = initial;
        let r = material.solve(
            &mut reference,
            Options {
                relative_tolerance: 1e-13,
                ..options
            },
        )?;
        assert!(r.converged, "{name} reference did not converge");
        let difference = actual
            .iter()
            .zip(&reference)
            .map(|(a, b)| (a - b).abs())
            .fold(0.0_f64, f64::max);
        assert!(
            difference < 1e-5,
            "{name}: converged-reference difference={difference:e}"
        );
        let strict_path = root.join(format!("{name}.simrep-strict.pa"));
        if strict_path.exists() {
            let strict = simrep_io::read_pa(strict_path)?.array;
            let strict = if cylindrical {
                transpose(&strict.potentials, strict.nx, strict.ny)
            } else {
                strict.potentials
            };
            assert_eq!(strict.len(), actual.len());
            let strict_difference = strict
                .iter()
                .zip(&reference)
                .map(|(a, b)| (a - b).abs())
                .fold(0.0_f64, f64::max);
            println!("{name}: SIMION_strict_max_vs_our_converged_V={strict_difference:.9e}");
        }
        let center = if cylindrical {
            dims[0] * (dims[1] / 2)
        } else {
            dims[0] / 2
        };
        let stride = if cylindrical { dims[0] } else { 1 };
        let h = if cylindrical { spacing[1] } else { spacing[0] };
        let field = -(actual[center + stride] - actual[center - stride]) / (2.0 * h);
        let reference_field =
            -(reference[center + stride] - reference[center - stride]) / (2.0 * h);
        println!(
            "{name}: dims={dims:?}, repetitions=3, tolerance={tolerance:e}, cycles={cycles}, residual={residual:.9e}, preparation_s={preparation_s:.6}, median_setup_s={setup_s:.6}, median_solve_s={compute_s:.6}, median_upload_s={upload_s:.6}, median_readback_s={readback_s:.6}, SIMION_example_refine_s={simion_compute:.2}, max_vs_converged_V={difference:.9e}, Ecenter={field:.12}, reference_Ecenter={reference_field:.12}, SIMION_example_Ecenter={simion_field:?}"
        );
        if simion_field.is_some() {
            println!(
                "  analytic infinite-domain Ecenter=0.25 V/mm, error={:.9e}; finite mirrored box and interface sampling remain in both solvers",
                (field - 0.25).abs()
            );
            assert!((field - 0.25).abs() < 0.02);
        } else {
            let exact_error = reference
                .iter()
                .enumerate()
                .map(|(i, &v)| {
                    let x = (i % dims[0]) as f64;
                    let exact = if x <= 50.0 {
                        100.0 - 1.5 * x
                    } else {
                        50.0 - 0.5 * x
                    };
                    (v - exact).abs()
                })
                .fold(0.0_f64, f64::max);
            assert!(exact_error < 1e-6);
            println!("  analytic slab reference max_error_V={exact_error:.9e}");
        }
    }
    Ok(())
}
