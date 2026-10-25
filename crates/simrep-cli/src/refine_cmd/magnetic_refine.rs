use std::{
    fs::File,
    io::{BufWriter, Write},
    path::{Path, PathBuf},
    time::Instant,
};

use serde::Deserialize;
use simrep_core::FieldType;
use simrep_solve::magnetostatics::{self, FieldFile, Formulation, MagneticProblem};

use super::*;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CartesianInput {
    cartesian: CartesianSpec,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CartesianSpec {
    potentials: [PathBuf; 3],
    current_density: [PathBuf; 3],
    #[serde(default = "one")]
    permeability: f64,
    solid: Option<PathBuf>,
}

fn one() -> f64 {
    1.0
}

fn refine(
    pa: &mut PaFile,
    layout: &Layout,
    problem: &MagneticProblem,
    options: Options,
    gpu: Option<&Gpu>,
    timings: &mut Timings,
) -> GpuResult<()> {
    for i in 0..pa.array.potentials.len() {
        if antimirror_plane(&pa.array, i) {
            if pa.array.electrodes[i] && pa.array.potentials[i] != 0.0 {
                return Err("nonzero magnetic boundary on an anti-mirror plane".into());
            }
            pa.array.potentials[i] = 0.0;
        }
    }
    let mut values = if layout.cylindrical {
        transpose(&pa.array.potentials, pa.array.nx, pa.array.ny)
    } else {
        std::mem::take(&mut pa.array.potentials)
    };
    let (converged, cycles, residual) = if let Some(gpu) = gpu {
        let report = problem.solve_gpu(gpu, &mut values, options)?;
        timings.setup += report.timings.setup;
        timings.solve += report.timings.upload + report.timings.solve + report.timings.readback;
        (
            report.converged,
            report.iterations,
            *report.residual_history.last().unwrap(),
        )
    } else {
        let report = problem.solve(&mut values, options)?;
        timings.setup += report.timings.setup;
        timings.solve += report.timings.solve;
        (
            report.converged,
            report.iterations,
            *report.residual_history.last().unwrap(),
        )
    };
    pa.array.potentials = if layout.cylindrical {
        transpose(&values, pa.array.ny, pa.array.nx)
    } else {
        values
    };
    println!(
        "  magnetic {:?}: cycles={cycles}, residual absolute={:.9e}, relative={:.9e}",
        problem.formulation, residual.absolute, residual.relative
    );
    if !converged {
        return Err(format!("magnetic refinement did not converge after {cycles} cycles").into());
    }
    Ok(())
}
fn magnetic_pa(path: &Path) -> GpuResult<PaFile> {
    if !path
        .extension()
        .is_some_and(|s| s.eq_ignore_ascii_case("pa"))
    {
        return Err(
            "magnetic material refinement requires ordinary .pa files, not fast-adjust bases"
                .into(),
        );
    }
    let pa = read_pa(path)?;
    if pa.array.field_type != FieldType::Magnetic || !pa.array.refinable || pa.ng == 0 {
        return Err(
            "magnetic refinement requires a refinable magnetic PA with positive header ng".into(),
        );
    }
    Ok(pa)
}

pub(super) fn run(options: RefineOptions) -> GpuResult<()> {
    let total = Instant::now();
    let spec_path = options.magnetic_material.as_ref().unwrap();
    let spec: serde_json::Value = serde_json::from_slice(&std::fs::read(spec_path)?)?;
    let start = Instant::now();
    let mut primary = magnetic_pa(&options.file)?;
    let ng = primary.ng;
    let primary_layout = Layout::new(&primary.array);
    // Preserve the linear policy, extending its measured crossover only for
    // magnetic Poisson. The smaller bracket needs native f64 on the adapter.
    let gpu = if options
        .device
        .requests_gpu(primary_layout.grid, Some(false))
        || options.device == RefineDevice::Auto
            && magnetostatics::prefer_gpu(primary_layout.grid, true)
    {
        let candidate = pollster::block_on(Gpu::new(options.backend.backends()))?;
        let selected = candidate.filter(|g| {
            options.device == RefineDevice::Gpu
                || g.info.device_type != DeviceType::Cpu
                    && magnetostatics::prefer_gpu(primary_layout.grid, g.has_f64())
        });
        if selected.is_none() && options.device == RefineDevice::Gpu {
            return Err("no GPU adapter found for magnetic refinement".into());
        }
        selected
    } else {
        None
    };
    println!(
        "Device: {} (requested {:?}); magnetic material refinement",
        if gpu.is_some() { "GPU" } else { "CPU" },
        options.device
    );
    let solve_options = Options {
        relative_tolerance: options.tolerance,
        max_cycles: options.max_cycles,
        ..Default::default()
    };
    let mut timings = Timings {
        setup: start.elapsed(),
        ..Default::default()
    };
    let parent = spec_path.parent().unwrap_or(Path::new("."));
    let header = if spec.get("cartesian").is_some() {
        let spec: CartesianInput = serde_json::from_value(spec)?;
        let spec = spec.cartesian;
        let paths = spec.potentials.map(|p| parent.join(p));
        if paths[0].canonicalize()? != options.file.canonicalize()? {
            return Err("Cartesian potentials[0] must name the refine input Ax PA".into());
        }
        let mut components = [primary, magnetic_pa(&paths[1])?, magnetic_pa(&paths[2])?];
        let mut solid_cells = Vec::new();
        for (a, pa) in components.iter_mut().enumerate() {
            let layout = Layout::new(&pa.array);
            if layout.grid.geometry != Geometry::Planar
                || layout.grid.dims != primary_layout.grid.dims
                || layout.grid.spacing != primary_layout.grid.spacing
                || pa.ng != ng
            {
                return Err(
                    "Cartesian A components must share planar dimensions, spacing and ng".into(),
                );
            }
            let prepare = Instant::now();
            let current = magnetostatics::load_cells(
                &parent.join(&spec.current_density[a]),
                layout.grid,
                false,
            )?;
            let mut problem = MagneticProblem::cartesian_current_component(
                layout.grid,
                &layout.fixed,
                spec.permeability,
                &current,
                pa.ng as f64,
            )?;
            if let Some(path) = &spec.solid {
                for (solid, value) in
                    problem
                        .solid_cells
                        .iter_mut()
                        .zip(magnetostatics::load_cells(
                            &parent.join(path),
                            layout.grid,
                            false,
                        )?)
                {
                    *solid |= value != 0.0;
                }
            }
            timings.setup += prepare.elapsed();
            refine(
                pa,
                &layout,
                &problem,
                solve_options,
                gpu.as_ref(),
                &mut timings,
            )?;
            solid_cells = problem.solid_cells;
        }
        // Only write after every component has converged.
        for (path, pa) in paths.iter().zip(&components) {
            output(path, pa, &mut timings)?;
        }
        FieldFile {
            version: 1,
            formulation: Formulation::CartesianA,
            potential: paths[0].canonicalize()?,
            ng: components[0].ng as f64,
            solid_cells,
            components: Some([
                paths[0].canonicalize()?,
                paths[1].canonicalize()?,
                paths[2].canonicalize()?,
            ]),
        }
    } else {
        let prepare = Instant::now();
        let problem = magnetostatics::load(
            spec_path,
            primary_layout.grid,
            &primary_layout.fixed,
            primary_layout.cylindrical,
            primary.ng as f64,
        )?;
        timings.setup += prepare.elapsed();
        refine(
            &mut primary,
            &primary_layout,
            &problem,
            solve_options,
            gpu.as_ref(),
            &mut timings,
        )?;
        output(&options.file, &primary, &mut timings)?;
        FieldFile {
            version: 1,
            formulation: problem.formulation,
            potential: options
                .file
                .file_name()
                .ok_or("missing magnetic PA filename")?
                .into(),
            ng: primary.ng as f64,
            solid_cells: if primary_layout.cylindrical {
                let d = primary_layout.grid.dims.map(|n| n.saturating_sub(1).max(1));
                transpose(&problem.solid_cells, d[0], d[1])
            } else {
                problem.solid_cells
            },
            components: None,
        }
    };
    let sidecar = options.file.with_extension("mag.json");
    let mut writer = BufWriter::new(File::create(&sidecar)?);
    serde_json::to_writer(&mut writer, &header)?;
    writer.flush()?;
    println!("Wrote {}", sidecar.display());
    println!(
        "Setup: {:.3} ms, solve: {:.3} ms, IO: {:.3} ms, end-to-end: {:.3} ms",
        ms(timings.setup),
        ms(timings.solve),
        ms(timings.io),
        ms(total.elapsed())
    );
    Ok(())
}
