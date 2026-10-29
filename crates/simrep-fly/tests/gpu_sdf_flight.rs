//! Software Vulkan is intentional for correctness, never a hardware speed claim.
use std::{sync::Arc, time::Instant};

use simrep_core::PotentialArray;
use simrep_fly::{
    FlightOptions, GridField, InstanceField, Particle, SplatReason, fly_many,
    gpu::{Backends, Gpu, Precision},
    surface::SurfaceModel,
};
use simrep_geom::{Sdf, SignedDistance};

const AXES: [[f64; 3]; 3] = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

fn distance(a: &simrep_fly::FlightResult, b: &simrep_fly::FlightResult) -> f64 {
    a.final_state
        .position_mm
        .iter()
        .zip(b.final_state.position_mm)
        .map(|(a, b)| (a - b).powi(2))
        .sum::<f64>()
        .sqrt()
}

fn save_pairs(
    path: &std::path::Path,
    reference: &[simrep_fly::FlightResult],
    actual: &[simrep_fly::FlightResult],
) {
    let pairs: Vec<_> = reference.iter().zip(actual).enumerate().map(|(i,(a,b))|serde_json::json!({
        "ion":i+1,"reference_reason":format!("{:?}",a.splat_reason),"actual_reason":format!("{:?}",b.splat_reason),
        "reference_position_mm":a.final_state.position_mm,"actual_position_mm":b.final_state.position_mm,
        "reference_tof_us":a.tof_us,"actual_tof_us":b.tof_us,
    })).collect();
    std::fs::write(path, serde_json::to_vec(&pairs).unwrap()).unwrap();
}

fn compare(
    gpu: &Gpu,
    field: &InstanceField,
    particles: &[Particle],
    label: &str,
) -> serde_json::Value {
    let options = FlightOptions {
        automatic_tolerance: false,
        relativistic: true,
        ..FlightOptions::default()
    };
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(16)
        .build()
        .unwrap();
    let start = Instant::now();
    let cpu = pool
        .install(|| fly_many(field, particles, &options))
        .unwrap();
    let cpu_ms = start.elapsed().as_secs_f64() * 1000.0;
    if std::env::var_os("SIMREP_GPU_SDF_QA_DIR").is_some() {
        assert!(
            gpu.has_timestamps(),
            "compute-only QA requires GPU timestamp queries"
        );
    }
    let mut resident = gpu.prepare(field, &[]).unwrap();
    // llvmpipe can JIT inside its first timestamped dispatch. Warm the same
    // resident pipeline before recording compute-only flight time.
    resident.fly(&particles[..1], &options).unwrap();
    let report = resident.fly(particles, &options).unwrap();
    assert_eq!(cpu.len(), report.results.len());
    if let Some(root) = std::env::var_os("SIMREP_GPU_SDF_QA_DIR") {
        let mode = if gpu.has_f64() { "f64" } else { "f32" };
        save_pairs(
            &std::path::PathBuf::from(root).join(format!("{label}-{mode}-pairs.json")),
            &cpu,
            &report.results,
        );
    }
    let mut maximum = 0.0_f64;
    let mut squared = 0.0;
    let mut tof = 0.0_f64;
    let mut mismatches = Vec::new();
    for (i, (a, b)) in cpu.iter().zip(&report.results).enumerate() {
        let d = distance(a, b);
        maximum = maximum.max(d);
        squared += d * d;
        tof = tof.max((a.tof_us - b.tof_us).abs());
        if a.splat_reason != b.splat_reason {
            mismatches.push(serde_json::json!({"ion":i+1,"cpu":format!("{:?}",a.splat_reason),"gpu":format!("{:?}",b.splat_reason),"hit_distance_mm":d,"cpu_position_mm":a.final_state.position_mm,"gpu_position_mm":b.final_state.position_mm}));
        }
    }
    let metrics = serde_json::json!({"case":label,"precision":if gpu.has_f64(){"f64"}else{"mixed-f32-f64"},"software":gpu.info.device_type==simrep_fly::gpu::DeviceType::Cpu,"gpu_timestamp_queries":gpu.has_timestamps(),"ions":cpu.len(),"agreement":cpu.len()-mismatches.len(),"mismatches":mismatches,"position_max_mm":maximum,"position_rms_mm":(squared/cpu.len() as f64).sqrt(),"tof_max_diff_us":tof,"cpu_sdf_compute_ms":cpu_ms,"gpu_sdf_compute_ms":report.timings.fly.as_secs_f64()*1000.0});
    println!("GPU_SDF_FLIGHT {metrics}");
    assert!(mismatches.len() <= cpu.len() / 1000, "{metrics}");
    assert!(
        maximum < if gpu.has_f64() { 1e-4 } else { 0.1 },
        "{metrics}"
    );
    assert!(tof < if gpu.has_f64() { 1e-5 } else { 1e-3 }, "{metrics}");
    metrics
}

#[test]
fn gpu_sdf_einzel_precise_and_preview() {
    let Some(probe) =
        pollster::block_on(Gpu::new(Backends::VULKAN)).unwrap_or_else(|e| panic!("{e}"))
    else {
        eprintln!("SKIP GPU SDF flight: no Vulkan adapter");
        return;
    };
    println!("GPU_SDF_ADAPTER {:?}", probe.info);
    let qa = std::env::var_os("SIMREP_GPU_SDF_QA_DIR").map(std::path::PathBuf::from);
    let (array, surface, particles) = if let Some(root) = &qa {
        let array = simrep_io::read_pa(root.join("einzel.pa0")).unwrap().array;
        let surface =
            SurfaceModel::from_reader(std::fs::File::open(root.join("einzel.cuts.json")).unwrap())
                .unwrap()
                .with_tolerance(1e-12)
                .unwrap();
        let particles = simrep_io::read_ion(root.join("final-beam.ion"))
            .unwrap()
            .into_iter()
            .map(|p| {
                Particle::from_relativistic_energy(
                    p.mass_amu,
                    p.charge_e,
                    p.position_mm,
                    p.energy_ev,
                    p.direction,
                    p.birth_time_us,
                )
                .unwrap()
            })
            .collect::<Vec<_>>();
        (array, surface, particles)
    } else {
        // A zero-voltage einzel is a reproducible collision regression. The
        // optional refined fixture above exercises the energized 2,400-ion QA.
        let gem = simrep_gem::parse_gem(include_str!("../../../examples/data/einzel.gem")).unwrap();
        let mut array = gem.rasterize().unwrap().pa.array;
        array.potentials.fill(0.0);
        let shapes = gem
            .electrode_sdfs()
            .unwrap()
            .into_iter()
            .map(|s| (s.voltage as usize, Arc::new(s) as Arc<dyn SignedDistance>))
            .collect();
        let surface = SurfaceModel::from_sdfs(shapes, 1e-12)
            .unwrap()
            .with_pa_domain(&array, [0.0; 3])
            .unwrap();
        let particles = (0..64)
            .map(|i| {
                let angle = (-11.0 + 22.0 * i as f64 / 63.0).to_radians();
                Particle::from_relativistic_energy(
                    40.0,
                    1.0,
                    [1.0, 1.0, 0.0],
                    500.0,
                    [angle.cos(), angle.sin(), 0.0],
                    0.0,
                )
                .unwrap()
            })
            .collect();
        (array, surface, particles)
    };
    let exact = InstanceField::new(
        GridField::new(array).unwrap(),
        [0.0; 3],
        AXES,
        1.0,
        None,
        Some(([1.0, 0.0, 0.0], 58.0)),
    )
    .unwrap()
    .with_surface(Arc::new(surface))
    .unwrap();
    let field = exact.clone().with_surface_sdf_resolution(2).unwrap();
    let mut metrics = Vec::new();
    for precision in [Precision::Precise, Precision::Preview] {
        let gpu = if precision == Precision::Precise {
            None
        } else {
            pollster::block_on(Gpu::new_with_precision(Backends::VULKAN, precision))
                .unwrap_or_else(|e| panic!("{e}"))
        };
        metrics.push(compare(
            gpu.as_ref().unwrap_or(&probe),
            &field,
            &particles,
            "einzel",
        ));
    }
    if let Some(root) = qa {
        let slab_array = simrep_io::read_pa(root.join("slab.pa0")).unwrap().array;
        let slab_surface =
            SurfaceModel::from_reader(std::fs::File::open(root.join("slab.cuts.json")).unwrap())
                .unwrap()
                .with_tolerance(1e-12)
                .unwrap()
                .select_collision_only(true);
        let slab = InstanceField::new(
            GridField::new(slab_array).unwrap(),
            [0.0; 3],
            AXES,
            1.0,
            None,
            Some(([1.0, 0.0, 0.0], 18.0)),
        )
        .unwrap()
        .with_collision_surface(Arc::new(slab_surface))
        .unwrap()
        .with_surface_sdf_resolution(2)
        .unwrap();
        let slab_particles = simrep_io::read_ion(root.join("slab.ion"))
            .unwrap()
            .into_iter()
            .map(|p| {
                Particle::from_relativistic_energy(
                    p.mass_amu,
                    p.charge_e,
                    p.position_mm,
                    p.energy_ev,
                    p.direction,
                    p.birth_time_us,
                )
                .unwrap()
            })
            .collect::<Vec<_>>();
        for precision in [Precision::Precise, Precision::Preview] {
            let gpu = pollster::block_on(Gpu::new_with_precision(Backends::VULKAN, precision))
                .unwrap()
                .unwrap();
            metrics.push(compare(
                &gpu,
                &slab,
                &slab_particles,
                "refined-insulator-slab",
            ));
        }
        let options = FlightOptions {
            automatic_tolerance: false,
            relativistic: true,
            ..FlightOptions::default()
        };
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(16)
            .build()
            .unwrap();
        let start = Instant::now();
        let reference = pool
            .install(|| fly_many(&exact, &particles, &options))
            .unwrap();
        let exact_ms = start.elapsed().as_secs_f64() * 1000.0;
        let sampled = pool
            .install(|| fly_many(&field, &particles, &options))
            .unwrap();
        save_pairs(
            &root.join("cpu-exact-vs-sdf-pairs.json"),
            &reference,
            &sampled,
        );
        let agreement = reference
            .iter()
            .zip(&sampled)
            .filter(|(a, b)| a.splat_reason == b.splat_reason)
            .count();
        let errors = reference
            .iter()
            .zip(&sampled)
            .map(|(a, b)| distance(a, b))
            .collect::<Vec<_>>();
        let summary = serde_json::json!({"gpu":metrics,"cpu_exact_compute_ms":exact_ms,"cpu_exact_vs_sdf":{"agreement":agreement,"position_max_mm":errors.iter().copied().fold(0.0,f64::max),"position_rms_mm":(errors.iter().map(|d|d*d).sum::<f64>()/errors.len() as f64).sqrt(),"tof_max_diff_us":reference.iter().zip(&sampled).map(|(a,b)|(a.tof_us-b.tof_us).abs()).fold(0.0,f64::max)},"electrode_hits":reference.iter().filter(|r|matches!(r.splat_reason,SplatReason::SurfaceElectrode{..})).count()});
        std::fs::write(
            root.join("gpu-flight-metrics.json"),
            format!("{summary:#}\n"),
        )
        .unwrap();
    }
}

#[test]
fn gpu_sdf_collision_surface_preserves_staircase_and_launch() {
    let Some(gpu) =
        pollster::block_on(Gpu::new(Backends::VULKAN)).unwrap_or_else(|e| panic!("{e}"))
    else {
        return;
    };
    let mut array = PotentialArray::new(21, 7, 1);
    for y in 0..7 {
        for x in [1, 2, 18, 19] {
            array.electrodes[y * 21 + x] = true;
        }
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
        AXES,
        1.0,
        None,
        Some(([1.0, 0.0, 0.0], 20.0)),
    )
    .unwrap()
    .with_collision_surface(Arc::new(surface))
    .unwrap()
    .with_surface_sdf_resolution(2)
    .unwrap();
    let particles = vec![
        Particle::new(1.0, 1.0, [3.0, 3.0, 0.0], [100.0, 0.0, 0.0], 0.0).unwrap(),
        Particle::new(1.0, 1.0, [3.0, 3.0, 0.0], [-100.0, 0.0, 0.0], 0.0).unwrap(),
        Particle::new(1.0, 1.0, [9.5, 3.0, 0.0], [100.0, 0.0, 0.0], 0.0).unwrap(),
    ];
    compare(&gpu, &field, &particles, "insulator-slab");
    let report = gpu
        .prepare(&field, &[])
        .unwrap()
        .fly(&particles, &FlightOptions::default())
        .unwrap();
    assert_eq!(
        report.results[0].splat_reason,
        SplatReason::SurfaceElectrode { number: 3 }
    );
    assert!(matches!(
        report.results[1].splat_reason,
        SplatReason::Electrode { .. }
    ));
    assert_eq!(report.results[2].tof_us, 0.0);
    let launch = gpu
        .prepare(&field, &[])
        .unwrap()
        .fly_from_electrode(&particles[2..], &FlightOptions::default())
        .unwrap();
    println!("SDF_LAUNCH {:?}", launch.results[0]);
    assert!(matches!(
        launch.results[0].splat_reason,
        SplatReason::Electrode { .. }
    ));
    assert!(launch.results[0].tof_us > 0.0);
    // One accepted chord crosses both the detector at x=8 and SDF at x=9.
    let plane_options = FlightOptions {
        initial_step_us: 0.1,
        max_step_us: 0.1,
        stop_plane: Some(simrep_fly::StopPlane {
            axis: 0,
            coordinate_mm: 8.0,
        }),
        ..FlightOptions::default()
    };
    let plane = gpu
        .prepare(&field, &[])
        .unwrap()
        .fly(&particles[..1], &plane_options)
        .unwrap();
    println!("SDF_PLANE_FIRST {:?}", plane.results[0]);
    assert_eq!(plane.results[0].splat_reason, SplatReason::StopPlane);
    assert!((plane.results[0].final_state.position_mm[0] - 8.0).abs() < 1e-9);
}
