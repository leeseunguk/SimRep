use simrep_core::{PotentialArray, Symmetry};
use simrep_fly::{
    FlightOptions, GridField, InstanceField, Particle, SplatReason, WorkbenchField, fly_many,
    gpu::{Backends, Gpu},
};

const AXES: [[f64; 3]; 3] = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

fn instance(array: PotentialArray) -> InstanceField {
    InstanceField::new(
        GridField::new(array).unwrap(),
        [0.0; 3],
        AXES,
        1.0,
        None,
        None,
    )
    .unwrap()
}

fn compare(gpu: &Gpu, field: &InstanceField, particles: &[Particle], options: &FlightOptions) {
    let cpu = fly_many(field, particles, options).unwrap();
    let mut resident = gpu.prepare(field, &[]).unwrap();
    let actual = resident.fly(particles, options).unwrap();
    for (a, b) in actual.results.iter().zip(cpu) {
        assert_eq!(a.splat_reason, b.splat_reason, "GPU {a:?}; CPU {b:?}");
        let distance = a
            .final_state
            .position_mm
            .iter()
            .zip(b.final_state.position_mm)
            .map(|(x, y)| (x - y).powi(2))
            .sum::<f64>()
            .sqrt();
        assert!(
            distance < 2e-4,
            "endpoint difference {distance:e}: {a:?} vs {b:?}"
        );
        assert!(
            (a.tof_us - b.tof_us).abs() < 1e-5 * b.tof_us.max(1.0),
            "TOF: {} vs {}",
            a.tof_us,
            b.tof_us
        );
    }
}

#[test]
fn gpu_rk45_geometry_precision_and_resident_voltages() {
    let Some(gpu) = pollster::block_on(Gpu::new(Backends::VULKAN)).unwrap() else {
        eprintln!("SKIP GPU flyer: no Vulkan adapter");
        return;
    };
    eprintln!("GPU flyer: {} ({:?})", gpu.info.name, gpu.info.device_type);
    let mut array = PotentialArray::new(9, 7, 1);
    for y in 0..array.ny {
        for x in 0..array.nx {
            array.potentials[y * array.nx + x] = -0.2 * x as f64;
        }
    }
    let particle = Particle::new(40.0, 1.0, [0.1, 2.0, 0.0], [1.0, 0.0, 0.0], 1e12).unwrap();
    let options = FlightOptions {
        duration_us: 20.0,
        max_step_us: 0.8,
        ..Default::default()
    };
    compare(
        &gpu,
        &instance(array.clone()),
        std::slice::from_ref(&particle),
        &options,
    );
    // Keep the historical one-node voxel option covered explicitly.
    array.electrodes[4 + 2 * array.nx] = true;
    let nearest = InstanceField::new(
        GridField::new(array.clone())
            .unwrap()
            .with_nearest_node_collisions(),
        [0.0; 3],
        AXES,
        1.0,
        None,
        None,
    )
    .unwrap();
    compare(&gpu, &nearest, std::slice::from_ref(&particle), &options);
    // A fully occupied cell must not disappear between accepted endpoints.
    for y in [2, 3] {
        for x in [4, 5] {
            array.electrodes[y * array.nx + x] = true;
        }
    }
    compare(
        &gpu,
        &instance(array.clone()),
        std::slice::from_ref(&particle),
        &options,
    );
    let field = instance(array.clone());
    let bases = [GridField::new(array.clone()).unwrap()];
    let mut resident = gpu.prepare(&field, &bases).unwrap();
    resident.set_voltages(&[0.0, 1.0]).unwrap();
    let one = resident
        .fly(std::slice::from_ref(&particle), &options)
        .unwrap();
    resident.set_voltages(&[0.0, 2.0]).unwrap();
    let two = resident
        .fly(std::slice::from_ref(&particle), &options)
        .unwrap();
    assert!(two.results[0].tof_us < one.results[0].tof_us);
    assert_eq!(
        two.results[0].splat_reason,
        SplatReason::Electrode { index: 22 }
    );
    // Cylindrical shells, mirrors and field-free workbench drift.
    array.electrodes.fill(false);
    array.symmetry = Symmetry::Cylindrical;
    array.mirrors.x = true;
    for y in [3, 4] {
        for x in [4, 5] {
            array.electrodes[y * array.nx + x] = true;
        }
    }
    let field = InstanceField::new(
        GridField::new(array).unwrap(),
        [0.0; 3],
        AXES,
        1.0,
        Some([[-12.0; 3], [12.0; 3]]),
        Some(([1.0, 1.0, 0.0], 9.0)),
    )
    .unwrap();
    let particles = [
        Particle::new(40.0, 1.0, [-2.0, 2.0, 1.0], [1.0, 0.02, 0.0], 0.0).unwrap(),
        Particle::new(40.0, 0.0, [0.0, 2.0, 1.0], [1.0, 0.4, 0.1], 0.0).unwrap(),
    ];
    compare(&gpu, &field, &particles, &options);
    // Large workbench translations must not round away in-cell motion.
    let field = InstanceField::new(
        GridField::new(PotentialArray::new(2, 2, 1)).unwrap(),
        [1e9, 0.0, 0.0],
        AXES,
        1.0,
        Some([[1e9 - 10.0, -10.0, -10.0], [1e9 + 10.0, 10.0, 10.0]]),
        Some(([1.0, 0.0, 0.0], 1e9 + 5.0)),
    )
    .unwrap();
    let ion = Particle::new(1.0, 0.0, [1e9 + 0.125, 0.5, 0.0], [1.0, 0.0, 0.0], 0.0).unwrap();
    compare(&gpu, &field, &[ion], &options);

    let mut a = PotentialArray::new(7, 7, 7);
    for z in 0..7 {
        for y in 0..7 {
            for x in 0..7 {
                let i = (z * 7 + y) * 7 + x;
                let xf = x as f64;
                a.potentials[i] = -0.1 * xf - 0.01 * xf.powi(3) - 0.02 * xf * y as f64;
                a.electrodes[i] = x >= 5;
            }
        }
    }
    let field = InstanceField::new(
        GridField::new(a).unwrap(),
        [100.0, -50.0, 20.0],
        [[0.0, 1.0, 0.0], [-1.0, 0.0, 0.0], [0.0, 0.0, 1.0]],
        2.0,
        None,
        None,
    )
    .unwrap();
    let ion = Particle::new(1000.0, 1.0, [96.0, -49.8, 24.0], [0.0, 2.0, 0.0], 0.0).unwrap();
    compare(&gpu, &field, &[ion], &options);

    // Static workbenches must read each instance's background, without the
    // expression path or accidentally reusing the first combined buffer.
    let mut array = PotentialArray::new(5, 3, 1);
    for y in 0..3 {
        for x in 0..5 {
            array.potentials[y * 5 + x] = -0.2 * x as f64;
        }
    }
    let first = instance(array.clone());
    for v in &mut array.potentials {
        *v *= 2.0;
    }
    let second = InstanceField::new(
        GridField::new(array).unwrap(),
        [8.0, 0.0, 0.0],
        AXES,
        1.0,
        None,
        None,
    )
    .unwrap();
    let workbench =
        WorkbenchField::new(vec![first, second], [[-1.0; 3], [13.0, 3.0, 1.0]]).unwrap();
    let particles = [Particle::new(40.0, 1.0, [0.1, 1.0, 0.0], [1.0, 0.0, 0.0], 0.0).unwrap()];
    let options = FlightOptions {
        max_step_us: 0.03,
        duration_us: 20.0,
        ..Default::default()
    };
    let cpu = fly_many(&workbench, &particles, &options).unwrap();
    let mut resident = gpu.prepare_workbench(&workbench).unwrap();
    let actual = resident.fly(&particles, &options).unwrap();
    assert_eq!(actual.results[0].splat_reason, cpu[0].splat_reason);
    let tolerance = if gpu.has_f64() { 1e-7 } else { 2e-4 };
    assert!((actual.results[0].tof_us - cpu[0].tof_us).abs() < tolerance);

    // Preserve a duration's low bits and accumulate thousands of short steps.
    let field = instance(PotentialArray::new(2, 2, 1));
    let ion = Particle::new(1.0, 0.0, [0.5, 0.5, 0.0], [0.0; 3], 1e12).unwrap();
    let options = FlightOptions {
        duration_us: 10.123456789,
        initial_step_us: 0.003,
        max_step_us: 0.003,
        ..Default::default()
    };
    let mut resident = gpu.prepare(&field, &[]).unwrap();
    let report = resident.fly(&[ion], &options).unwrap();
    assert_eq!(report.results[0].splat_reason, SplatReason::TimeLimit);
    assert!(
        (report.results[0].tof_us - options.duration_us).abs() < 2e-10,
        "two-float TOF: {:.15e}",
        report.results[0].tof_us
    );

    // Cross the bounded-dispatch size and retain order through the short tail.
    let particles: Vec<_> = (0..32_771)
        .map(|i| {
            Particle::new(
                1.0,
                0.0,
                [0.25 + 0.5 * i as f64 / 32_771.0, 0.5, 0.0],
                [0.0; 3],
                i as f64,
            )
            .unwrap()
        })
        .collect();
    let report = resident
        .fly(
            &particles,
            &FlightOptions {
                duration_us: 0.0,
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(report.results.len(), particles.len());
    for (actual, expected) in report.results.iter().zip(&particles) {
        assert_eq!(actual.splat_reason, SplatReason::TimeLimit);
        assert_eq!(actual.tof_us, 0.0);
        assert_eq!(actual.final_state.time_us, expected.initial.time_us);
        assert!((actual.final_state.position_mm[0] - expected.initial.position_mm[0]).abs() < 1e-7);
    }
}
