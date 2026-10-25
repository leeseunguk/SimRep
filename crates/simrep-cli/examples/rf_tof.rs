//! Reproducible RF/TOF validation and resident-context timing.
//! rf_tof ROOT CASE cpu|gpu FACTOR MAX_STEP_US COPIES
use serde::Deserialize;
use simrep_core::{PotentialArray, Symmetry};
use simrep_fly::{
    FlightOptions, GridField, InstanceField, Particle, SplatReason, VoltageExpression,
    WorkbenchField, fly, fly_many, fly_many_from_electrode,
    gpu::{Backends, Gpu, GpuResult},
};
use simrep_io::{read_fast_adjust, read_pa};
use simrep_solve::mg::{Geometry, Grid, Options, solve};
use std::collections::BTreeMap;
use std::fs::File;
use std::io::BufReader;
use std::path::PathBuf;
use std::time::Instant;

#[derive(Deserialize)]
struct Instance {
    pa: String,
    origin: [f64; 3],
    axes: [[f64; 3]; 3],
    scale: f64,
}
#[derive(Deserialize)]
struct Case {
    instances: Vec<Instance>,
    bounds: [f64; 6],
    particles: Vec<[f64; 10]>,
    finals: Vec<[f64; 5]>,
    #[serde(default)]
    traces: Vec<[f64; 8]>,
}

fn trace_output(trace: &[f64; 8], r: &simrep_fly::FlightResult) -> f64 {
    let p = r.final_state.position_mm;
    println!(
        "TRACE_NATIVE,{},{:.15e},{:.15e},{:.15e},{:.15e}",
        trace[0] as usize, trace[1], p[0], p[1], p[2]
    );
    (0..3).map(|a| (p[a] - trace[a + 2]).powi(2)).sum()
}

// Node-defined electrode surfaces, kept at the same physical coordinates.
// A fine node is metal only when all corners of its containing coarse
// rectangle are metal; this preserves zero-thickness electrode sheets.
fn refined(a: &PotentialArray, factor: usize) -> GpuResult<PotentialArray> {
    if factor == 1 {
        return Ok(a.clone());
    }
    if a.nz != 1 {
        return Err("reference builder requires a two-dimensional PA".into());
    }
    let mut fine = PotentialArray::new((a.nx - 1) * factor + 1, (a.ny - 1) * factor + 1, 1);
    fine.symmetry = a.symmetry;
    fine.mirrors = a.mirrors;
    fine.anti_mirrors = a.anti_mirrors;
    fine.dx_mm = a.dx_mm / factor as f64;
    fine.dy_mm = a.dy_mm / factor as f64;
    fine.dz_mm = a.dz_mm;
    for y in 0..fine.ny {
        for x in 0..fine.nx {
            let (xx, yy) = (x / factor, y / factor);
            let x1 = (xx + usize::from(x % factor != 0)).min(a.nx - 1);
            let y1 = (yy + usize::from(y % factor != 0)).min(a.ny - 1);
            let tx = (x % factor) as f64 / factor as f64;
            let ty = (y % factor) as f64 / factor as f64;
            let corners = [
                xx + a.nx * yy,
                x1 + a.nx * yy,
                xx + a.nx * y1,
                x1 + a.nx * y1,
            ];
            let i = x + fine.nx * y;
            fine.electrodes[i] = corners.iter().all(|&j| a.electrodes[j]);
            fine.potentials[i] = (1.0 - ty)
                * ((1.0 - tx) * a.potentials[corners[0]] + tx * a.potentials[corners[1]])
                + ty * ((1.0 - tx) * a.potentials[corners[2]] + tx * a.potentials[corners[3]]);
        }
    }
    let transpose = |values: &[f64], nx: usize, ny: usize| -> Vec<f64> {
        (0..values.len())
            .map(|i| values[i / ny + nx * (i % ny)])
            .collect()
    };
    let (mut values, fixed, grid) = if a.symmetry == Symmetry::Cylindrical {
        (
            transpose(&fine.potentials, fine.nx, fine.ny),
            (0..fine.electrodes.len())
                .map(|i| fine.electrodes[i / fine.ny + fine.nx * (i % fine.ny)])
                .collect(),
            Grid {
                dims: [fine.ny, fine.nx, 1],
                spacing: [fine.dy_mm, fine.dx_mm, fine.dz_mm],
                geometry: Geometry::Cylindrical { radial_origin: 0.0 },
            },
        )
    } else {
        (
            fine.potentials.clone(),
            fine.electrodes.clone(),
            Grid {
                dims: [fine.nx, fine.ny, 1],
                spacing: [fine.dx_mm, fine.dy_mm, fine.dz_mm],
                geometry: Geometry::Planar,
            },
        )
    };
    let report = solve(
        &mut values,
        &fixed,
        grid,
        Options {
            relative_tolerance: 1e-13,
            max_cycles: 400,
            ..Default::default()
        },
    )?;
    if !report.converged {
        return Err("grid reference did not converge".into());
    }
    eprintln!(
        "reference: {}x{} cycles={} residual={:?}",
        fine.nx,
        fine.ny,
        report.iterations,
        report.residual_history.last()
    );
    fine.potentials = if a.symmetry == Symmetry::Cylindrical {
        transpose(&values, fine.ny, fine.nx)
    } else {
        values
    };
    Ok(fine)
}

fn main() -> GpuResult<()> {
    let start = Instant::now();
    let args: Vec<_> = std::env::args().collect();
    if !(7..=9).contains(&args.len()) {
        return Err(
            "usage: rf_tof ROOT CASE cpu|gpu FACTOR MAX_STEP_US COPIES [TOLERANCE] [validate]"
                .into(),
        );
    }
    let folder = PathBuf::from(&args[1]).join(&args[2]);
    let case: Case =
        serde_json::from_reader(BufReader::new(File::open(folder.join("case.json"))?))?;
    let factor: usize = args[4].parse()?;
    let defaults = FlightOptions::default();
    let automatic = args[5] == "default";
    let step: f64 = if automatic {
        defaults.max_step_us
    } else {
        args[5].parse()?
    };
    let copies: usize = args[6].parse()?;
    let tolerance: f64 = args
        .get(7)
        .map(|s| s.parse())
        .transpose()?
        .unwrap_or(defaults.relative_tolerance);
    let validate = args.get(8).is_some_and(|s| s == "validate");
    if factor == 0 || copies == 0 || !step.is_finite() || step <= 0.0 {
        return Err("invalid factor, step or copy count".into());
    }
    let rf = args[2] != "tof";
    let square = args[2] == "rf_square_wave";
    let mut instances = Vec::new();
    for (instance_number, i) in case.instances.iter().enumerate() {
        let array = read_pa(folder.join(&i.pa))?.array;
        eprintln!(
            "input {}: {}x{} {:?} electrodes={}",
            i.pa,
            array.nx,
            array.ny,
            array.symmetry,
            array.electrodes.iter().filter(|&&v| v).count()
        );
        let mut bases = Vec::new();
        let array = if rf {
            let pa = folder.join(format!("{}.pa0", args[2]));
            let background = read_fast_adjust(&pa, &[(1, 0.0), (2, 0.0)])?;
            for n in 1..=2 {
                let zero = read_fast_adjust(&pa, &[(n, 0.0)])?;
                let mut unit = read_fast_adjust(&pa, &[(n, 1.0)])?;
                for (v, z) in unit.potentials.iter_mut().zip(zero.potentials) {
                    *v -= z;
                }
                let sign = if n == 1 { 1.0 } else { -1.0 };
                // The shipped PRG uses cos(2*pi*140000*t/1e6).
                // Up to the first half-cycle its square wave has one edge.
                let source = if square {
                    format!("{}*(1-2*step(1/(4*frequency)))", 80.0 * sign)
                } else {
                    format!("{}*cos(2*pi*frequency*t)", 100.0 * sign)
                };
                bases.push((
                    GridField::new(refined(&unit, factor)?)?,
                    VoltageExpression::parse(
                        &source,
                        &BTreeMap::from([("frequency".into(), 0.14)]),
                    )?,
                ));
            }
            refined(&background, factor)?
        } else {
            refined(&array, factor)?
        };
        let grid = GridField::new(array)?.with_transparent_faces(if !rf && instance_number == 0 {
            [true, false, false, false, false, false]
        } else if !rf && instance_number == 1 {
            [false, true, false, false, false, false]
        } else {
            [false; 6]
        });
        instances.push(
            InstanceField::new(grid, i.origin, i.axes, i.scale, None, None)?
                .with_voltage_expressions(bases)?,
        );
    }
    let b = case.bounds;
    let field = WorkbenchField::new(instances, [[b[0], b[1], b[2]], [b[3], b[4], b[5]]])?;
    let original: Vec<_> = case
        .particles
        .iter()
        .map(|p| Particle::new(p[1], p[2], [p[3], p[4], p[5]], [p[6], p[7], p[8]], p[0]))
        .collect::<Result<_, _>>()?;
    let particles: Vec<_> = (0..copies).flat_map(|_| original.iter().cloned()).collect();
    let options = FlightOptions {
        duration_us: if rf { 0.5 / 0.14 } else { 40.0 },
        max_step_us: step,
        initial_step_us: 0.001_f64.min(step),
        relativistic: true,
        relative_tolerance: tolerance,
        position_tolerance_mm: tolerance,
        velocity_tolerance_mm_per_us: tolerance,
        ..Default::default()
    };
    let (results, compute) = if args[3] == "gpu" {
        let initialization = Instant::now();
        let gpu = pollster::block_on(Gpu::new(Backends::VULKAN))?.ok_or("no Vulkan adapter")?;
        eprintln!("gpu_init_s: {:.9}", initialization.elapsed().as_secs_f64());
        eprintln!("adapter: {} {:?}", gpu.info.name, gpu.info.device_type);
        let init = Instant::now();
        let mut resident = gpu.prepare_workbench(&field)?;
        eprintln!("resident_prepare_s: {:.9}", init.elapsed().as_secs_f64());
        let report = if rf {
            resident.fly(&particles, &options)?
        } else {
            resident.fly_from_electrode(&particles, &options)?
        };
        eprintln!(
            "gpu: setup={:.9} upload={:.9} fly={:.9} readback={:.9}",
            report.timings.setup.as_secs_f64(),
            report.timings.upload.as_secs_f64(),
            report.timings.fly.as_secs_f64(),
            report.timings.readback.as_secs_f64()
        );
        if validate {
            let mut squares = 0.0;
            for trace in &case.traces {
                let p = &original[trace[0] as usize - 1];
                let opts = FlightOptions {
                    duration_us: trace[1],
                    ..options.clone()
                };
                let actual = if rf {
                    resident.fly(std::slice::from_ref(p), &opts)?
                } else {
                    resident.fly_from_electrode(std::slice::from_ref(p), &opts)?
                };
                squares += trace_output(trace, &actual.results[0]);
            }
            eprintln!(
                "trajectory: samples={} oracle_rms_mm={:.12e}",
                case.traces.len(),
                (squares / case.traces.len() as f64).sqrt()
            );
        }
        (report.results, report.timings.fly.as_secs_f64())
    } else {
        let t = Instant::now();
        let results = if rf {
            fly_many(&field, &particles, &options)?
        } else {
            fly_many_from_electrode(&field, &particles, &options)?
        };
        (results, t.elapsed().as_secs_f64())
    };
    let mut repeated_position_error = 0.0;
    let mut repeated_tof_error = 0.0;
    for (i, result) in results.iter().enumerate() {
        let first = &results[i % original.len()];
        repeated_position_error += (0..3)
            .map(|a| (result.final_state.position_mm[a] - first.final_state.position_mm[a]).powi(2))
            .sum::<f64>();
        repeated_tof_error += (result.tof_us - first.tof_us).powi(2);
    }
    eprintln!(
        "replicas: ions={} position_rms_mm={:.12e} tof_rms_us={:.12e} accepted={} rejected={}",
        results.len(),
        (repeated_position_error / results.len() as f64).sqrt(),
        (repeated_tof_error / results.len() as f64).sqrt(),
        results.iter().map(|r| r.accepted_steps).sum::<usize>(),
        results.iter().map(|r| r.rejected_steps).sum::<usize>(),
    );
    if repeated_position_error != 0.0 || repeated_tof_error != 0.0 {
        return Err("identical beam replicas produced different terminal states".into());
    }
    println!(
        "ion,tof_us,x_mm,y_mm,z_mm,vx_mm_us,vy_mm_us,vz_mm_us,oracle_dt_us,oracle_distance_mm,reason"
    );
    let mut squares = 0.0;
    let mut time_squares = 0.0;
    for (n, r) in results.iter().take(original.len()).enumerate() {
        let reference = case.finals[n];
        let pos = r.final_state.position_mm;
        let velocity = r.final_state.velocity_mm_per_us;
        let distance = (0..3)
            .map(|a| (pos[a] - reference[a + 1]).powi(2))
            .sum::<f64>()
            .sqrt();
        let dt = r.tof_us - reference[0];
        squares += distance * distance;
        time_squares += dt * dt;
        println!(
            "{},{:.15e},{:.15e},{:.15e},{:.15e},{:.15e},{:.15e},{:.15e},{:.15e},{:.15e},{:?}",
            n + 1,
            r.tof_us,
            pos[0],
            pos[1],
            pos[2],
            velocity[0],
            velocity[1],
            velocity[2],
            dt,
            distance,
            r.splat_reason
        );
    }
    eprintln!(
        "summary: case={} factor={} step={} ions={} compute_s={:.9} total_s={:.9} oracle_position_rms_mm={:.12e} oracle_tof_rms_us={:.12e}",
        args[2],
        factor,
        step,
        particles.len(),
        compute,
        start.elapsed().as_secs_f64(),
        (squares / original.len() as f64).sqrt(),
        (time_squares / original.len() as f64).sqrt()
    );
    if validate && args[3] == "cpu" {
        let mut squares = 0.0;
        for trace in &case.traces {
            let p = &original[trace[0] as usize - 1];
            let opts = FlightOptions {
                duration_us: trace[1],
                ..options.clone()
            };
            let actual = if rf {
                fly_many(&field, std::slice::from_ref(p), &opts)?
            } else {
                fly_many_from_electrode(&field, std::slice::from_ref(p), &opts)?
            };
            squares += trace_output(trace, &actual[0]);
            if rf {
                let fine = fly(
                    &field,
                    p,
                    &FlightOptions {
                        max_step_us: step / 4.0,
                        ..opts
                    },
                )?;
                let q = fine.final_state.position_mm;
                println!(
                    "TRACE_REFERENCE,{},{:.15e},{:.15e},{:.15e},{:.15e}",
                    trace[0] as usize, trace[1], q[0], q[1], q[2]
                );
            }
        }
        eprintln!(
            "trajectory: samples={} oracle_rms_mm={:.12e}",
            case.traces.len(),
            (squares / case.traces.len() as f64).sqrt()
        );
    }
    if validate && rf && args[3] == "cpu" && copies == 1 {
        // Independent analytic RF field and a finer integration, not SIMION as
        // the truth. The ideal plates are 80 mm apart.
        let analytic = simrep_fly::AnalyticField(|_: [f64; 3], t: f64| simrep_fly::FieldSample {
            electric_v_per_mm: [
                0.0,
                if square {
                    2.0 * 80.0 / 80.0 * if t < 0.25 / 0.14 { 1.0 } else { -1.0 }
                } else {
                    2.0 * 100.0 / 80.0 * (2.0 * std::f64::consts::PI * 0.14 * t).cos()
                },
                0.0,
            ],
            ..Default::default()
        });
        let mut analytic_error = 0.0;
        let mut refined_error = 0.0;
        for (n, (p, r)) in original.iter().zip(&results).enumerate() {
            let fine = fly(
                &field,
                p,
                &FlightOptions {
                    max_step_us: step / 4.0,
                    ..options.clone()
                },
            )?;
            let exact = fly(
                &analytic,
                p,
                &FlightOptions {
                    max_step_us: step / 4.0,
                    ..options.clone()
                },
            )?;
            let q = fine.final_state.position_mm;
            println!(
                "REFERENCE,{},{:.15e},{:.15e},{:.15e},{:.15e}",
                n + 1,
                fine.tof_us,
                q[0],
                q[1],
                q[2]
            );
            analytic_error += (0..3)
                .map(|a| (r.final_state.position_mm[a] - exact.final_state.position_mm[a]).powi(2))
                .sum::<f64>();
            refined_error += (0..3)
                .map(|a| (r.final_state.position_mm[a] - fine.final_state.position_mm[a]).powi(2))
                .sum::<f64>();
        }
        eprintln!(
            "RF reference: analytic_rms_mm={:.12e}, quarter_step_rms_mm={:.12e}",
            (analytic_error / original.len() as f64).sqrt(),
            (refined_error / original.len() as f64).sqrt()
        );
    }
    if results.iter().any(|r| {
        !matches!(
            r.splat_reason,
            SplatReason::Electrode { .. } | SplatReason::OutsideGrid | SplatReason::TimeLimit
        )
    }) {
        return Err("nonphysical numerical termination".into());
    }
    Ok(())
}
