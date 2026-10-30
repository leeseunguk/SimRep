//! Partitioning must not change any per-ion arithmetic in either precision.
use std::sync::Arc;

use simrep_core::PotentialArray;
use simrep_fly::{
    FlightOptions, GridField, InstanceField, Particle,
    gpu::{Backends, Gpu, Precision, TrajectorySampling},
    surface::SurfaceModel,
};
use simrep_geom::{Sdf, SignedDistance};

fn gpu(precision: Precision, timestamps: bool) -> Gpu {
    if timestamps {
        return pollster::block_on(Gpu::new_with_precision(Backends::VULKAN, precision))
            .unwrap()
            .expect("Vulkan adapter required for SDF dispatch regression");
    }
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: Backends::VULKAN,
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let adapter = pollster::block_on(instance.request_adapter(&Default::default())).unwrap();
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_features: if precision == Precision::Precise {
            adapter.features() & wgpu::Features::SHADER_F64
        } else {
            wgpu::Features::empty()
        },
        required_limits: adapter.limits(),
        ..Default::default()
    }))
    .unwrap();
    let gpu = pollster::block_on(Gpu::from_device_with_precision(
        device,
        queue,
        adapter.get_info(),
        precision,
    ))
    .unwrap();
    assert!(!gpu.has_timestamps());
    gpu
}

#[test]
fn sdf_tiny_dispatches_preserve_terminal_states() {
    let mut array = PotentialArray::new(21, 7, 1);
    for (i, potential) in array.potentials.iter_mut().enumerate() {
        *potential = -((i % 21) as f64);
    }
    let surface = SurfaceModel::from_sdfs(
        vec![(
            3,
            Arc::new(Sdf::Box {
                center: [9.5, 3.0, 0.0],
                half_extent: [0.5, 4.0, 1.0],
            }) as Arc<dyn SignedDistance>,
        )],
        1e-12,
    )
    .unwrap()
    .with_pa_domain(&array, [0.0; 3])
    .unwrap();
    let field = InstanceField::new(
        GridField::new(array).unwrap(),
        [0.0; 3],
        [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        1.0,
        None,
        Some(([1.0, 0.0, 0.0], 20.0)),
    )
    .unwrap()
    .with_surface(Arc::new(surface))
    .unwrap();
    let particles: Vec<_> = (0..65)
        .map(|i| {
            Particle::new(
                1.0,
                1.0,
                [3.0, 1.0 + f64::from(i) / 20.0, 0.0],
                [if i % 2 == 0 { 100.0 } else { -100.0 }, 0.0, 0.0],
                0.0,
            )
            .unwrap()
        })
        .collect();
    let options = FlightOptions {
        step_limits: simrep_fly::StepLimits {
            max_grid_cells: 0.05,
            ..Default::default()
        },
        ..Default::default()
    };
    let indices = [0, 2, 3, 32, 64];
    let sampling = TrajectorySampling {
        ion_indices: &indices,
        every: 1,
        max_samples_per_ion: 256,
    };
    for timestamps in [true, false] {
        for precision in [Precision::Precise, Precision::Preview] {
            let gpu = gpu(precision, timestamps);
            for divisor in [2, 4] {
                eprintln!(
                    "SDF partition: {precision:?}, timestamps={}, N={divisor}",
                    gpu.has_timestamps()
                );
                let field = field.clone().with_surface_sdf_resolution(divisor).unwrap();
                let mut resident = gpu.prepare(&field, &[]).unwrap();
                assert!(resident.set_max_ions_per_dispatch(0).is_err());
                resident.set_max_ions_per_dispatch(particles.len()).unwrap();
                let unsplit = resident
                    .fly_sampled(&particles, &options, &sampling)
                    .unwrap();
                assert!(unsplit.results.iter().all(|r| r.accepted_steps > 32));
                resident.set_max_ions_per_dispatch(3).unwrap();
                let pending = resident
                    .submit_sampled(&particles, &options, &sampling)
                    .unwrap();
                // Deferred chunks retain the submitted voltages and pipeline even
                // after another flight changes the resident field's cell cache.
                resident.set_voltages(&[2.0]).unwrap();
                resident.fly(&particles[..1], &options).unwrap();
                let split = pending.collect().unwrap();
                assert_eq!(split.results.len(), unsplit.results.len());
                for (a, b) in unsplit.results.iter().zip(&split.results) {
                    assert_eq!(a.splat_reason, b.splat_reason);
                    assert_eq!(
                        a.final_state.position_mm.map(f64::to_bits),
                        b.final_state.position_mm.map(f64::to_bits)
                    );
                    assert_eq!(
                        a.final_state.velocity_mm_per_us.map(f64::to_bits),
                        b.final_state.velocity_mm_per_us.map(f64::to_bits)
                    );
                    assert_eq!(
                        a.final_state.time_us.to_bits(),
                        b.final_state.time_us.to_bits()
                    );
                    assert_eq!(a.tof_us.to_bits(), b.tof_us.to_bits());
                    assert_eq!(a.accepted_steps, b.accepted_steps);
                    assert_eq!(a.rejected_steps, b.rejected_steps);
                    assert_eq!(a.trajectory, b.trajectory);
                }
                assert!(resident.fly(&[], &options).unwrap().results.is_empty());
            }
        }
    }
}
