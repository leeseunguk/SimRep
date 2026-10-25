//! Opt-in hardware measurements using copies of the app's solved templates.

use super::*;

/// The app's cached, adjustable templates, with no IPC or renderer work.
#[test]
fn live_refly_benchmark() {
    let Ok(directory) = std::env::var("SIMREP_GPU_LIVE_BENCH") else {
        return;
    };
    let precision = if std::env::var_os("SIMREP_GPU_PREVIEW").is_some() {
        Precision::Preview
    } else {
        Precision::Precise
    };
    let gpu = pollster::block_on(Gpu::new_with_precision(Backends::VULKAN, precision))
        .unwrap()
        .expect("benchmark requires Vulkan");
    assert_ne!(gpu.info.device_type, DeviceType::Cpu);
    assert!(gpu.has_timestamps());
    eprintln!("LIVE adapter={} precision={precision:?}", gpu.info.name);
    for (name, relative) in [
        ("einzel", "einzel/einzel.pa0"),
        ("quadrupole", "phase7b/quad/quadrupole.pa0"),
    ] {
        let path = std::path::Path::new(&directory).join(relative);
        let file = simrep_io::read_pa(&path).unwrap();
        let reference = file.pa0.as_ref().unwrap().reference_voltage;
        let bases: Vec<_> = (1..=3)
            .map(|index| {
                let mut array = simrep_io::read_pa(path.with_extension(format!("pa{index}")))
                    .unwrap()
                    .array;
                array.potentials.iter_mut().for_each(|v| *v /= reference);
                GridField::new_compact(array).unwrap()
            })
            .collect();
        let field = InstanceField::new(
            GridField::new_compact(file.array).unwrap(),
            [0.0; 3],
            [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            1.0,
            None,
            None,
        )
        .unwrap();
        let mut resident = gpu.prepare(&field, &bases).unwrap();
        resident.cache_basis_powers().unwrap();
        let options = FlightOptions {
            relative_tolerance: 1e-8,
            position_tolerance_mm: 1e-8,
            velocity_tolerance_mm_per_us: 1e-8,
            initial_step_us: 0.001,
            max_step_us: 0.1,
            duration_us: 100.0,
            relativistic: true,
            ..Default::default()
        };
        for count in [10_000, 100_000] {
            let golden = std::f64::consts::PI * (3.0 - 5.0_f64.sqrt());
            let particles: Vec<_> = (0..count)
                .map(|i| {
                    let radius = 3.0 * ((i as f64 + 0.5) / count as f64).sqrt();
                    let (sin, cos) = (i as f64 * golden).sin_cos();
                    Particle::from_relativistic_energy(
                        100.0,
                        1.0,
                        [1.0, radius * cos, radius * sin],
                        100.0,
                        [1.0, 0.0, 0.0],
                        0.0,
                    )
                    .unwrap()
                })
                .collect();
            let indices: Vec<_> = (0..300).map(|i| i * count / 300).collect();
            let sampling = TrajectorySampling {
                ion_indices: &indices,
                every: 1,
                max_samples_per_ion: 400,
            };
            let mut measurements = Vec::new();
            let mut last_results = Vec::new();
            for run in 0..6 {
                resident
                    .set_voltages(&[1.0, 1.0 + run as f64 * 0.01, 0.0, 0.0])
                    .unwrap();
                let start = Instant::now();
                let report = resident
                    .fly_sampled(&particles, &options, &sampling)
                    .unwrap();
                let wall = start.elapsed();
                assert!(report.results.iter().all(|r| matches!(
                    r.splat_reason,
                    SplatReason::Electrode { .. } | SplatReason::OutsideGrid
                )));
                if run >= 3 {
                    let t = report.timings;
                    let values = [wall, t.setup, t.upload, t.combine, t.fly, t.readback]
                        .map(|t| t.as_secs_f64() * 1000.0);
                    eprintln!("LIVE {name} ions={count} run={} ms={values:?}", run - 3);
                    measurements.push(values);
                    last_results = report.results;
                }
            }
            let medians: [f64; 6] = std::array::from_fn(|index| {
                let mut values: Vec<_> = measurements.iter().map(|m| m[index]).collect();
                values.sort_by(f64::total_cmp);
                values[1]
            });
            eprintln!(
                "LIVE {name} ions={count} median wall/setup/upload/combine/fly/readback ms={medians:?}"
            );
            let capacity = resident.max_ions_per_dispatch();
            resident
                .set_max_ions_per_dispatch(MAX_IONS_PER_DISPATCH)
                .unwrap();
            let control = resident
                .fly_sampled(&particles, &options, &sampling)
                .unwrap();
            resident.set_max_ions_per_dispatch(capacity).unwrap();
            super::tests::assert_flight_bits(&last_results, &control.results);
            eprintln!(
                "LIVE {name} ions={count} partition_bits=identical capacity={capacity} buffer_bytes={}",
                resident.gpu_memory_bytes()
            );
        }
    }
}

#[test]
fn template_compute_benchmark() {
    let Ok(directory) = std::env::var("SIMREP_GPU_BENCH") else {
        return;
    };
    let precision = if std::env::var_os("SIMREP_GPU_PREVIEW").is_some() {
        Precision::Preview
    } else {
        Precision::Precise
    };
    let powers = std::env::var_os("SIMREP_GPU_POWERS").is_some();
    let node_cells = std::env::var_os("SIMREP_GPU_NODE_CELLS").is_some();
    let gpu = pollster::block_on(Gpu::new_with_precision(Backends::VULKAN, precision))
        .unwrap()
        .expect("benchmark requires a Vulkan adapter");
    assert_ne!(gpu.info.device_type, DeviceType::Cpu);
    eprintln!(
        "BENCH adapter={} f64={} powers={powers}",
        gpu.info.name, gpu.wide
    );
    for (name, relative, voltages) in [
        ("einzel", "einzel/einzel.pa0", [0.0, 80.0, 0.0]),
        ("quadrupole", "quadrupole.pa0", [2.5, -2.5, 0.0]),
    ] {
        let path = std::path::Path::new(&directory).join(relative);
        let array = simrep_io::read_fast_adjust(
            &path,
            &[(1, voltages[0]), (2, voltages[1]), (3, voltages[2])],
        )
        .unwrap();
        let start = Instant::now();
        let mut grid = if powers {
            GridField::new(array)
        } else {
            GridField::new_compact(array)
        }
        .unwrap();
        if node_cells {
            grid = grid.with_node_cell_collisions();
        }
        let field = InstanceField::new(
            grid,
            [0.0; 3],
            [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            1.0,
            None,
            None,
        )
        .unwrap();
        eprintln!(
            "BENCH {name} node_cells={}",
            field.grid.uses_node_cell_collisions()
        );
        let reconstruction = start.elapsed();
        let mut resident = gpu.prepare_internal(&field, &[], !powers, true).unwrap();
        eprintln!(
            "BENCH {name} reconstruction={reconstruction:?} prepare={:?} upload={:?} bytes={}",
            resident.timings.setup,
            resident.timings.upload,
            resident.basis.size()
        );
        let options = FlightOptions {
            duration_us: 100.0,
            max_step_us: 0.1,
            relative_tolerance: 1e-8,
            position_tolerance_mm: 1e-8,
            velocity_tolerance_mm_per_us: 1e-8,
            relativistic: true,
            ..Default::default()
        };
        for count in [1_000, 10_000, 100_000] {
            let particles: Vec<_> = (0..count)
                .map(|i| {
                    let f = i as f64 / (count - 1) as f64;
                    let (y, z) = if name == "einzel" {
                        (-7.0 + 14.0 * f, 0.0)
                    } else {
                        let r = 3.0 * f.sqrt();
                        let angle = i as f64 * 2.399963229728653;
                        (r * angle.cos(), r * angle.sin())
                    };
                    Particle::from_relativistic_energy(
                        100.0,
                        1.0,
                        [1.0, y, z],
                        100.0,
                        [1.0, 0.0, 0.0],
                        0.0,
                    )
                    .unwrap()
                })
                .collect();
            let mut ion_file = String::from("; TOB,MASS,CHARGE,X,Y,Z,AZ,EL,KE,CWF,COLOR\n");
            for particle in &particles {
                let [x, y, z] = particle.initial.position_mm;
                ion_file.push_str(&format!("0,100,1,{x:.17e},{y:.17e},{z:.17e},0,0,100,1,1\n"));
            }
            std::fs::write(
                std::path::Path::new(&directory).join(format!("{name}-{count}.ion")),
                ion_file,
            )
            .unwrap();
            // Match the interactive resident-device use case and exercise
            // driver/clock warm-up equally for the control and new layout.
            let warmup = resident.fly(&particles, &options).unwrap();
            let report = resident.fly(&particles, &options).unwrap();
            assert!(report.results.iter().all(|r| matches!(
                r.splat_reason,
                SplatReason::Electrode { .. } | SplatReason::OutsideGrid | SplatReason::TimeLimit
            )));
            eprintln!(
                "BENCH {name} ions={count} compute={:?} combine={:?} upload={:?} readback={:?} steps={}/{}",
                report.timings.fly,
                warmup.timings.combine,
                report.timings.upload,
                report.timings.readback,
                report
                    .results
                    .iter()
                    .map(|r| r.accepted_steps)
                    .sum::<usize>(),
                report
                    .results
                    .iter()
                    .map(|r| r.rejected_steps)
                    .sum::<usize>(),
            );
        }
        if name == "quadrupole" && !powers {
            let file = simrep_io::read_pa(&path).unwrap();
            let reference = file.pa0.as_ref().unwrap().reference_voltage;
            let arrays: Vec<_> = (1..=3)
                .map(|i| {
                    let mut a = simrep_io::read_pa(path.with_extension(format!("pa{i}")))
                        .unwrap()
                        .array;
                    a.potentials.iter_mut().for_each(|v| *v /= reference);
                    a
                })
                .collect();
            let start = Instant::now();
            let mut base = GridField::new_compact(file.array).unwrap();
            if node_cells {
                base = base.with_node_cell_collisions();
            }
            let bases: Vec<_> = arrays
                .into_iter()
                .map(|a| {
                    let grid = GridField::new_compact(a).unwrap();
                    if node_cells {
                        grid.with_node_cell_collisions()
                    } else {
                        grid
                    }
                })
                .collect();
            let field = InstanceField::new(
                base,
                [0.0; 3],
                [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
                1.0,
                None,
                None,
            )
            .unwrap();
            let mut resident = gpu.prepare(&field, &bases).unwrap();
            if std::env::var_os("SIMREP_GPU_CACHE_POWERS").is_some() {
                resident.cache_basis_powers().unwrap();
            }
            let prep = start.elapsed();
            resident.set_voltages(&[1.0, 2.5, -2.5, 0.0]).unwrap();
            let ion = Particle::from_relativistic_energy(
                100.0,
                1.0,
                [1.0, 0.1, 0.1],
                100.0,
                [1.0, 0.0, 0.0],
                0.0,
            )
            .unwrap();
            let report = resident.fly(&[ion], &options).unwrap();
            eprintln!(
                "BENCH quadrupole-four-bases total_prep={prep:?} setup={:?} upload={:?} first_combine={:?} basis_bytes={} combined_bytes={}",
                resident.timings.setup,
                resident.timings.upload,
                report.timings.combine,
                resident.basis.size(),
                resident.combined.size()
            );
        }
    }
}
