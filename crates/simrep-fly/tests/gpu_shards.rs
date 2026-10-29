use simrep_core::PotentialArray;
use simrep_fly::{
    FlightOptions, GridField, InstanceField, Particle,
    gpu::{Backends, Gpu, TrajectorySampling},
    physics::{
        collision::{CollisionModel, Gas, Hs1},
        magnetic::MagneticField,
    },
};

#[test]
fn interleaved_gpu_streams_and_samples_match_unsharded_across_dispatch_boundaries() {
    let Some(gpu) = pollster::block_on(Gpu::new(Backends::VULKAN)).unwrap() else {
        eprintln!("SKIP GPU shard test: no Vulkan adapter");
        return;
    };
    let electric = InstanceField::new(
        GridField::new(PotentialArray::new(2, 2, 2)).unwrap(),
        [0.0; 3],
        [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        1.0,
        Some([[-100.0; 3], [100.0; 3]]),
        None,
    )
    .unwrap();
    let magnetic = MagneticField::default();
    let model = CollisionModel::Hs1(Hs1 {
        gas: Gas {
            pressure_pa: 53.0,
            ..Default::default()
        },
        ..Default::default()
    });
    let particles: Vec<_> = (0..15)
        .map(|k| {
            Particle::new(200.0, 1.0, [0.5; 3], [0.3, 0.1, 0.0], (k % 3) as f64 * 0.01).unwrap()
        })
        .collect();
    let options = FlightOptions {
        duration_us: 0.05,
        max_step_us: 0.001,
        relativistic: false,
        ..Default::default()
    };
    let mut prepared = gpu
        .prepare_physics(&electric, &[], &magnetic, Some(&model), 813)
        .unwrap();
    prepared.set_max_ions_per_dispatch(7).unwrap();
    let full_indices: Vec<_> = (0..particles.len()).collect();
    let full = prepared
        .fly_sampled(
            &particles,
            &options,
            &TrajectorySampling {
                ion_indices: &full_indices,
                every: 3,
                max_samples_per_ion: 1024,
            },
        )
        .unwrap();
    assert!(
        full.results
            .windows(2)
            .any(|w| w[0].final_state.velocity_mm_per_us != w[1].final_state.velocity_mm_per_us)
    );
    prepared.set_max_ions_per_dispatch(2).unwrap();
    for count in [1, 2, 3, 7] {
        for offset in 0..count {
            let indices: Vec<_> = (0..particles.len())
                .filter(|k| k % count == offset)
                .collect();
            let subset: Vec<_> = indices.iter().map(|&k| particles[k].clone()).collect();
            let local: Vec<_> = (0..subset.len()).collect();
            let shard = prepared
                .fly_interleaved(
                    &subset,
                    &options,
                    offset as u64,
                    count as u64,
                    Some(&TrajectorySampling {
                        ion_indices: &local,
                        every: 3,
                        max_samples_per_ion: 1024,
                    }),
                )
                .unwrap();
            for (result, &index) in shard.results.iter().zip(&indices) {
                let original = &full.results[index];
                assert_eq!(
                    result.final_state, original.final_state,
                    "N={count}, k={index}"
                );
                assert_eq!(result.tof_us.to_bits(), original.tof_us.to_bits());
                assert_eq!(result.splat_reason, original.splat_reason);
                assert_eq!(result.trajectory, original.trajectory);
            }
        }
    }
}
