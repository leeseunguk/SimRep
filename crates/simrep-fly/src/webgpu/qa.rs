use super::*;
use simrep_core::PotentialArray;
use simrep_solve::mg::{Geometry, Grid, Options};
use simrep_solve::webgpu::Gpu;

fn u32_at(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}
fn f32_at(bytes: &mut [u8], offset: usize, value: f64) {
    bytes[offset..offset + 4].copy_from_slice(&(value as f32).to_le_bytes());
}
fn reason(reason: SplatReason) -> u32 {
    match reason {
        SplatReason::Electrode { .. } | SplatReason::SurfaceElectrode { .. } => 1,
        SplatReason::OutsideGrid => 2,
        SplatReason::StopPlane => 3,
        SplatReason::TimeLimit => 4,
        SplatReason::StepLimit => 5,
        SplatReason::StepUnderflow => 6,
        SplatReason::NonFinite => 7,
    }
}

#[test]
fn portable_einzel_matches_native_f32_and_cpu_splats() {
    pollster::block_on(async {
        let gpu = Gpu::with_backends(wgpu::Backends::VULKAN)
            .await
            .unwrap()
            .expect("WebGPU native reference requires a Vulkan adapter");
        let mut array = PotentialArray::new(61, 21, 1);
        for y in 0..21 {
            for x in 0..61 {
                let i = y * 61 + x;
                let plate =
                    (4..=16).contains(&x) || (20..=40).contains(&x) || (44..=56).contains(&x);
                array.electrodes[i] = x == 0 || x == 60 || (plate && !(7..=13).contains(&y));
                if array.electrodes[i] && (20..=40).contains(&x) {
                    array.potentials[i] = 12.0;
                }
            }
        }
        let grid = Grid {
            dims: [61, 21, 1],
            spacing: [1.0; 3],
            geometry: Geometry::Planar,
        };
        let options = Options {
            relative_tolerance: 1e-9,
            max_cycles: 200,
            ..Options::default()
        };
        let mut portable_potential = array.potentials.clone();
        let gpu_solve = gpu
            .solve(&mut portable_potential, &array.electrodes, grid, options)
            .await
            .unwrap();
        assert!(gpu_solve.converged, "residual {}", gpu_solve.residual);
        simrep_solve::mg::solve(&mut array.potentials, &array.electrodes, grid, options).unwrap();
        let solve_max_error = portable_potential
            .iter()
            .zip(&array.potentials)
            .map(|(a, b)| (a - b).abs())
            .fold(0.0, f64::max);
        assert!(
            solve_max_error < 1e-5,
            "solver discrepancy {solve_max_error}"
        );
        let field = InstanceField::new(
            GridField::new(array).unwrap(),
            [0.0; 3],
            [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            1.0,
            None,
            None,
        )
        .unwrap();
        let particles: Vec<_> = (0..15)
            .map(|i| {
                Particle::new(
                    40.0,
                    1.0,
                    [1.0, 8.0 + (i % 5) as f64, 0.0],
                    [2.0 + (i / 5) as f64, 0.0, 0.0],
                    0.0,
                )
                .unwrap()
            })
            .collect();
        let flight_options = FlightOptions {
            duration_us: 100.0,
            max_step_us: 0.1,
            ..FlightOptions::default()
        };
        let start = std::time::Instant::now();
        let cpu = fly_many(&field, &particles, &flight_options).unwrap();
        let cpu_ms = start.elapsed().as_secs_f64() * 1000.0;
        let native = gpu::Gpu::new_with_precision(gpu::Backends::VULKAN, gpu::Precision::Preview)
            .await
            .unwrap()
            .unwrap();
        let mut native_field = native.prepare(&field, &[]).unwrap();
        let native_report = native_field.fly(&particles, &flight_options).unwrap();
        let (cells, degrees, coefficients, nodal) = field.grid.gpu_data(false);
        assert!(!nodal);
        let coefficient_f32: Vec<_> = coefficients.iter().map(|v| *v as f32).collect();
        let mut mask_u32 = field.grid.collision_mask();
        let packed_degree = (degrees[0] | (degrees[1] << 4) | (degrees[2] << 8)) as u32;
        for cell in 0..cells.iter().product() {
            let origin = [
                cell % cells[0],
                cell / cells[0] % cells[1],
                cell / (cells[0] * cells[1]),
            ];
            let solid =
                (0..2).all(|y| (0..2).all(|x| mask_u32[(origin[1] + y) * 61 + origin[0] + x] != 0));
            mask_u32.push(packed_degree | (u32::from(solid) << 12));
        }
        let mut parameter_bytes = vec![0u8; 640];
        for (offset, value) in [
            (0, 61),
            (4, 21),
            (8, 1),
            (16, cells[0] as u32),
            (20, cells[1] as u32),
            (24, cells[2] as u32),
            (28, coefficients.len() as u32),
            (32, degrees[0] as u32),
            (36, degrees[1] as u32),
            (40, degrees[2] as u32),
            (44, 1),
            (224, particles.len() as u32),
            (228, flight_options.max_steps as u32),
            (240, 61 * 21),
            (248, u32::from(field.grid.uses_node_cell_collisions())),
        ] {
            u32_at(&mut parameter_bytes, offset, value);
        }
        for offset in [64, 68, 72, 76, 80, 100, 120, 256] {
            f32_at(&mut parameter_bytes, offset, 1.0);
        }
        for (offset, value) in [
            (192, 100.0),
            (196, flight_options.initial_step_us),
            (200, flight_options.min_step_us),
            (204, 0.1),
            (208, flight_options.relative_tolerance.max(2e-7)),
            (212, flight_options.position_tolerance_mm.max(2e-7)),
            (216, flight_options.velocity_tolerance_mm_per_us.max(2e-7)),
            (592, flight_options.step_limits.max_grid_cells),
            (596, flight_options.step_limits.max_cyclotron_radians),
            (
                600,
                1e30 * flight_options.step_limits.max_voltage_phase_radians,
            ),
        ] {
            f32_at(&mut parameter_bytes, offset, value);
        }
        let mut ion_bytes = vec![0u8; particles.len() * 144];
        for (i, particle) in particles.iter().enumerate() {
            let bytes = &mut ion_bytes[i * 144..(i + 1) * 144];
            u32_at(bytes, 16, u32::MAX);
            for a in 0..3 {
                let q = particle.initial.position_mm[a];
                u32_at(bytes, 4 * a, q.floor() as i32 as u32);
                f32_at(bytes, 32 + 4 * a, q - q.floor());
                f32_at(bytes, 48 + 4 * a, particle.initial.velocity_mm_per_us[a]);
            }
            f32_at(bytes, 64, particle.charge_e / particle.mass_amu);
        }
        let input = webgpu::FlightInput {
            parameter_bytes,
            coefficient_f32,
            mask_u32,
            ion_bytes,
        };
        let output = webgpu::fly(&gpu, &input).await.unwrap();
        let packed_reasons: Vec<_> = output
            .end_state_bytes
            .as_chunks::<64>()
            .0
            .iter()
            .map(|end| u32::from_le_bytes(end[24..28].try_into().unwrap()))
            .collect();
        let cpu_reasons: Vec<_> = cpu.iter().map(|r| reason(r.splat_reason)).collect();
        let native_reasons: Vec<_> = native_report
            .results
            .iter()
            .map(|r| reason(r.splat_reason))
            .collect();
        assert_eq!(packed_reasons, native_reasons);
        assert_eq!(packed_reasons, cpu_reasons);
        assert!(Gpu::require_precision("f64").is_err());
        let report = serde_json::json!({
            "adapter": gpu.info.name, "ions": particles.len(),
            "portableReasons": packed_reasons, "nativeF32Reasons": native_reasons, "cpuF64Reasons": cpu_reasons,
            "portableComputeMs": output.compute_ms,
            "nativeF32ComputeMs": native_report.timings.fly.as_secs_f64() * 1000.0,
            "cpuF64ComputeMs": cpu_ms, "solveComputeMs": gpu_solve.compute_ms,
            "solveMaxErrorVsCpuF64": solve_max_error,
            "nativeEndStates": output.end_state_bytes,
        });
        eprintln!("{report}");
        if let Some(directory) = std::env::var_os("SIMREP_WEBGPU_EVIDENCE_DIR") {
            let directory = std::path::PathBuf::from(directory);
            std::fs::create_dir_all(&directory).unwrap();
            std::fs::write(
                directory.join("flight-input.json"),
                serde_json::to_vec(&input).unwrap(),
            )
            .unwrap();
            std::fs::write(
                directory.join("native-reference.json"),
                serde_json::to_vec_pretty(&report).unwrap(),
            )
            .unwrap();
            std::fs::write(directory.join("flight.wgsl"), webgpu::shader_source()).unwrap();
        }
    });
}
