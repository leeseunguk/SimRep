use numpy::PyReadonlyArray3;
use pyo3::prelude::*;
use serde::Deserialize;
use simrep_core::{PotentialArray, Symmetry};
use simrep_solve::{
    gpu::{Backends, Gpu},
    mg::{Geometry, Grid, Options, solve},
};

use crate::pa::{self, Arrays, Result, error};

#[derive(Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    pub device: String,
    pub backend: String,
    pub tolerance: f64,
    pub max_cycles: usize,
    pub permittivity: Option<String>,
    pub adjustable: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            device: "auto".into(),
            backend: "vulkan".into(),
            tolerance: 1e-10,
            max_cycles: 100,
            permittivity: None,
            adjustable: false,
        }
    }
}

pub fn backends(name: &str) -> Result<Backends> {
    match name {
        "auto" => Ok(Backends::DX12 | Backends::VULKAN),
        "vulkan" => Ok(Backends::VULKAN),
        "dx12" => Ok(Backends::DX12),
        "metal" => Ok(Backends::METAL),
        "all" => Ok(Backends::all()),
        _ => Err("backend must be auto, vulkan, dx12, metal or all".into()),
    }
}

pub fn gpu(s: &Settings) -> Result<Option<Gpu>> {
    let backend = backends(&s.backend)?;
    match s.device.as_str() {
        "cpu" => Ok(None),
        "auto" | "gpu" => {
            let gpu = pollster::block_on(Gpu::new(backend))?;
            if gpu.is_none() && s.device == "gpu" {
                return Err(
                    "no GPU adapter found; select device='cpu' or a supported backend".into(),
                );
            }
            if let Some(gpu) = &gpu {
                if s.device == "gpu" {
                    crate::require_hardware_gpu(&gpu.info)?;
                }
                eprintln!(
                    "Adapter: {} ({:?}, {:?})",
                    gpu.info.name, gpu.info.backend, gpu.info.device_type
                );
                crate::report_gpu_residual(gpu.has_f64());
            }
            Ok(gpu)
        }
        _ => Err("device must be auto, cpu or gpu".into()),
    }
}

pub fn options(s: &Settings) -> Options {
    Options {
        relative_tolerance: s.tolerance,
        max_cycles: s.max_cycles,
        ..Options::default()
    }
}

pub fn transpose<T: Copy>(v: &[T], nx: usize, ny: usize) -> Vec<T> {
    (0..v.len()).map(|i| v[i / ny + nx * (i % ny)]).collect()
}

pub fn grid(a: &PotentialArray) -> Grid {
    if a.symmetry == Symmetry::Cylindrical {
        Grid {
            dims: [a.ny, a.nx, 1],
            spacing: [a.dy_mm, a.dx_mm, a.dz_mm],
            geometry: Geometry::Cylindrical { radial_origin: 0.0 },
        }
    } else {
        Grid {
            dims: [a.nx, a.ny, a.nz],
            spacing: [a.dx_mm, a.dy_mm, a.dz_mm],
            geometry: Geometry::Planar,
        }
    }
}

#[pyfunction]
pub fn refine<'py>(
    py: Python<'py>,
    potentials: PyReadonlyArray3<'py, f64>,
    electrodes: PyReadonlyArray3<'py, bool>,
    metadata: &str,
    settings: &str,
) -> PyResult<(Arrays<'py>, String)> {
    let mut file = pa::owned(potentials, electrodes, metadata)?;
    let s: Settings = serde_json::from_str(settings).map_err(error)?;
    let report = py.detach(|| -> Result<String> {
        backends(&s.backend)?;
        let a = &mut file.array;
        if !a.refinable {
            return Err("PA is marked non-refinable (field map)".into());
        }
        let mut fixed = a.electrodes.clone();
        for (i, fixed) in fixed.iter_mut().enumerate() {
            if (a.anti_mirrors.x && i.is_multiple_of(a.nx))
                || (a.anti_mirrors.y && (i / a.nx).is_multiple_of(a.ny))
                || (a.anti_mirrors.z && i / (a.nx * a.ny) == 0)
            {
                if a.electrodes[i] && a.potentials[i] != 0.0 {
                    return Err("nonzero electrode on an anti-mirror plane".into());
                }
                *fixed = true;
                a.potentials[i] = 0.0;
            }
        }
        let cylindrical = a.symmetry == Symmetry::Cylindrical;
        let mut values = if cylindrical { transpose(&a.potentials, a.nx, a.ny) } else { std::mem::take(&mut a.potentials) };
        if cylindrical { fixed = transpose(&fixed, a.nx, a.ny); }
        let material = s.permittivity.as_ref().map(|path| {
            simrep_solve::dielectric::load_material(path, grid(a), &fixed, cylindrical)
        }).transpose()?;
        if s.adjustable && material.as_ref().is_some_and(|m| m.is_nonlinear()) {
            return Err("nonlinear permittivity cannot use fast-adjust bases; refine at actual voltages".into());
        }
        let gpu = if s.device == "auto"
            && !material.as_ref().is_some_and(|material|
                simrep_solve::dielectric::prefer_gpu(grid(a), material.is_nonlinear())) {
            None
        } else {
            gpu(&s)?
        };
        let gpu = gpu.filter(|gpu| s.device != "auto" || material.is_none()
            || gpu.info.device_type != simrep_solve::gpu::DeviceType::Cpu);
        let mut field_change = None;
        if s.device == "auto" {
            let reason = match &material {
                None => "no dielectric material; use --device gpu for GPU refine",
                Some(material) if !simrep_solve::dielectric::prefer_gpu(grid(a), material.is_nonlinear()) =>
                    "dielectric grid below measured GPU crossover",
                Some(_) if gpu.is_none() => "no hardware GPU adapter available for the requested backend",
                Some(_) => "dielectric grid meets measured GPU crossover",
            };
            eprintln!("auto: {} ({reason})", if gpu.is_some() { "GPU" } else { "CPU" });
        }
        let (converged, cycles, residual, compute_ms) = if let Some(material) = &material {
            let r = material.solve(&mut values, options(&s), gpu.as_ref())?;
            field_change = r.field_change;
            (r.converged, r.iterations, r.residual.relative, r.compute.as_secs_f64() * 1000.0)
        } else if let Some(gpu) = &gpu {
            let r = gpu.solve(&mut values, &fixed, grid(a), options(&s))?;
            (r.converged, r.iterations, r.residual_history.last().unwrap().relative, r.timings.solve.as_secs_f64() * 1000.0)
        } else {
            let r = solve(&mut values, &fixed, grid(a), options(&s))?;
            (r.converged, r.iterations, r.residual_history.last().unwrap().relative, r.timings.solve.as_secs_f64() * 1000.0)
        };
        if !converged { return Err(format!("refinement did not converge after {cycles} cycles (relative residual {residual:e}); increase max_cycles").into()); }
        a.potentials = if cylindrical { transpose(&values, a.ny, a.nx) } else { values };
        Ok(serde_json::json!({"cycles":cycles,"relative_residual":residual,"relative_field_change":field_change,"compute_ms":compute_ms,"device":if gpu.is_some() {"gpu"} else {"cpu"}}).to_string())
    }).map_err(error)?;
    Ok((pa::arrays(py, file)?, report))
}
