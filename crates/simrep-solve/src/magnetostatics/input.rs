use std::path::{Path, PathBuf};

use serde::Deserialize;
use simrep_core::Symmetry;

use super::MagneticProblem;
use crate::{
    gpu::GpuResult,
    mg::{Geometry, Grid},
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Spec {
    permeability: Option<PathBuf>,
    magnetization: Option<[Option<PathBuf>; 3]>,
    negative_divergence: Option<PathBuf>,
    current_density: Option<PathBuf>,
    solid: Option<PathBuf>,
}

/// Load cell PAs named by a JSON magnetic-material specification. Paths are
/// relative to that JSON. Magnetization components follow PA coordinates;
/// cylindrical (Mz,Mr,0) storage is converted to solver (Mr,Mz,0).
pub fn load(
    path: impl AsRef<Path>,
    grid: Grid,
    fixed: &[bool],
    cylindrical_storage: bool,
    ng: f64,
) -> GpuResult<MagneticProblem> {
    let path = path.as_ref();
    let spec: Spec = serde_json::from_slice(&std::fs::read(path)?)?;
    if spec.permeability.is_none()
        && spec.magnetization.is_none()
        && spec.negative_divergence.is_none()
        && spec.current_density.is_none()
        && spec.solid.is_none()
    {
        return Err("empty magnetic-material specification; supply at least one of permeability, magnetization, negative_divergence, current_density or solid".into());
    }
    let parent = path.parent().unwrap_or(Path::new("."));
    let cells = grid
        .dims
        .map(|n| n.saturating_sub(1).max(1))
        .iter()
        .product();
    let read = |path: &Path| load_cells(&parent.join(path), grid, cylindrical_storage);
    let mu = spec
        .permeability
        .as_deref()
        .map(read)
        .transpose()?
        .unwrap_or_else(|| vec![1.0; cells]);
    let mut problem = if let Some(current) = spec.current_density {
        if spec.magnetization.is_some() || spec.negative_divergence.is_some() {
            return Err(
                "a magnetic specification must choose currents or scalar magnetization, not both"
                    .into(),
            );
        }
        MagneticProblem::currents(grid, fixed, &mu, &read(&current)?, ng)?
    } else {
        if spec.negative_divergence.is_some()
            && spec.magnetization.is_none()
            && spec.solid.is_none()
        {
            return Err("a precomputed -div M without magnetization requires a solid cell PA to identify the magnet interior".into());
        }
        let mut m = vec![[0.0; 3]; cells];
        if let Some(components) = spec.magnetization {
            for (a, component) in components.iter().enumerate() {
                if let Some(path) = component {
                    let axis = if cylindrical_storage && a < 2 {
                        1 - a
                    } else {
                        a
                    };
                    for (m, value) in m.iter_mut().zip(read(path)?) {
                        m[axis] = value;
                    }
                }
            }
        }
        let divergence = spec.negative_divergence.as_deref().map(read).transpose()?;
        MagneticProblem::scalar(grid, fixed, &mu, &m, divergence.as_deref(), ng)?
    };
    if let Some(path) = spec.solid {
        for (solid, value) in problem.solid_cells.iter_mut().zip(read(&path)?) {
            *solid |= value != 0.0;
        }
    }
    Ok(problem)
}

/// Read finite non-electrode primal-cell data in the solver's storage order.
pub fn load_cells(path: &Path, grid: Grid, cylindrical_storage: bool) -> GpuResult<Vec<f64>> {
    let pa = simrep_io::read_pa(path)?.array;
    let mut dims = [pa.nx, pa.ny, pa.nz];
    let mut spacing = [pa.dx_mm, pa.dy_mm, pa.dz_mm];
    let mut values = pa.potentials;
    if cylindrical_storage {
        dims.swap(0, 1);
        spacing.swap(0, 1);
        values = (0..values.len())
            .map(|i| values[i / pa.ny + pa.nx * (i % pa.ny)])
            .collect();
    }
    if dims != grid.dims.map(|n| n.saturating_sub(1).max(1))
        || spacing != grid.spacing
        || (pa.symmetry == Symmetry::Cylindrical)
            != matches!(grid.geometry, Geometry::Cylindrical { .. })
        || pa.electrodes.iter().any(|&e| e)
        || values.iter().any(|v| !v.is_finite())
    {
        return Err("magnetic cell PA requires matching symmetry/spacing, one fewer point per active dimension, finite values, and no electrode points".into());
    }
    Ok(values)
}
