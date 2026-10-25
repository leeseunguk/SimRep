//! Opt-in measurements against captured SIMION magnetic examples.
//! SIMREP_PHYSICS_ORACLE points to physics-validation-20261023.
use std::{hint::black_box, path::Path, time::Instant};

use simrep_core::FieldType;
use simrep_fly::{
    Field, FlightOptions, FlightResult, GridField, InstanceField, Integrator, Particle,
    SplatReason, fly, fly_many,
    physics::magnetic::{Coil, Electromagnetic, MagneticField, MagneticPa},
};
use simrep_io::read_pa;

fn records<'a>(text: &'a str, tag: &'a str) -> impl Iterator<Item = Vec<&'a str>> {
    text.lines()
        .filter_map(move |line| line.strip_prefix(tag).map(|row| row.split(',').collect()))
}

#[test]
#[ignore = "requires SIMREP_CPU_MAGNETIC_ORACLE (captured SIMION magnetic fixtures)"]
fn magnetic_cpu_map_and_profile() {
    let root =
        std::env::var("SIMREP_CPU_MAGNETIC_ORACLE").expect("requires SIMREP_CPU_MAGNETIC_ORACLE");
    let gates = [
        ("magnet1", 5.47282e-7, 4.01298e-7),
        ("mag90", 2.54520e-7, 2.31283e-9),
        ("sector_2dp", 1.85622e-7, 7.11352e-9),
        ("sector_2dc", 3.01237e-7, 3.69093e-9),
        ("sector_3dp", 9.77063e-7, 4.95515e-8),
        ("helmholtz", 1.46092e-8, 2.69845e-11),
        ("solenoid", 1.22619e-6, 1.30382e-9),
    ];
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(16)
        .build()
        .unwrap();
    for (name, position_gate, tof_gate) in gates {
        if std::env::var("SIMREP_MAGNETIC_CASE").is_ok_and(|s| s != name) {
            continue;
        }
        let (field, particles, _, simion_s) = load(Path::new(&root), name);
        let options = FlightOptions {
            duration_us: if name == "helmholtz" { 0.03 } else { 10000.0 },
            ..Default::default()
        };
        let reference = pool
            .install(|| {
                fly_many(
                    &field,
                    &particles,
                    &FlightOptions {
                        max_step_us: 0.0025,
                        ..options.clone()
                    },
                )
            })
            .unwrap();
        let repetitions = match name {
            "magnet1" | "mag90" => 100,
            "solenoid" => 1,
            _ => 10,
        };
        let particles: Vec<_> = (0..repetitions)
            .flat_map(|_| particles.iter().cloned())
            .collect();
        let mut exact_times = Vec::new();
        for _ in 0..5 {
            let start = Instant::now();
            black_box(
                pool.install(|| fly_many(&field, &particles, &options))
                    .unwrap(),
            );
            exact_times.push(start.elapsed().as_secs_f64());
        }
        exact_times.sort_by(f64::total_cmp);
        let trajectory = fly(
            &field,
            &particles[0],
            &FlightOptions {
                sample_every: Some(1),
                ..options.clone()
            },
        )
        .unwrap()
        .trajectory
        .unwrap();
        let points: Vec<_> = trajectory
            .into_iter()
            .map(|s| s.position_mm)
            .filter(|&p| field.magnetic.sample(p, 0.0).magnetic_gauss != [0.0; 3])
            .collect();
        let metadata =
            std::fs::read_to_string(Path::new(&root).join(name).join("metadata.log")).unwrap();
        let bounds = numbers(&records(&metadata, "BOUNDS,").next().unwrap());
        let bounds = Some([
            bounds[..3].try_into().unwrap(),
            bounds[3..].try_into().unwrap(),
        ]);
        let prepared = field.magnetic.prepare_coil_map(bounds).unwrap();
        for (region, c) in prepared.calibration().iter().enumerate() {
            println!(
                "CPU_MAP,{name},region={region},nodes={},max_abs_gauss={:.12e},max_relative={:.12e},accepted={}",
                c.nodes, c.max_absolute_gauss, c.max_relative, c.accepted
            );
            assert!(c.accepted, "{name}: calibration rejected reference map");
        }
        let mapped = Electromagnetic {
            electric: field.electric,
            magnetic: prepared,
        };
        let mut times = Vec::new();
        let mut actual = Vec::new();
        for _ in 0..5 {
            let start = Instant::now();
            actual = pool
                .install(|| fly_many(&mapped, &particles, &options))
                .unwrap();
            times.push(start.elapsed().as_secs_f64());
        }
        times.sort_by(f64::total_cmp);
        let (position, tof) = actual
            .chunks(reference.len())
            .map(|chunk| errors(chunk, &reference))
            .fold((0.0_f64, 0.0_f64), |(p, t), (a, b)| (p.max(a), t.max(b)));
        assert!(
            position <= position_gate,
            "{name}: {position} > {position_gate}"
        );
        assert!(tof <= tof_gate, "{name}: {tof} > {tof_gate}");
        println!(
            "CPU_MAGNETIC,{name},ions={},simion_retained_s={simion_s:.9},exact_median_s={:.9},mapped_median_s={:.9},position_mm={position:.12e},tof_us={tof:.12e},accepted={},rejected={}",
            particles.len(),
            exact_times[2],
            times[2],
            actual.iter().map(|r| r.accepted_steps).sum::<usize>(),
            actual.iter().map(|r| r.rejected_steps).sum::<usize>()
        );
        let samples = 100_000;
        for (label, magnetic) in [
            ("exact", &field.magnetic as &dyn Field),
            ("mapped", &mapped.magnetic as &dyn Field),
        ] {
            let start = Instant::now();
            for i in 0..samples {
                black_box(magnetic.sample(black_box(points[i % points.len()]), 0.0));
            }
            println!(
                "CPU_FIELD,{name},{label},sample_ns={:.3}",
                start.elapsed().as_nanos() as f64 / samples as f64
            );
        }
        if name == "solenoid"
            && let Ok(counts) = std::env::var("SIMREP_CPU_MAGNETIC_COUNTS")
        {
            for count in counts.split(',').map(|n| n.parse::<usize>().unwrap()) {
                let population: Vec<_> = (0..count)
                    .map(|i| particles[i % particles.len()].clone())
                    .collect();
                let mut times = Vec::new();
                let mut results = Vec::new();
                for _ in 0..3 {
                    let start = Instant::now();
                    results = pool
                        .install(|| fly_many(&mapped, &population, &options))
                        .unwrap();
                    times.push(start.elapsed().as_secs_f64());
                }
                times.sort_by(f64::total_cmp);
                let (position, tof) = results
                    .chunks(reference.len())
                    .map(|chunk| errors(chunk, &reference[..chunk.len()]))
                    .fold((0.0_f64, 0.0_f64), |(p, t), (a, b)| (p.max(a), t.max(b)));
                assert!(position <= position_gate && tof <= tof_gate);
                println!(
                    "CPU_SOLENOID_THROUGHPUT,ions={count},median_s={:.9},position_mm={position:.12e},tof_us={tof:.12e}",
                    times[1]
                );
            }
        }
    }
}

#[test]
#[ignore = "requires SIMREP_CPU_MAGNETIC_ORACLE (captured SIMION mag90 fixture)"]
fn magnetic_cpu_mag90_matched_precision() {
    let root =
        std::env::var("SIMREP_CPU_MAGNETIC_ORACLE").expect("requires SIMREP_CPU_MAGNETIC_ORACLE");
    let (field, particles, oracle, _) = load(Path::new(&root), "mag90");
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(16)
        .build()
        .unwrap();
    let reference = pool
        .install(|| {
            fly_many(
                &field,
                &particles,
                &FlightOptions {
                    duration_us: 10000.0,
                    max_step_us: 0.0025,
                    ..Default::default()
                },
            )
        })
        .unwrap();
    let (simion_position, simion_tof) =
        oracle
            .iter()
            .zip(&reference)
            .fold((0.0_f64, 0.0_f64), |(p, t), (a, b)| {
                (
                    p.max(distance(
                        a[1..].try_into().unwrap(),
                        b.final_state.position_mm,
                    )),
                    t.max((a[0] - b.tof_us).abs()),
                )
            });
    println!("MAG90_SIMION,position_mm={simion_position:.12e},tof_us={simion_tof:.12e}");
    let particles: Vec<_> = (0..100).flat_map(|_| particles.iter().cloned()).collect();
    let mut matched = f64::INFINITY;
    let mut retained = f64::INFINITY;
    for tolerance in [1e-12, 1e-11, 1e-10, 1e-9, 1e-8, 1e-7, 1e-6, 1e-5, 1e-4] {
        for cap in [f64::MAX, 0.05, 0.1, 0.25, 0.5, 1.0, 2.0] {
            let options = FlightOptions {
                duration_us: 10000.0,
                max_step_us: cap,
                relative_tolerance: tolerance,
                position_tolerance_mm: tolerance,
                velocity_tolerance_mm_per_us: tolerance,
                automatic_tolerance: false,
                ..Default::default()
            };
            let mut samples = Vec::new();
            let mut results = Vec::new();
            for _ in 0..3 {
                let start = Instant::now();
                results = pool
                    .install(|| fly_many(&field, &particles, &options))
                    .unwrap();
                samples.push(start.elapsed().as_secs_f64());
            }
            samples.sort_by(f64::total_cmp);
            let (p, t) = results
                .chunks(reference.len())
                .map(|chunk| errors(chunk, &reference))
                .fold((0.0_f64, 0.0_f64), |(p, t), (a, b)| (p.max(a), t.max(b)));
            let simion_gate = p <= simion_position && t <= simion_tof;
            let retained_gate = p <= 2.54520e-7 && t <= 2.31283e-9;
            if simion_gate {
                matched = matched.min(samples[1]);
            }
            if retained_gate {
                retained = retained.min(samples[1]);
            }
            println!(
                "MAG90_MATCHED,tolerance={tolerance:.3e},cap_us={cap:.3e},median_s={:.9},position_mm={p:.12e},tof_us={t:.12e},simion_gate={simion_gate},retained_gate={retained_gate}",
                samples[1]
            );
        }
    }
    println!("MAG90_BEST,simion_matched_s={matched:.9},retained_gate_s={retained:.9}");
    assert!(retained.is_finite());
}

fn numbers(columns: &[&str]) -> Vec<f64> {
    columns.iter().map(|v| v.parse().unwrap()).collect()
}

fn distance(a: [f64; 3], b: [f64; 3]) -> f64 {
    a.into_iter()
        .zip(b)
        .map(|(a, b)| (a - b).powi(2))
        .sum::<f64>()
        .sqrt()
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
    let bounds = numbers(&records(&metadata, "BOUNDS,").next().unwrap());
    let bounds = Some([
        bounds[..3].try_into().unwrap(),
        bounds[3..].try_into().unwrap(),
    ]);
    let identity = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    let mut electric = InstanceField::new(
        GridField::new(simrep_core::PotentialArray::new(1, 1, 1)).unwrap(),
        [0.0; 3],
        identity,
        1.0,
        bounds,
        None,
    )
    .unwrap();
    let mut magnetic = MagneticField::default();
    for row in records(&metadata, "INSTANCE,") {
        let file = read_pa(folder.join(format!("instance{}.pa", row[0]))).unwrap();
        let v = numbers(&row[2..]);
        let origin = v[..3].try_into().unwrap();
        let axes = std::array::from_fn(|a| std::array::from_fn(|b| v[3 + a * 3 + b]));
        let scale = v[12];
        if file.array.field_type == FieldType::Magnetic {
            let mut pa =
                MagneticPa::placed(file.array, file.ng as f64, origin, axes, scale, bounds)
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
                scale,
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
    let particles = records(&log, "START,")
        .map(|row| {
            let v = numbers(&row[1..]);
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
    let oracle = records(&log, "FINAL,")
        .map(|row| {
            let v = numbers(&row[1..]);
            (row[0].parse::<usize>().unwrap(), [v[0], v[1], v[2], v[3]])
        })
        .collect::<std::collections::BTreeMap<_, _>>()
        .into_values()
        .collect();
    let compute = log
        .lines()
        .find_map(|line| {
            line.split_once(" splats, ")
                .and_then(|(_, s)| s.split_whitespace().next())
                .and_then(|s| s.parse().ok())
        })
        .unwrap();
    (
        Electromagnetic { electric, magnetic },
        particles,
        oracle,
        compute,
    )
}

fn errors(actual: &[FlightResult], reference: &[FlightResult]) -> (f64, f64) {
    actual
        .iter()
        .zip(reference)
        .fold((0.0_f64, 0.0_f64), |(p, t), (a, b)| {
            assert_eq!(a.splat_reason, b.splat_reason);
            (
                p.max(distance(
                    a.final_state.position_mm,
                    b.final_state.position_mm,
                )),
                t.max((a.tof_us - b.tof_us).abs()),
            )
        })
}

#[test]
#[ignore = "requires SIMREP_PHYSICS_ORACLE (captured SIMION magnetic convergence fixtures)"]
fn magnetic_compute_and_convergence() {
    let root = std::env::var("SIMREP_PHYSICS_ORACLE").expect("requires SIMREP_PHYSICS_ORACLE");
    let root = Path::new(&root);
    let cases = [
        "magnet1",
        "mag90",
        "sector_2dp",
        "sector_2dc",
        "sector_3dp",
        "helmholtz",
        "solenoid",
    ];
    for name in cases {
        if std::env::var("SIMREP_MAGNETIC_CASE").is_ok_and(|selected| selected != name) {
            continue;
        }
        let (field, particles, oracle, simion_s) = load(root, name);
        let base = FlightOptions {
            duration_us: if name == "helmholtz" { 0.03 } else { 10000.0 },
            relative_tolerance: 1e-10,
            position_tolerance_mm: 1e-10,
            velocity_tolerance_mm_per_us: 1e-10,
            ..Default::default()
        };
        let reference = fly_many(
            &field,
            &particles,
            &FlightOptions {
                max_step_us: 0.0025,
                relative_tolerance: 1e-12,
                position_tolerance_mm: 1e-12,
                velocity_tolerance_mm_per_us: 1e-12,
                ..base.clone()
            },
        )
        .unwrap();
        if name == "sector_3dp" {
            let finer = fly_many(
                &field,
                &particles,
                &FlightOptions {
                    max_step_us: 0.0003125,
                    max_steps: 4_000_000,
                    relative_tolerance: 1e-12,
                    position_tolerance_mm: 1e-12,
                    velocity_tolerance_mm_per_us: 1e-12,
                    ..base.clone()
                },
            )
            .unwrap();
            let (position, tof) = errors(&reference, &finer);
            assert!(
                position < 1e-7,
                "3dp reference has not converged: {position}"
            );
            assert!(tof < 1e-7, "3dp reference TOF has not converged: {tof}");
            println!(
                "CONVERGENCE,{name},reference_cap_us=0.0025,finer_cap_us=0.0003125,position_mm={position:.12e},tof_us={tof:.12e}"
            );
        }
        assert_eq!(oracle.len(), reference.len());
        let (oracle_position, oracle_tof) =
            oracle
                .iter()
                .zip(&reference)
                .fold((0.0_f64, 0.0_f64), |(p, t), (a, b)| {
                    (
                        p.max(distance(
                            a[1..].try_into().unwrap(),
                            b.final_state.position_mm,
                        )),
                        t.max((a[0] - b.tof_us).abs()),
                    )
                });
        println!(
            "REFERENCE,{name},ions={},simion_s={simion_s:.6},simion_position_mm={oracle_position:.12e},simion_tof_us={oracle_tof:.12e}",
            particles.len()
        );
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(16)
            .build()
            .unwrap();
        let mut old_errors = None;
        for label in ["old_cli", "old_api", "current", "default"] {
            let options = if label == "default" {
                FlightOptions {
                    duration_us: base.duration_us,
                    ..Default::default()
                }
            } else if label == "current" {
                let mut options = FlightOptions {
                    duration_us: base.duration_us,
                    automatic_tolerance: false,
                    ..Default::default()
                };
                options.step_limits.max_grid_cells = 1.0;
                options
            } else {
                let tolerance = if label == "old_cli" { 1e-8 } else { 1e-10 };
                FlightOptions {
                    max_step_us: 0.1,
                    relative_tolerance: tolerance,
                    position_tolerance_mm: tolerance,
                    velocity_tolerance_mm_per_us: tolerance,
                    ..base.clone()
                }
            };
            let mut times = Vec::new();
            let mut results = Vec::new();
            for _ in 0..5 {
                let start = Instant::now();
                results = pool.install(|| fly_many(&field, &particles, &options).unwrap());
                times.push(start.elapsed().as_secs_f64());
            }
            times.sort_by(f64::total_cmp);
            let (position, tof) = errors(&results, &reference);
            if label == "old_api" {
                old_errors = Some((position, tof));
            }
            if label == "default" {
                let (old_position, old_tof) = old_errors.unwrap();
                assert!(
                    position <= old_position,
                    "{name}: default {position} > old {old_position}"
                );
                assert!(tof <= old_tof, "{name}: default {tof} > old {old_tof}");
                assert!(position < oracle_position);
                assert!(tof < oracle_tof);
            }
            println!(
                "DEFAULTS,{name},{label},median_s={:.9},accepted={},rejected={},position_mm={position:.12e},tof_us={tof:.12e}",
                times[2],
                results.iter().map(|r| r.accepted_steps).sum::<usize>(),
                results.iter().map(|r| r.rejected_steps).sum::<usize>()
            );
        }
        if std::env::var_os("SIMREP_DEFAULTS_ONLY").is_some() {
            continue;
        }
        let caps: &[f64] = if name == "sector_3dp" {
            &[0.02, 0.1, 0.2, 0.5, 1.0, 2.0]
        } else {
            &[0.02, 1.0]
        };
        for &cap in caps {
            let options = FlightOptions {
                max_step_us: cap,
                ..base.clone()
            };
            for threads in if name == "sector_3dp" {
                vec![1, 8, 16]
            } else {
                vec![16]
            } {
                let pool = rayon::ThreadPoolBuilder::new()
                    .num_threads(threads)
                    .build()
                    .unwrap();
                let mut times = Vec::new();
                let mut results = Vec::new();
                for _ in 0..5 {
                    let start = Instant::now();
                    results = pool.install(|| fly_many(&field, &particles, &options).unwrap());
                    times.push(start.elapsed().as_secs_f64());
                }
                times.sort_by(f64::total_cmp);
                let (position, tof) = errors(&results, &reference);
                assert!(
                    position <= oracle_position,
                    "{name}: {position} > {oracle_position}"
                );
                assert!(tof <= oracle_tof, "{name}: {tof} > {oracle_tof}");
                println!(
                    "RK45,{name},cap_us={cap},threads={threads},median_s={:.9},accepted={},rejected={},position_mm={position:.12e},tof_us={tof:.12e}",
                    times[2],
                    results.iter().map(|r| r.accepted_steps).sum::<usize>(),
                    results.iter().map(|r| r.rejected_steps).sum::<usize>()
                );
            }
        }
        if name == "sector_3dp" {
            for step in [0.02, 0.1, 0.2, 0.5] {
                let options = FlightOptions {
                    integrator: Integrator::Boris,
                    initial_step_us: step,
                    max_step_us: step,
                    ..base.clone()
                };
                let start = Instant::now();
                let results = fly_many(&field, &particles, &options).unwrap();
                let (position, tof) = errors(&results, &reference);
                println!(
                    "BORIS,{name},step_us={step},compute_s={:.9},accepted={},position_mm={position:.12e},tof_us={tof:.12e}",
                    start.elapsed().as_secs_f64(),
                    results.iter().map(|r| r.accepted_steps).sum::<usize>()
                );
            }
            let trajectory = fly(
                &field,
                &particles[0],
                &FlightOptions {
                    max_step_us: 0.2,
                    sample_every: Some(1),
                    ..base
                },
            )
            .unwrap()
            .trajectory
            .unwrap();
            let points: Vec<_> = trajectory
                .into_iter()
                .map(|s| s.position_mm)
                .filter(|&p| field.magnetic.sample(p, 0.0).magnetic_gauss != [0.0; 3])
                .collect();
            assert!(!points.is_empty());
            let start = Instant::now();
            for i in 0..100_000 {
                black_box(
                    field.magnetic.arrays[0].sample(black_box(points[i % points.len()]), 0.0),
                );
            }
            println!(
                "FIELD,{name},active_sample_ns={:.3},trajectory_points={}",
                start.elapsed().as_nanos() as f64 / 100_000.0,
                points.len()
            );
        }
        assert!(reference.iter().all(|r| matches!(
            r.splat_reason,
            SplatReason::OutsideGrid | SplatReason::Electrode { .. } | SplatReason::TimeLimit
        )));
    }
}
