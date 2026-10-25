//! Fresh nonlinear solves, including material/hierarchy work; I/O excluded.
//! Usage: nonlinear_dielectric_oracle <dielectric directory> [cpu|vulkan] [tolerance]
use std::{path::Path, time::Instant};

use simrep_solve::{
    dielectric::{FieldLaw, FieldSampler, NonlinearOptions, NonlinearPermittivity},
    gpu::{Backends, Gpu, GpuResult},
    mg::{Geometry, Grid, Options},
};

fn material(grid: Grid, fixed: &[bool], eps: &[f64]) -> GpuResult<NonlinearPermittivity> {
    Ok(NonlinearPermittivity::from_cells(
        grid,
        fixed,
        eps,
        vec![(
            6.0,
            FieldLaw::SaturatingQuadratic {
                lambda: 8e-11,
                minimum: 1.0,
            },
        )],
        FieldSampler::SphereOffset {
            center: [50.0, 0.0, 0.0],
            radius: 10.0,
            offset: 0.5,
        },
    )?)
}

fn main() -> GpuResult<()> {
    let args: Vec<_> = std::env::args().collect();
    let root = Path::new(args.get(1).ok_or("expected dielectric directory")?);
    let tolerance = args
        .get(3)
        .map(|v| v.parse::<f64>())
        .transpose()?
        .unwrap_or(1e-10);
    let gpu = match args.get(2).map(String::as_str).unwrap_or("cpu") {
        "cpu" => None,
        "vulkan" => Some(pollster::block_on(Gpu::new(Backends::VULKAN))?.ok_or("no GPU")?),
        _ => return Err("expected cpu or vulkan".into()),
    };
    if let Some(gpu) = &gpu {
        println!(
            "Adapter: {} {:?} {:?}",
            gpu.info.name, gpu.info.backend, gpu.info.device_type
        );
    }
    let pa = simrep_io::read_pa(root.join("sphere_nonlinear_3dp.pa"))?.array;
    let eps = simrep_io::read_pa(root.join("sphere_nonlinear_3dp-dielectric.pa"))?
        .array
        .potentials;
    let grid = Grid {
        dims: [pa.nx, pa.ny, pa.nz],
        spacing: [pa.dx_mm, pa.dy_mm, pa.dz_mm],
        geometry: Geometry::Planar,
    };
    let initial: Vec<_> = (0..pa.potentials.len())
        .map(|i| if i % pa.nx == 0 { 1e7 } else { 0.0 })
        .collect();
    let options = Options {
        relative_tolerance: tolerance,
        max_cycles: 300,
        ..Options::default()
    };
    let outer = NonlinearOptions {
        field_tolerance: tolerance,
        ..NonlinearOptions::default()
    };
    // Compute an independent tighter CPU fixed point, not a SIMION reference.
    let mut reference = initial.clone();
    let prepared = material(grid, &pa.electrodes, &eps)?;
    let report = prepared.solve(
        &mut reference,
        Options {
            relative_tolerance: 1e-13,
            ..options
        },
        NonlinearOptions {
            field_tolerance: 1e-12,
            ..outer
        },
        None,
    )?;
    assert!(report.converged, "reference: {report:?}");
    let reference2 = reference.clone();
    let report = prepared.solve(
        &mut reference,
        Options {
            relative_tolerance: 2e-14,
            ..options
        },
        NonlinearOptions {
            field_tolerance: 2e-13,
            ..outer
        },
        None,
    )?;
    assert!(report.converged, "reference check: {report:?}");
    let reference_change = reference
        .iter()
        .zip(reference2)
        .map(|(a, b)| (a - b).abs())
        .fold(0.0_f64, f64::max);
    println!(
        "REFERENCE iterations={} residual={:.9e} tightening_max_V={reference_change:.9e}",
        report.iterations,
        report.residual_history.last().unwrap().relative
    );
    let snapshot = root.join("sphere_nonlinear_3dp.simrep-nonlinear.pa");
    if snapshot.exists() {
        let simion = simrep_io::read_pa(snapshot)?.array.potentials;
        let max = simion
            .iter()
            .zip(&reference)
            .map(|(a, b)| (a - b).abs())
            .fold(0.0_f64, f64::max);
        let rms = (simion
            .iter()
            .zip(&reference)
            .map(|(a, b)| (a - b).powi(2))
            .sum::<f64>()
            / reference.len() as f64)
            .sqrt();
        println!("SIMION max_vs_reference_V={max:.9e} rms_V={rms:.9e}");
    } else {
        println!("SIMION snapshot unavailable; run nonlinear_dielectric_oracle.lua");
    }
    let mut times = Vec::new();
    for repetition in 1..=3 {
        let mut values = initial.clone();
        let start = Instant::now();
        let m = material(grid, &pa.electrodes, &eps)?;
        let report = m.solve(&mut values, options, outer, gpu.as_ref())?;
        let compute = start.elapsed().as_secs_f64();
        assert!(report.converged, "{report:?}");
        let max = values
            .iter()
            .zip(&reference)
            .map(|(a, b)| (a - b).abs())
            .fold(0.0_f64, f64::max);
        let center = pa.nx / 2;
        let field = -(values[center + 1] - values[center - 1]) / (2.0 * grid.spacing[0]);
        println!(
            "NONLINEAR_NATIVE repetition={repetition} compute_s={compute:.9} iterations={} cycles={} residual={:.9e} field_change={:.9e} max_vs_reference_V={max:.9e} Ecenter={field:.12}",
            report.iterations,
            report.inner_cycles,
            report.residual_history.last().unwrap().relative,
            report.field_history.last().unwrap()
        );
        times.push(compute);
    }
    times.sort_by(f64::total_cmp);
    println!(
        "NONLINEAR_MEDIAN compute_s={:.9} repetitions=3 tolerance={tolerance:e}",
        times[1]
    );
    Ok(())
}
