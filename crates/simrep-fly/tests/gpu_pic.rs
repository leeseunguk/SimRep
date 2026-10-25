use std::time::Duration;

use simrep_fly::{
    AnalyticField, FieldSample, FlightOptions, Particle,
    physics::{ChargeMode, EnsembleOptions, GpuPic, PicMesh, SpaceCharge, fly_ensemble},
};
use simrep_solve::{
    gpu::{Backends, Gpu, GpuResult},
    mg::{Geometry, Grid, Options},
};

fn mesh(n: usize) -> PicMesh {
    let grid = Grid {
        dims: [n; 3],
        spacing: [4.0 / (n - 1) as f64; 3],
        geometry: Geometry::Planar,
    };
    let fixed = (0..n * n * n)
        .map(|i| {
            let c = [i % n, i / n % n, i / (n * n)];
            c.iter().any(|&x| x == 0 || x + 1 == n)
        })
        .collect();
    PicMesh::new(
        grid,
        [-2.0; 3],
        fixed,
        Options {
            relative_tolerance: 1e-11,
            ..Default::default()
        },
    )
    .unwrap()
}
fn particles(n: usize) -> Vec<Particle> {
    let side = (n as f64).cbrt().ceil() as usize;
    (0..n)
        .map(|i| {
            let c = [i % side, i / side % side, i / (side * side)];
            let p = c.map(|c| 1.4 * ((c as f64 + 0.5) / side as f64 - 0.5));
            Particle::new(1.0, 1.0, p, [0.1, 0.03, -0.01], 0.0).unwrap()
        })
        .collect()
}
fn options(dt: f64, duration: f64) -> EnsembleOptions {
    EnsembleOptions {
        flight: FlightOptions {
            initial_step_us: dt,
            max_step_us: dt,
            duration_us: duration,
            ..Default::default()
        },
        charge_mode: ChargeMode::Coulombic(1e-15),
        ..Default::default()
    }
}
fn median(v: &mut [Duration]) -> f64 {
    v.sort_unstable();
    v[v.len() / 2].as_secs_f64()
}

fn main() -> GpuResult<()> {
    let Some(gpu) = pollster::block_on(Gpu::new(Backends::VULKAN))? else {
        println!("SKIP GPU PIC: no Vulkan adapter");
        return Ok(());
    };
    if !gpu.has_f64() {
        println!("SKIP GPU PIC: SHADER_F64 unavailable");
        return Ok(());
    }
    println!(
        "GPU PIC adapter: {} ({:?})",
        gpu.info.name, gpu.info.device_type
    );
    let make_gpu = |n, capacity| {
        GpuPic::new(
            Gpu::from_device(gpu.device().clone(), gpu.queue().clone(), gpu.info.clone()),
            mesh(n),
            capacity,
        )
    };
    let mut p = particles(256);
    let positions: Vec<_> = p.iter().map(|p| p.initial.position_mm).collect();
    let q = ChargeMode::Coulombic(1e-15).charges(&p, &[])?;
    let mut cpu = mesh(17);
    let mut device = make_gpu(17, p.len())?;
    let cpu_e = cpu.field(&positions, &q, &vec![1.0; p.len()])?;
    let gpu_e = device.field(&positions, &q, &vec![1.0; p.len()])?;
    let field_error = cpu_e
        .iter()
        .flatten()
        .zip(gpu_e.iter().flatten())
        .map(|(a, b)| (a - b).abs())
        .fold(0.0, f64::max);
    assert!(
        field_error < 2e-10,
        "GPU CIC/gather field error={field_error:e}"
    );
    let grid = Grid {
        dims: [17, 15, 1],
        spacing: [0.25, 0.25, 1.0],
        geometry: Geometry::Cylindrical { radial_origin: 0.0 },
    };
    let fixed = (0..17 * 15)
        .map(|i| i % 17 == 16 || i / 17 == 0 || i / 17 == 14)
        .collect::<Vec<_>>();
    let cylinder = || {
        PicMesh::new(
            grid,
            [0.0, 0.0, -1.75],
            fixed.clone(),
            Options {
                relative_tolerance: 1e-11,
                ..Default::default()
            },
        )
        .unwrap()
    };
    let mut cpu_cylinder = cylinder();
    let mut gpu_cylinder = GpuPic::new(
        Gpu::from_device(gpu.device().clone(), gpu.queue().clone(), gpu.info.clone()),
        cylinder(),
        p.len(),
    )?;
    let a = cpu_cylinder.field(&positions, &q, &vec![1.0; p.len()])?;
    let b = gpu_cylinder.field(&positions, &q, &vec![1.0; p.len()])?;
    let cylinder_error = a
        .iter()
        .flatten()
        .zip(b.iter().flatten())
        .map(|(a, b)| (a - b).abs())
        .fold(0.0, f64::max);
    assert!(
        cylinder_error < 2e-10,
        "GPU cylindrical CIC field error={cylinder_error:e}"
    );
    for particle in &mut p[128..] {
        particle.initial.time_us = 0.002;
    }
    let field = AnalyticField(|_, _| FieldSample {
        electric_v_per_mm: [0.001, 0.0, 0.0],
        magnetic_gauss: [0.0, 0.0, 20.0],
        ..Default::default()
    });
    let o = options(0.002, 0.006);
    let reference = fly_ensemble(&field, &p, &o, &mut SpaceCharge::Pic(Box::new(mesh(17))))?;
    let actual = fly_ensemble(
        &field,
        &p,
        &o,
        &mut SpaceCharge::GpuPic(Box::new(make_gpu(17, p.len())?)),
    )?;
    let mut endpoint_error = 0.0_f64;
    let mut velocity_error = 0.0_f64;
    for (a, b) in reference.results.iter().zip(&actual.results) {
        assert_eq!(a.splat_reason, b.splat_reason);
        assert_eq!(a.tof_us, b.tof_us);
        for axis in 0..3 {
            endpoint_error = endpoint_error
                .max((a.final_state.position_mm[axis] - b.final_state.position_mm[axis]).abs());
            velocity_error = velocity_error.max(
                (a.final_state.velocity_mm_per_us[axis] - b.final_state.velocity_mm_per_us[axis])
                    .abs(),
            );
        }
    }
    assert!(
        endpoint_error < 1e-10 && velocity_error < 1e-9,
        "GPU push error: position={endpoint_error:e}, velocity={velocity_error:e}"
    );
    println!(
        "PASS GPU PIC N=256 (staggered births): field max={field_error:e}, cylinder max={cylinder_error:e}, endpoint max={endpoint_error:e}, velocity max={velocity_error:e}"
    );
    if std::env::var_os("SIMREP_PIC_BENCH").is_some() {
        let p = particles(100_000);
        let field = AnalyticField(|_, _| FieldSample::default());
        let o = options(0.0002, 0.0006);
        let mut cpu_times = Vec::new();
        let mut gpu_times = Vec::new();
        for trial in 0..3 {
            let cpu = fly_ensemble(&field, &p, &o, &mut SpaceCharge::Pic(Box::new(mesh(33))))?;
            let gpu = fly_ensemble(
                &field,
                &p,
                &o,
                &mut SpaceCharge::GpuPic(Box::new(make_gpu(33, p.len())?)),
            )?;
            cpu_times.push(cpu.compute_time);
            gpu_times.push(gpu.compute_time);
            let error = cpu
                .results
                .iter()
                .zip(&gpu.results)
                .flat_map(|(a, b)| {
                    (0..3).map(move |axis| {
                        (a.final_state.position_mm[axis] - b.final_state.position_mm[axis]).abs()
                    })
                })
                .fold(0.0, f64::max);
            assert!(error < 1e-10, "large GPU PIC agreement error={error:e}");
            println!(
                "PIC_TIMING trial={trial} N={} steps={} CPU={:.9} GPU={:.9} endpoint_max={error:e}",
                p.len(),
                gpu.global_steps,
                cpu.compute_time.as_secs_f64(),
                gpu.compute_time.as_secs_f64()
            );
        }
        let cpu = median(&mut cpu_times);
        let gpu = median(&mut gpu_times);
        println!(
            "PIC_MEDIAN N={} repeats=3 CPU={cpu:.9} GPU={gpu:.9} CPU_over_GPU={:.4}",
            p.len(),
            cpu / gpu
        );
    }
    Ok(())
}
