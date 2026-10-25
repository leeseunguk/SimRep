use super::*;
use crate::Field;
use simrep_core::PotentialArray;

pub(super) fn assert_flight_bits(a: &[FlightResult], b: &[FlightResult]) {
    assert_eq!(a.len(), b.len());
    for (a, b) in a.iter().zip(b) {
        assert_eq!(a.splat_reason, b.splat_reason);
        assert_eq!(a.accepted_steps, b.accepted_steps);
        assert_eq!(a.rejected_steps, b.rejected_steps);
        assert_eq!(a.tof_us.to_bits(), b.tof_us.to_bits());
        let states_a = std::iter::once(&a.final_state).chain(a.trajectory.iter().flatten());
        let states_b = std::iter::once(&b.final_state).chain(b.trajectory.iter().flatten());
        assert_eq!(
            a.trajectory.as_ref().map(Vec::len),
            b.trajectory.as_ref().map(Vec::len)
        );
        for (a, b) in states_a.zip(states_b) {
            assert_eq!(
                a.position_mm.map(f64::to_bits),
                b.position_mm.map(f64::to_bits)
            );
            assert_eq!(
                a.velocity_mm_per_us.map(f64::to_bits),
                b.velocity_mm_per_us.map(f64::to_bits)
            );
            assert_eq!(a.time_us.to_bits(), b.time_us.to_bits());
        }
    }
}

#[test]
fn dispatch_capacity_obeys_workgroup_binding_and_buffer_limits() {
    for wide in [false, true] {
        let mut limits = wgpu::Limits {
            max_compute_workgroups_per_dimension: 2,
            max_storage_buffer_binding_size: 1 << 28,
            max_buffer_size: 1 << 30,
            ..Default::default()
        };
        assert_eq!(ion_capacity(&limits, wide), 128);
        limits.max_storage_buffer_binding_size = 100 * if wide { 224 } else { 144 };
        assert_eq!(ion_capacity(&limits, wide), 100);
        limits.max_buffer_size = 50 * if wide { 224 } else { 144 };
        assert_eq!(ion_capacity(&limits, wide), 50);
        // An allocation rounded to the next power of two must also stay bounded.
        assert_eq!(
            101usize
                .next_power_of_two()
                .min(ion_capacity(&limits, wide)),
            50
        );
    }
}

#[test]
fn pending_flights_snapshot_voltages_samples_and_pipeline_and_recycle_buffers() {
    let mut array = PotentialArray::new(7, 5, 1);
    for (i, value) in array.potentials.iter_mut().enumerate() {
        *value = -0.1 * (i % 7) as f64;
    }
    let field = InstanceField::new(
        GridField::new(array).unwrap(),
        [1e9, 0.0, 0.0],
        [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        1.0,
        None,
        None,
    )
    .unwrap();
    let particles: Vec<_> = (0..7)
        .map(|i| {
            Particle::new(
                40.0,
                1.0,
                [1e9 + 0.25, 1.0 + i as f64 * 0.1, 0.0],
                [1.0, 0.0, 0.0],
                1000.0 + i as f64,
            )
            .unwrap()
        })
        .collect();
    let options = FlightOptions {
        duration_us: 1.0,
        max_step_us: 0.1,
        ..Default::default()
    };
    let sampling = TrajectorySampling {
        ion_indices: &[0, 2, 3, 6],
        every: 2,
        max_samples_per_ion: 64,
    };
    for precision in [Precision::Precise, Precision::Preview] {
        let Some(gpu) =
            pollster::block_on(Gpu::new_with_precision(Backends::VULKAN, precision)).unwrap()
        else {
            eprintln!("SKIP pending GPU flights: no Vulkan adapter");
            return;
        };
        let mut prepared = gpu.prepare(&field, &[]).unwrap();
        assert_eq!(
            prepared.max_ions_per_dispatch(),
            gpu.ion_capacity()
                .min(if gpu.wide || gpu.info.device_type == DeviceType::Cpu {
                    MAX_IONS_PER_DISPATCH
                } else {
                    PREFERRED_IONS_PER_DISPATCH
                })
        );
        let initial_bytes = prepared.gpu_memory_bytes();
        assert_eq!(
            initial_bytes,
            [
                &prepared.basis,
                &prepared.combined,
                &prepared.mask,
                &prepared.uniform,
                &prepared.instances,
                &prepared.physics_data
            ]
            .iter()
            .map(|b| b.size())
            .sum::<u64>()
        );
        assert!(prepared.set_max_ions_per_dispatch(0).is_err());
        prepared.set_max_ions_per_dispatch(usize::MAX).unwrap();
        assert_eq!(prepared.max_ions_per_dispatch(), gpu.ion_capacity());
        prepared.set_max_ions_per_dispatch(3).unwrap();
        let first = prepared
            .fly_sampled(&particles, &options, &sampling)
            .unwrap();
        let warmed_bytes = prepared.gpu_memory_bytes();
        assert!(warmed_bytes > initial_bytes);
        prepared.set_voltages(&[2.0]).unwrap();
        let second = prepared
            .fly_sampled(&particles, &options, &sampling)
            .unwrap();
        prepared.set_voltages(&[1.0]).unwrap();
        let pending_first = prepared
            .submit_sampled(&particles, &options, &sampling)
            .unwrap();
        prepared.set_voltages(&[2.0]).unwrap();
        let pending_second = prepared
            .submit_sampled(&particles, &options, &sampling)
            .unwrap();
        assert!(prepared.gpu_memory_bytes() > warmed_bytes);
        // New options, a new shader and an out-of-order collect cannot change
        // either already submitted flight.
        prepared
            .set_voltage_expressions(&[Some(VoltageExpression::constant(3.0).unwrap())])
            .unwrap();
        let boris = FlightOptions {
            integrator: Integrator::Boris,
            initial_step_us: 0.1,
            ..options.clone()
        };
        let pending_third = prepared.submit(&particles, &boris).unwrap();
        assert_flight_bits(&second.results, &pending_second.collect().unwrap().results);
        assert_flight_bits(&first.results, &pending_first.collect().unwrap().results);
        assert_flight_bits(
            &pending_third.collect().unwrap().results,
            &prepared.fly(&particles, &boris).unwrap().results,
        );
        let bytes = prepared.gpu_memory_bytes();
        drop(
            prepared
                .submit_sampled(&particles, &boris, &sampling)
                .unwrap(),
        );
        prepared.fly_sampled(&particles, &boris, &sampling).unwrap();
        assert_eq!(prepared.gpu_memory_bytes(), bytes);
        assert!(
            prepared
                .submit(&[], &options)
                .unwrap()
                .collect()
                .unwrap()
                .results
                .is_empty()
        );
        // The handle keeps resource snapshots, not a borrow of PreparedField.
        let pending = {
            let mut temporary = gpu.prepare(&field, &[]).unwrap();
            temporary.submit(&particles, &options).unwrap()
        };
        let plain = gpu
            .prepare(&field, &[])
            .unwrap()
            .fly(&particles, &options)
            .unwrap();
        assert_flight_bits(&plain.results, &pending.collect().unwrap().results);
    }
}

#[test]
fn all_corner_cells_mirrored_integer_faces_and_cylindrical_shells() {
    for precision in [Precision::Precise, Precision::Preview] {
        let Some(gpu) =
            pollster::block_on(Gpu::new_with_precision(Backends::VULKAN, precision)).unwrap()
        else {
            eprintln!("SKIP node-cell GPU geometry: no Vulkan adapter");
            return;
        };
        let mut array = PotentialArray::new(7, 7, 7);
        array.mirrors.x = true;
        array.mirrors.y = true;
        for z in 1..=4 {
            for y in 1..=4 {
                for x in 3..=4 {
                    array.electrodes[(z * 7 + y) * 7 + x] = true;
                }
            }
        }
        array.electrodes[(5 * 7 + 5) * 7 + 1] = true;
        let grid = GridField::new_compact(array)
            .unwrap()
            .with_node_cell_collisions();
        let field = InstanceField::new(
            grid,
            [0.0; 3],
            [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            1.0,
            None,
            None,
        )
        .unwrap();
        let particles = [
            Particle::new(1.0, 0.0, [0.25, 2.25, 2.25], [2.0, 0.0, 0.0], 0.0).unwrap(),
            Particle::new(1.0, 0.0, [-4.0, 2.0, 2.0], [-1.0, 0.0, 0.0], 0.0).unwrap(),
            Particle::new(1.0, 0.0, [0.25, -2.5, 2.5], [2.0, 0.0, 0.0], 0.0).unwrap(),
            Particle::new(1.0, 0.0, [0.25, 5.0, 5.0], [2.0, 0.0, 0.0], 0.0).unwrap(),
        ];
        let options = FlightOptions {
            duration_us: 5.0,
            max_step_us: 2.0,
            ..Default::default()
        };
        let cpu = crate::fly_many(&field, &particles, &options).unwrap();
        let actual = gpu
            .prepare(&field, &[])
            .unwrap()
            .fly(&particles, &options)
            .unwrap();
        let tolerance = if gpu.wide { 2e-8 } else { 2e-4 };
        for (a, b) in actual.results.iter().zip(&cpu) {
            assert_eq!(a.splat_reason, b.splat_reason, "{a:?} vs {b:?}");
            assert!((a.tof_us - b.tof_us).abs() < tolerance);
            for axis in 0..3 {
                assert!(
                    (a.final_state.position_mm[axis] - b.final_state.position_mm[axis]).abs()
                        < tolerance
                );
            }
        }
        assert!(matches!(
            actual.results[0].splat_reason,
            SplatReason::Electrode { .. }
        ));
        assert_eq!(actual.results[3].splat_reason, SplatReason::OutsideGrid);

        let mut array = PotentialArray::new(9, 7, 1);
        array.symmetry = Symmetry::Cylindrical;
        for y in 2..=3 {
            for x in 0..=7 {
                array.electrodes[y * 9 + x] = true;
            }
        }
        let grid = GridField::new(array)
            .unwrap()
            .with_node_cell_collisions()
            .with_transparent_faces([true, false, false, false, false, false]);
        let field = InstanceField::new(
            grid,
            [0.0; 3],
            [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            1.0,
            None,
            None,
        )
        .unwrap();
        let particles = [
            Particle::new(1.0, 0.0, [2.0, 0.25, 0.0], [0.0, 1.0, 0.0], 0.0).unwrap(),
            Particle::new(1.0, 0.0, [0.25, 0.25, 0.0], [0.0, 1.0, 0.0], 0.0).unwrap(),
        ];
        let options = FlightOptions {
            duration_us: 10.0,
            ..options
        };
        let cpu = crate::fly_many(&field, &particles, &options).unwrap();
        let actual = gpu
            .prepare(&field, &[])
            .unwrap()
            .fly(&particles, &options)
            .unwrap();
        for (a, b) in actual.results.iter().zip(&cpu) {
            assert_eq!(a.splat_reason, b.splat_reason);
            assert!((a.tof_us - b.tof_us).abs() < tolerance);
        }
    }
}

#[test]
fn compact_fields_reused_buffers_shared_device_and_selected_samples() {
    let mut array = PotentialArray::new(7, 7, 7);
    for z in 0..7 {
        for y in 0..7 {
            for x in 0..7 {
                let i = (z * 7 + y) * 7 + x;
                array.potentials[i] = -0.2 * x as f64 + 0.01 * (y as f64).powi(2) + 0.03 * z as f64;
                array.electrodes[i] = x == 6;
            }
        }
    }
    let full = GridField::new(array.clone()).unwrap();
    let compact = GridField::new_compact(array).unwrap();
    for point in [[1.25, 2.375, 3.125], [5.7, 0.2, 4.1], [6.1, 2.0, 3.0]] {
        assert_eq!(full.sample(point, 0.0), compact.sample(point, 0.0));
    }
    for precision in [Precision::Precise, Precision::Preview] {
        let Some(gpu) =
            pollster::block_on(Gpu::new_with_precision(Backends::VULKAN, precision)).unwrap()
        else {
            eprintln!("SKIP selected GPU trajectories: no Vulkan adapter");
            return;
        };
        let shared = pollster::block_on(Gpu::from_device(
            gpu.device().clone(),
            gpu.queue().clone(),
            gpu.info.clone(),
        ))
        .unwrap();
        let solver = simrep_solve::gpu::Gpu::from_device(
            shared.device().clone(),
            shared.queue().clone(),
            shared.info.clone(),
        );
        assert_eq!(solver.has_f64(), shared.has_f64());
        if precision == Precision::Precise {
            let preview = pollster::block_on(Gpu::from_device_with_precision(
                shared.device().clone(),
                shared.queue().clone(),
                shared.info.clone(),
                Precision::Preview,
            ))
            .unwrap();
            assert!(!preview.has_f64());
            assert_eq!(preview.device(), shared.device());
        }
        let field = InstanceField::new(
            compact.clone(),
            [1e9, 0.0, 0.0],
            [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            1.25,
            None,
            None,
        )
        .unwrap();
        let particles: Vec<_> = (0..3)
            .map(|i| {
                Particle::new(
                    40.0,
                    1.0,
                    [1e9 + 0.25, 1.0 + i as f64 * 0.125, 2.0],
                    [2.0, 0.01, 0.0],
                    1000.0 + i as f64,
                )
                .unwrap()
            })
            .collect();
        let options = FlightOptions {
            duration_us: 10.0,
            max_step_us: 0.1,
            relativistic: true,
            ..Default::default()
        };
        let mut resident = shared.prepare(&field, &[]).unwrap();
        let plain = resident.fly(&particles, &options).unwrap();
        let mut cached = shared
            .prepare(&field, std::slice::from_ref(&compact))
            .unwrap();
        cached.set_voltages(&[0.25, 0.75]).unwrap();
        let compact_result = cached.fly(&particles, &options).unwrap();
        cached.cache_basis_powers().unwrap();
        cached.cache_basis_powers().unwrap();
        let cached_result = cached.fly(&particles, &options).unwrap();
        for (a, b) in compact_result.results.iter().zip(&cached_result.results) {
            assert_eq!(a.splat_reason, b.splat_reason);
            assert!((a.tof_us - b.tof_us).abs() < if shared.wide { 1e-9 } else { 1e-4 });
        }
        cached.set_voltages(&[0.3, 1.7]).unwrap();
        let adjusted = cached.fly(&particles, &options).unwrap();
        assert!(adjusted.results[0].tof_us < cached_result.results[0].tof_us);
        let ions = resident
            .ion_buffers
            .lock()
            .unwrap()
            .idle
            .last()
            .unwrap()
            .ions
            .clone();
        let sampling = TrajectorySampling {
            ion_indices: &[0, 2],
            every: 3,
            max_samples_per_ion: 100,
        };
        let sampled = resident
            .fly_sampled(&particles, &options, &sampling)
            .unwrap();
        // A repeated call at the same capacities retains the actual handles.
        let reused = resident
            .ion_buffers
            .lock()
            .unwrap()
            .idle
            .last()
            .unwrap()
            .ions
            .clone();
        let again = resident
            .fly_sampled(&particles, &options, &sampling)
            .unwrap();
        assert_eq!(
            reused,
            resident
                .ion_buffers
                .lock()
                .unwrap()
                .idle
                .last()
                .unwrap()
                .ions
        );
        assert_ne!(ions, reused); // Sampling grew the initial staging allocation.
        assert!(sampled.results[1].trajectory.is_none());
        for &index in sampling.ion_indices {
            let a = &sampled.results[index];
            let b = &plain.results[index];
            assert_eq!(a.final_state, b.final_state);
            assert_eq!(a.splat_reason, b.splat_reason);
            assert_eq!(a.accepted_steps, b.accepted_steps);
            assert_eq!(a.trajectory, again.results[index].trajectory);
            let path = a.trajectory.as_ref().unwrap();
            let tolerance = if shared.wide { 2e-12 } else { 2e-5 };
            for (sample, expected) in [
                (&path[0], &particles[index].initial),
                (path.last().unwrap(), &a.final_state),
            ] {
                assert!((sample.time_us - expected.time_us).abs() < tolerance);
                for axis in 0..3 {
                    assert!(
                        (sample.position_mm[axis] - expected.position_mm[axis]).abs() < tolerance
                    );
                    assert!(
                        (sample.velocity_mm_per_us[axis] - expected.velocity_mm_per_us[axis]).abs()
                            < tolerance
                    );
                }
            }
            assert_eq!(path.len(), 1 + a.accepted_steps.div_ceil(3));
        }
        let overflow = resident.fly_sampled(
            &particles,
            &options,
            &TrajectorySampling {
                max_samples_per_ion: 2,
                ..sampling
            },
        );
        assert!(
            overflow
                .err()
                .unwrap()
                .to_string()
                .contains("capacity exceeded")
        );
        // An overflow leaves mapped buffers reusable.
        resident.fly(&particles, &options).unwrap();

        // Existing static-to-expression updates also work on compact bases.
        resident
            .set_voltage_expressions(&[Some(VoltageExpression::constant(1.0).unwrap())])
            .unwrap();
        let dynamic = resident.fly(&particles, &options).unwrap();
        for (a, b) in dynamic.results.iter().zip(&plain.results) {
            assert_eq!(a.splat_reason, b.splat_reason);
            assert!((a.tof_us - b.tof_us).abs() < if shared.wide { 1e-9 } else { 1e-4 });
        }
        resident.set_voltage_expressions(&[None]).unwrap();
        let moving =
            Particle::new(1.0, 0.0, [1e9 + 0.25, 1.0, 2.0], [10.0, 0.0, 0.0], 0.0).unwrap();
        let auto = resident
            .fly(
                std::slice::from_ref(&moving),
                &FlightOptions {
                    duration_us: 1.0,
                    initial_step_us: 0.2,
                    max_step_us: f64::MAX,
                    ..Default::default()
                },
            )
            .unwrap();
        let one_cell = resident
            .fly(
                std::slice::from_ref(&moving),
                &FlightOptions {
                    duration_us: 1.0,
                    initial_step_us: 0.2,
                    max_step_us: f64::MAX,
                    step_limits: crate::StepLimits {
                        max_grid_cells: 1.0,
                        ..Default::default()
                    },
                    ..Default::default()
                },
            )
            .unwrap();
        assert!(one_cell.results[0].accepted_steps >= 6);
        assert!(auto.results[0].accepted_steps < one_cell.results[0].accepted_steps);
        assert_eq!(
            auto.results[0].splat_reason,
            one_cell.results[0].splat_reason
        );
        assert!(
            (auto.results[0].tof_us - one_cell.results[0].tof_us).abs()
                < if shared.wide { 1e-9 } else { 1e-4 }
        );
        let neutral = Particle::new(1.0, 0.0, [1e9 + 0.25, 1.0, 2.0], [0.0; 3], 0.0).unwrap();
        let particles = vec![neutral; MAX_IONS_PER_DISPATCH + 3];
        resident
            .set_max_ions_per_dispatch(MAX_IONS_PER_DISPATCH)
            .unwrap();
        let options = FlightOptions {
            duration_us: 0.01,
            max_step_us: f64::MAX,
            ..Default::default()
        };
        let indices = [
            0,
            MAX_IONS_PER_DISPATCH - 1,
            MAX_IONS_PER_DISPATCH,
            MAX_IONS_PER_DISPATCH + 2,
        ];
        let result = resident
            .fly_sampled(
                &particles,
                &options,
                &TrajectorySampling {
                    ion_indices: &indices,
                    every: 2,
                    max_samples_per_ion: 8,
                },
            )
            .unwrap();
        for (i, result) in result.results.iter().enumerate() {
            assert_eq!(result.splat_reason, SplatReason::TimeLimit);
            if indices.contains(&i) {
                assert_eq!(
                    result.trajectory.as_ref().unwrap().len(),
                    1 + result.accepted_steps.div_ceil(2)
                );
            } else {
                assert!(result.trajectory.is_none());
            }
        }
    }
}
