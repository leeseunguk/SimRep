//! Opt-in, independently re-solved quadrupole grid/time convergence study.

use super::*;
use simrep_core::PotentialArray;
use simrep_solve::mg::{Geometry, Grid, Options, solve};
use std::fmt::Write;

fn refined(coarse: &PotentialArray, factor: usize) -> PotentialArray {
    let dims = [coarse.nx, coarse.ny, coarse.nz];
    let size = dims.map(|n| (n - 1) * factor + 1);
    let mut fine = PotentialArray::new(size[0], size[1], size[2]);
    fine.dx_mm = coarse.dx_mm / factor as f64;
    fine.dy_mm = coarse.dy_mm / factor as f64;
    fine.dz_mm = coarse.dz_mm / factor as f64;
    fine.mirrors = coarse.mirrors;
    fine.anti_mirrors = coarse.anti_mirrors;
    fine.potentials
        .par_iter_mut()
        .zip(fine.electrodes.par_iter_mut())
        .enumerate()
        .for_each(|(i, (potential, electrode))| {
            let q = [i % size[0], i / size[0] % size[1], i / (size[0] * size[1])]
                .map(|v| v as f64 / factor as f64);
            let low: [usize; 3] = std::array::from_fn(|a| (q[a].floor() as usize).min(dims[a] - 2));
            let t: [f64; 3] = std::array::from_fn(|a| q[a] - low[a] as f64);
            *electrode = true;
            for corner in 0..8 {
                let d = [corner & 1, (corner >> 1) & 1, (corner >> 2) & 1];
                let weight = (0..3)
                    .map(|a| if d[a] == 0 { 1.0 - t[a] } else { t[a] })
                    .product::<f64>();
                if weight == 0.0 {
                    continue;
                }
                let j = ((low[2] + d[2]) * dims[1] + low[1] + d[1]) * dims[0] + low[0] + d[0];
                *potential += weight * coarse.potentials[j];
                *electrode &= coarse.electrodes[j];
            }
        });
    let report = solve(
        &mut fine.potentials,
        &fine.electrodes,
        Grid {
            dims: size,
            spacing: [fine.dx_mm, fine.dy_mm, fine.dz_mm],
            geometry: Geometry::Planar,
        },
        Options {
            relative_tolerance: 1e-12,
            max_cycles: 200,
            ..Default::default()
        },
    )
    .unwrap();
    assert!(report.converged);
    eprintln!(
        "QUAD_REFERENCE factor={factor} solve={:?} cycles={} residual={:?}",
        report.elapsed,
        report.iterations,
        report.residual_history.last()
    );
    fine
}

fn write_results(path: &std::path::Path, results: &[FlightResult]) {
    let mut csv = String::from("ion,tof_us,x_mm,y_mm,z_mm\n");
    for (i, r) in results.iter().enumerate() {
        let [x, y, z] = r.final_state.position_mm;
        writeln!(
            csv,
            "{},{:.17e},{x:.17e},{y:.17e},{z:.17e}",
            i + 1,
            r.tof_us
        )
        .unwrap();
        assert!(matches!(
            r.splat_reason,
            SplatReason::OutsideGrid | SplatReason::Electrode { .. }
        ));
    }
    std::fs::write(path, csv).unwrap();
}

fn placed(array: PotentialArray, compact: bool) -> InstanceField {
    InstanceField::new(
        if compact {
            GridField::new_compact(array)
        } else {
            GridField::new(array)
        }
        .unwrap(),
        [0.0; 3],
        [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        1.0,
        None,
        None,
    )
    .unwrap()
}

#[test]
#[ignore = "requires SIMREP_GPU_QUAD_REFERENCE (quadrupole reference fixtures and hardware Vulkan)"]
fn quadrupole_grid_time_reference() {
    let directory =
        std::env::var("SIMREP_GPU_QUAD_REFERENCE").expect("requires SIMREP_GPU_QUAD_REFERENCE");
    let directory = std::path::Path::new(&directory);
    let array = simrep_io::read_fast_adjust(
        directory.join("quadrupole.pa0"),
        &[(1, 2.5), (2, -2.5), (3, 0.0)],
    )
    .unwrap();
    let particles: Vec<_> = simrep_io::read_particles(directory.join("particles.ion"))
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
        .collect();
    let options = FlightOptions {
        duration_us: 100.0,
        max_step_us: 0.1,
        relative_tolerance: 1e-8,
        position_tolerance_mm: 1e-8,
        velocity_tolerance_mm_per_us: 1e-8,
        relativistic: true,
        ..Default::default()
    };
    let field = placed(array.clone(), false);
    let gpu = pollster::block_on(Gpu::new(Backends::VULKAN))
        .unwrap()
        .unwrap();
    assert_ne!(gpu.info.device_type, DeviceType::Cpu);
    let mut resident = gpu.prepare(&field, &[]).unwrap();
    for count in [100, particles.len()] {
        let start = Instant::now();
        let cpu = crate::fly_many(&field, &particles[..count], &options).unwrap();
        let cpu_compute = start.elapsed();
        let actual = resident.fly(&particles[..count], &options).unwrap();
        eprintln!(
            "QUAD_REFERENCE ions={count} cpu={cpu_compute:?} gpu={:?}",
            actual.timings.fly
        );
        write_results(&directory.join(format!("cpu-{count}.csv")), &cpu);
        write_results(&directory.join(format!("gpu-{count}.csv")), &actual.results);
    }
    for factor in [2, 4] {
        let fine = placed(refined(&array, factor), true);
        for step in if factor == 4 {
            vec![0.05, 0.025]
        } else {
            vec![0.05]
        } {
            let options = FlightOptions {
                max_step_us: step,
                relative_tolerance: 1e-12,
                position_tolerance_mm: 1e-12,
                velocity_tolerance_mm_per_us: 1e-12,
                ..options.clone()
            };
            let start = Instant::now();
            let results = crate::fly_many(&fine, &particles, &options).unwrap();
            eprintln!(
                "QUAD_REFERENCE factor={factor} step={step} fly={:?}",
                start.elapsed()
            );
            write_results(
                &directory.join(format!("reference-{factor}-{step}.csv")),
                &results,
            );
        }
    }
}

#[test]
#[ignore = "requires SIMREP_GPU_QUAD_RULES (quadrupole collision fixtures and f64 Vulkan)"]
fn quadrupole_collision_rule_agreement() {
    let directory = std::env::var("SIMREP_GPU_QUAD_RULES").expect("requires SIMREP_GPU_QUAD_RULES");
    let directory = std::path::Path::new(&directory);
    let array = simrep_io::read_fast_adjust(
        directory.join("quadrupole.pa0"),
        &[(1, 2.5), (2, -2.5), (3, 0.0)],
    )
    .unwrap();
    let particles: Vec<_> = simrep_io::read_particles(directory.join("particles.ion"))
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
        .collect();
    let gpu = pollster::block_on(Gpu::new(Backends::VULKAN))
        .unwrap()
        .unwrap();
    assert!(gpu.has_f64());
    let options = FlightOptions {
        duration_us: 100.0,
        max_step_us: 0.1,
        relative_tolerance: 1e-8,
        position_tolerance_mm: 1e-8,
        velocity_tolerance_mm_per_us: 1e-8,
        relativistic: true,
        ..Default::default()
    };
    for node_cell in [false, true] {
        let mut grid = GridField::new_compact(array.clone()).unwrap();
        if node_cell {
            grid = grid.with_node_cell_collisions();
        } else {
            grid = grid.with_nearest_node_collisions();
        }
        assert_eq!(grid.uses_node_cell_collisions(), node_cell);
        let field = InstanceField::new(
            grid,
            [0.0; 3],
            [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            1.0,
            None,
            None,
        )
        .unwrap();
        let cpu = crate::fly_many(&field, &particles, &options).unwrap();
        let actual = gpu
            .prepare(&field, &[])
            .unwrap()
            .fly(&particles, &options)
            .unwrap();
        let mut position_sum = 0.0;
        let mut position_max: f64 = 0.0;
        let mut time_sum = 0.0;
        let mut time_max: f64 = 0.0;
        let mut mismatches = 0;
        for (a, b) in actual.results.iter().zip(&cpu) {
            let error = (0..3)
                .map(|axis| {
                    (a.final_state.position_mm[axis] - b.final_state.position_mm[axis]).powi(2)
                })
                .sum::<f64>();
            position_sum += error;
            position_max = position_max.max(error.sqrt());
            let time = (a.tof_us - b.tof_us).abs();
            time_sum += time * time;
            time_max = time_max.max(time);
            mismatches += usize::from(a.splat_reason != b.splat_reason);
        }
        let position_rms = (position_sum / particles.len() as f64).sqrt();
        let time_rms = (time_sum / particles.len() as f64).sqrt();
        let name = if node_cell {
            "node-cell"
        } else {
            "nearest-node"
        };
        let summary = format!(
            "QUAD_RULES {name} ions={} position_rms_mm={position_rms:.12e} position_max_mm={position_max:.12e} tof_rms_us={time_rms:.12e} tof_max_us={time_max:.12e} reason_mismatches={mismatches}\n",
            particles.len()
        );
        eprint!("{summary}");
        std::fs::write(
            directory.join(format!("rule-{name}-agreement.txt")),
            summary,
        )
        .unwrap();
        write_results(&directory.join(format!("rule-{name}-cpu.csv")), &cpu);
        write_results(
            &directory.join(format!("rule-{name}-gpu.csv")),
            &actual.results,
        );
        assert_eq!(mismatches, 0);
        assert!(position_rms < 1e-6 && time_rms < 1e-6);
    }
}
