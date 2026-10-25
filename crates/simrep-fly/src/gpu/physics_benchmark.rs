//! Opt-in native GPU physics replay. Inputs remain in the installed oracle tree.
use super::*;
use crate::physics::{
    collision::{CollisionModel, DiffusionStatistics, Gas, Hs1, Sds, fly_many_collisional},
    magnetic::{Coil, Electromagnetic, MagneticField, MagneticPa},
};
use simrep_core::{FieldType, PotentialArray};
use std::{io::Write, path::Path};

fn numbers(row: &str) -> Vec<f64> {
    row.split(',').map(|v| v.parse().unwrap()).collect()
}

fn load(
    root: &Path,
    name: &str,
) -> (
    Electromagnetic<InstanceField, MagneticField>,
    Vec<Particle>,
    Vec<[f64; 4]>,
    f64,
) {
    let folder = root.join(name);
    let metadata = std::fs::read_to_string(folder.join("metadata.log")).unwrap();
    let log = std::fs::read_to_string(folder.join("flight.log")).unwrap();
    let bounds = numbers(
        metadata
            .lines()
            .find_map(|l| l.strip_prefix("BOUNDS,"))
            .unwrap(),
    );
    let bounds = Some([
        bounds[..3].try_into().unwrap(),
        bounds[3..].try_into().unwrap(),
    ]);
    let identity = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    let mut electric = InstanceField::new(
        GridField::new(PotentialArray::new(1, 1, 1)).unwrap(),
        [0.0; 3],
        identity,
        1.0,
        bounds,
        None,
    )
    .unwrap();
    let mut magnetic = MagneticField::default();
    for row in metadata.lines().filter_map(|l| l.strip_prefix("INSTANCE,")) {
        let columns: Vec<_> = row.split(',').collect();
        let file = simrep_io::read_pa(folder.join(format!("instance{}.pa", columns[0]))).unwrap();
        let v: Vec<f64> = columns[2..].iter().map(|v| v.parse().unwrap()).collect();
        let origin = v[..3].try_into().unwrap();
        let axes = std::array::from_fn(|a| std::array::from_fn(|b| v[3 + 3 * a + b]));
        if file.array.field_type == FieldType::Magnetic {
            let mut pa =
                MagneticPa::placed(file.array, f64::from(file.ng), origin, axes, v[12], bounds)
                    .unwrap();
            if name == "sector_3dp" {
                pa = pa.with_polarity_flip_axis(0).unwrap();
            }
            if name == "sector_2dc" {
                pa = pa.with_angular_sector(0, 0.0, 90.0).unwrap();
            }
            if matches!(name, "helmholtz" | "solenoid") {
                magnetic.support = Some(pa);
            } else {
                magnetic.arrays.push(pa);
            }
        } else {
            electric = InstanceField::new(
                GridField::new(file.array).unwrap(),
                origin,
                axes,
                v[12],
                bounds,
                None,
            )
            .unwrap();
        }
    }
    if name == "helmholtz" {
        magnetic.coils = [-15.0, 15.0]
            .map(|z| Coil::hoop([0.0, 0.0, z], [0.0, 0.0, 1.0], 30.0, 100.0).unwrap())
            .to_vec();
    } else if name == "solenoid" {
        magnetic.coils =
            MagneticField::solenoid([-50.0, 0.0, 0.0], [50.0, 0.0, 0.0], 10.0, 0.7958, 100)
                .unwrap()
                .coils;
    }
    let particles = log
        .lines()
        .filter_map(|l| l.strip_prefix("START,"))
        .map(|row| {
            let (_, row) = row.split_once(',').unwrap();
            let v = numbers(row);
            Particle::new(
                v[1],
                v[2],
                v[3..6].try_into().unwrap(),
                v[6..9].try_into().unwrap(),
                v[0],
            )
            .unwrap()
        })
        .collect();
    let oracle = log
        .lines()
        .filter_map(|l| l.strip_prefix("FINAL,"))
        .map(|row| {
            let (index, row) = row.split_once(',').unwrap();
            let v = numbers(row);
            (index.parse::<usize>().unwrap(), [v[0], v[1], v[2], v[3]])
        })
        .collect::<std::collections::BTreeMap<_, _>>()
        .into_values()
        .collect();
    let simion = log
        .lines()
        .find_map(|l| {
            l.split_once(" splats, ")
                .and_then(|(_, v)| v.split_whitespace().next())
                .and_then(|v| v.parse().ok())
        })
        .unwrap();
    (
        Electromagnetic { electric, magnetic },
        particles,
        oracle,
        simion,
    )
}

fn errors(actual: &[FlightResult], reference: &[FlightResult]) -> (f64, f64) {
    actual
        .iter()
        .zip(reference)
        .fold((0.0_f64, 0.0_f64), |(p, t), (a, b)| {
            assert_eq!(a.splat_reason, b.splat_reason);
            (
                p.max(
                    (0..3)
                        .map(|k| {
                            (a.final_state.position_mm[k] - b.final_state.position_mm[k]).powi(2)
                        })
                        .sum::<f64>()
                        .sqrt(),
                ),
                t.max((a.tof_us - b.tof_us).abs()),
            )
        })
}

#[test]
fn gpu_physics_reference_agreement() {
    let Ok(root) = std::env::var("SIMREP_GPU_PHYSICS") else {
        return;
    };
    let gpu = pollster::block_on(Gpu::new(Backends::VULKAN))
        .unwrap()
        .expect("native Vulkan adapter");
    assert!(gpu.has_f64(), "reference acceptance requires native f64");
    eprintln!(
        "Adapter: {} {:?} {:?}",
        gpu.info.name, gpu.info.backend, gpu.info.device_type
    );
    for name in [
        "magnet1",
        "mag90",
        "sector_2dp",
        "sector_2dc",
        "sector_3dp",
        "helmholtz",
        "solenoid",
    ] {
        if std::env::var("SIMREP_GPU_CASE").is_ok_and(|v| v != name) {
            continue;
        }
        let (field, particles, oracle, _) = load(Path::new(&root), name);
        let options = FlightOptions {
            duration_us: if name == "helmholtz" { 0.03 } else { 10000.0 },
            ..Default::default()
        };
        let reference = crate::fly_many(
            &field,
            &particles,
            &FlightOptions {
                max_step_us: 0.0025,
                ..options.clone()
            },
        )
        .unwrap();
        let cpu = crate::fly_many(&field, &particles, &options).unwrap();
        let old_api = crate::fly_many(
            &field,
            &particles,
            &FlightOptions {
                max_step_us: 0.1,
                relative_tolerance: 1e-10,
                position_tolerance_mm: 1e-10,
                velocity_tolerance_mm_per_us: 1e-10,
                ..options.clone()
            },
        )
        .unwrap();
        let actual = gpu
            .prepare_physics(&field.electric, &[], &field.magnetic, None, 813)
            .unwrap()
            .fly(&particles, &options)
            .unwrap();
        if name == "solenoid" {
            for (device, results) in [("cpu", cpu.as_slice()), ("gpu", actual.results.as_slice())] {
                let accepted: usize = results.iter().map(|r| r.accepted_steps).sum();
                let rejected: usize = results.iter().map(|r| r.rejected_steps).sum();
                let maximum = results.iter().map(|r| r.accepted_steps).max().unwrap();
                println!(
                    "GPU_STEP_PROFILE,{name},device={device},ions={},accepted_per_ion={:.6},rejected_per_ion={:.6},maximum_accepted={maximum}",
                    results.len(),
                    accepted as f64 / results.len() as f64,
                    rejected as f64 / results.len() as f64
                );
            }
        }
        let (position, tof) = errors(&actual.results, &reference);
        let (cpu_position, cpu_tof) = errors(&cpu, &reference);
        let (old_position, old_tof) = errors(&old_api, &reference);
        let (simion_position, simion_tof) =
            oracle
                .iter()
                .zip(&reference)
                .fold((0.0_f64, 0.0_f64), |(p, t), (a, b)| {
                    (
                        p.max(
                            (0..3)
                                .map(|k| (a[k + 1] - b.final_state.position_mm[k]).powi(2))
                                .sum::<f64>()
                                .sqrt(),
                        ),
                        t.max((a[0] - b.tof_us).abs()),
                    )
                });
        println!(
            "GPU_REFERENCE,{name},ions={},position_mm={position:.12e},tof_us={tof:.12e},cpu_position_mm={cpu_position:.12e},cpu_tof_us={cpu_tof:.12e},old_position_mm={old_position:.12e},old_tof_us={old_tof:.12e},simion_position_mm={simion_position:.12e},simion_tof_us={simion_tof:.12e}",
            particles.len()
        );
        assert!(position < simion_position && tof < simion_tof);
        assert!(position <= old_position);
        assert!(tof <= old_tof);
    }
}

fn write_csv(path: &Path, particles: &[Particle], results: &[FlightResult]) {
    let mut file = std::io::BufWriter::new(std::fs::File::create(path).unwrap());
    writeln!(file, "ion,tof_us,x_mm,y_mm,z_mm,ke_ev").unwrap();
    for (i, (p, r)) in particles.iter().zip(results).enumerate() {
        let x = r.final_state.position_mm;
        let v = r.final_state.velocity_mm_per_us;
        let energy = p.mass_amu * dot(v, v) / (2.0 * crate::ELECTRIC_ACCELERATION);
        writeln!(
            file,
            "{i},{:.17e},{:.17e},{:.17e},{:.17e},{energy:.17e}",
            r.tof_us, x[0], x[1], x[2]
        )
        .unwrap();
    }
}

#[test]
fn gpu_physics_collision_distributions() {
    let Ok(root) = std::env::var("SIMREP_GPU_PHYSICS") else {
        return;
    };
    let output = Path::new(&root).join("gpu-physics");
    std::fs::create_dir_all(&output).unwrap();
    let preview = std::env::var("SIMREP_GPU_PREVIEW").as_deref() == Ok("1");
    let gpu = pollster::block_on(Gpu::new_with_precision(
        Backends::VULKAN,
        if preview {
            Precision::Preview
        } else {
            Precision::Precise
        },
    ))
    .unwrap()
    .expect("native Vulkan adapter");
    assert!(
        preview || gpu.has_f64(),
        "precise distribution acceptance requires native f64"
    );
    let electric = InstanceField::new(
        GridField::new(PotentialArray::new(1, 1, 1)).unwrap(),
        [0.0; 3],
        [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        1.0,
        None,
        None,
    )
    .unwrap();
    let magnetic = MagneticField::default();
    for name in ["hs1", "sds"] {
        let (model, energy) = if name == "hs1" {
            (CollisionModel::Hs1(Hs1::default()), 0.1)
        } else {
            (
                CollisionModel::Sds(Sds {
                    gas: Gas {
                        pressure_pa: 760.0 * 133.322,
                        temperature_k: 273.15,
                        mass_amu: 28.94515,
                        velocity_mm_per_us: [0.0; 3],
                    },
                    gas_diameter_nm: 0.366,
                    ion_diameter_nm: None,
                    mobility_stp: None,
                    diffusion: true,
                    statistics: DiffusionStatistics::from_path(
                        std::env::var("SIMREP_GPU_MBM").expect("SIMREP_GPU_MBM"),
                    )
                    .unwrap(),
                }),
                0.0,
            )
        };
        let particles =
            vec![
                Particle::from_energy(200.0, 1.0, [0.0; 3], energy, [1.0, 0.0, 0.0], 0.0).unwrap();
                4096
            ];
        let options = FlightOptions {
            duration_us: 50.0,
            initial_step_us: if name == "sds" { 50.0 } else { 0.001 },
            max_step_us: if name == "sds" { 50.0 } else { 0.05 },
            ..Default::default()
        };
        let cpu = fly_many_collisional(&electric, &particles, &options, &model, 813).unwrap();
        let actual = gpu
            .prepare_physics(&electric, &[], &magnetic, Some(&model), 813)
            .unwrap()
            .fly(&particles, &options)
            .unwrap();
        assert!(
            actual
                .results
                .iter()
                .all(|r| r.splat_reason == SplatReason::TimeLimit)
        );
        write_csv(&output.join(format!("{name}-cpu.csv")), &particles, &cpu);
        write_csv(
            &output.join(format!(
                "{name}-gpu{}.csv",
                if preview { "-preview" } else { "" }
            )),
            &particles,
            &actual.results,
        );
        println!(
            "GPU_DISTRIBUTION,{name},ions=4096,compute_s={:.9}",
            actual.timings.fly.as_secs_f64()
        );
    }
}

#[test]
fn gpu_physics_compute_benchmark() {
    let Ok(root) = std::env::var("SIMREP_GPU_PHYSICS") else {
        return;
    };
    if std::env::var("SIMREP_GPU_TIMING").as_deref() != Ok("1") {
        return;
    }
    let cpu_only = std::env::var("SIMREP_GPU_CPU_ONLY").as_deref() == Ok("1");
    let gpu = pollster::block_on(Gpu::new(Backends::VULKAN))
        .unwrap()
        .expect("native Vulkan adapter");
    assert!(
        gpu.has_f64() && gpu.has_timestamps(),
        "compute calibration requires native f64 timestamps"
    );
    assert_ne!(
        gpu.info.device_type,
        DeviceType::Cpu,
        "do not calibrate a software Vulkan adapter"
    );
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(16)
        .build()
        .unwrap();
    for name in ["sector_3dp", "solenoid", "hs1"] {
        if std::env::var("SIMREP_GPU_CASE").is_ok_and(|v| v != name) {
            continue;
        }
        let model = (name == "hs1").then(|| CollisionModel::Hs1(Hs1::default()));
        let (field, source, simion) = if name == "hs1" {
            let electric = InstanceField::new(
                GridField::new(PotentialArray::new(1, 1, 1)).unwrap(),
                [0.0; 3],
                [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
                1.0,
                None,
                None,
            )
            .unwrap();
            (
                Electromagnetic {
                    electric,
                    magnetic: MagneticField::default(),
                },
                vec![
                    Particle::from_energy(200.0, 1.0, [0.0; 3], 0.1, [1.0, 0.0, 0.0], 0.0).unwrap(),
                ],
                9.96 / 4096.0,
            )
        } else {
            let (field, source, _, simion) = load(Path::new(&root), name);
            (field, source, simion)
        };
        let options = FlightOptions {
            duration_us: if name == "hs1" { 50.0 } else { 10000.0 },
            max_step_us: if name == "hs1" { 0.05 } else { f64::MAX },
            ..Default::default()
        };
        let mut prepared = if cpu_only {
            None
        } else {
            Some(
                gpu.prepare_physics(&field.electric, &[], &field.magnetic, model.as_ref(), 813)
                    .unwrap(),
            )
        };
        let mut measured_rate = None;
        for count in std::env::var("SIMREP_GPU_COUNTS")
            .unwrap_or_else(|_| "1000,10000,100000,1000000".into())
            .split(',')
            .map(|v| v.parse::<usize>().unwrap())
        {
            if cpu_only && count > 100_000 {
                continue;
            }
            let particles: Vec<_> = source.iter().cycle().take(count).cloned().collect();
            let mut cpu = Vec::new();
            let mut compute = Vec::new();
            if let Some(prepared) = &mut prepared {
                prepared.fly(&particles, &options).unwrap();
            }
            for repeat in 0..3 {
                if let Some(prepared) = &mut prepared {
                    let report = prepared.fly(&particles, &options).unwrap();
                    assert!(report.results.iter().all(|r| if name == "hs1" {
                        r.splat_reason == SplatReason::TimeLimit
                    } else {
                        matches!(
                            r.splat_reason,
                            SplatReason::OutsideGrid | SplatReason::Electrode { .. }
                        )
                    }));
                    println!(
                        "GPU_TIMING_SAMPLE,{name},ions={count},compute_s={:.9},upload_s={:.9},readback_s={:.9}",
                        report.timings.fly.as_secs_f64(),
                        report.timings.upload.as_secs_f64(),
                        report.timings.readback.as_secs_f64()
                    );
                    compute.push(report.timings.fly.as_secs_f64());
                }
                if count > 100_000 || count == 100_000 && repeat != 0 {
                    continue;
                }
                let start = Instant::now();
                let result = pool
                    .install(|| {
                        if let Some(model) = &model {
                            fly_many_collisional(&field, &particles, &options, model, 813)
                        } else {
                            crate::fly_many(&field, &particles, &options)
                        }
                    })
                    .unwrap();
                let elapsed = start.elapsed().as_secs_f64();
                println!("CPU_TIMING_SAMPLE,{name},ions={count},compute_s={elapsed:.9},workers=16");
                cpu.push(elapsed);
                assert_eq!(result.len(), count);
            }
            cpu.sort_by(f64::total_cmp);
            compute.sort_by(f64::total_cmp);
            if count == 10_000 {
                measured_rate = Some(cpu[1] / count as f64);
            }
            if count == 100_000
                && let Some(rate) = measured_rate
            {
                let estimate = rate * count as f64;
                println!(
                    "CPU_LINEARITY,{name},ions={count},single_actual_s={:.9},extrapolated_from_10k_s={estimate:.9},deviation_percent={:.6}",
                    cpu[0],
                    100.0 * (cpu[0] / estimate - 1.0)
                );
            }
            let (cpu_seconds, cpu_kind) = if count < 100_000 {
                (cpu[1], "actual_median_3")
            } else if let Some(rate) = measured_rate {
                (rate * count as f64, "linear_extrapolation_from_10k")
            } else {
                (
                    cpu.first().copied().unwrap_or(f64::NAN),
                    "actual_single_or_unavailable",
                )
            };
            if cpu_only {
                println!(
                    "CPU_TIMING,{name},ions={count},cpu_s={cpu_seconds:.9},cpu_kind={cpu_kind}"
                );
            } else {
                println!(
                    "GPU_TIMING,{name},ions={count},gpu_median_s={:.9},cpu_s={:.9},cpu_kind={cpu_kind},simion_per_ion_extrapolated_s={:.9}",
                    compute[1],
                    cpu_seconds,
                    simion * count as f64 / source.len() as f64
                );
            }
        }
    }
}
