use std::path::Path;

use serde::Deserialize;
use simrep_core::Symmetry;
use simrep_geom::Sdf;

use super::{
    FieldLaw, FieldSampler, NonlinearOptions, NonlinearPermittivity, Permittivity, Region,
};
use crate::{
    gpu::{Gpu, GpuResult},
    mg::{Geometry, Grid, Options, Residual},
};

pub enum LoadedMaterial {
    Linear(Permittivity),
    Nonlinear(NonlinearPermittivity, NonlinearOptions),
}

pub struct MaterialReport {
    pub converged: bool,
    pub iterations: usize,
    pub residual: Residual,
    pub field_change: Option<f64>,
    pub compute: std::time::Duration,
}

impl LoadedMaterial {
    pub fn is_nonlinear(&self) -> bool {
        matches!(self, Self::Nonlinear(..))
    }

    pub fn solve(
        &self,
        potential: &mut [f64],
        options: Options,
        gpu: Option<&Gpu>,
    ) -> GpuResult<MaterialReport> {
        let start = std::time::Instant::now();
        let (converged, iterations, residual, field_change) = match self {
            Self::Nonlinear(material, outer) => {
                let r = material.solve(potential, options, *outer, gpu)?;
                (
                    r.converged,
                    r.iterations,
                    *r.residual_history.last().unwrap(),
                    r.field_history.last().copied(),
                )
            }
            Self::Linear(material) => {
                if let Some(gpu) = gpu {
                    let r = material.solve_gpu(gpu, potential, options)?;
                    (
                        r.converged,
                        r.iterations,
                        *r.residual_history.last().unwrap(),
                        None,
                    )
                } else {
                    let r = material.solve(potential, options)?;
                    (
                        r.converged,
                        r.iterations,
                        *r.residual_history.last().unwrap(),
                        None,
                    )
                }
            }
        };
        Ok(MaterialReport {
            converged,
            iterations,
            residual,
            field_change,
            compute: start.elapsed(),
        })
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NonlinearInput {
    nonlinear: ConstitutiveInput,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ConstitutiveInput {
    cell_permittivity: std::path::PathBuf,
    laws: Vec<LawInput>,
    #[serde(default)]
    sampler: FieldSampler,
    #[serde(default)]
    options: NonlinearOptions,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LawInput {
    epsilon0: f64,
    law: FieldLaw,
}

/// Additive loader: existing PA/SDF inputs stay linear; a JSON `nonlinear`
/// object supplies a cell PA (relative to the JSON file) and constitutive laws.
pub fn load_material(
    path: impl AsRef<Path>,
    grid: Grid,
    fixed: &[bool],
    cylindrical_storage: bool,
) -> GpuResult<LoadedMaterial> {
    let path = path.as_ref();
    if path
        .extension()
        .is_some_and(|s| s.eq_ignore_ascii_case("json"))
    {
        let value: serde_json::Value = serde_json::from_slice(&std::fs::read(path)?)?;
        if value.get("nonlinear").is_some() {
            let input: NonlinearInput = serde_json::from_value(value)?;
            let input = input.nonlinear;
            let cell_path = path
                .parent()
                .unwrap_or(Path::new("."))
                .join(input.cell_permittivity);
            let values = cell_values(&cell_path, grid, cylindrical_storage)?;
            return Ok(LoadedMaterial::Nonlinear(
                NonlinearPermittivity::from_cells(
                    grid,
                    fixed,
                    &values,
                    input
                        .laws
                        .into_iter()
                        .map(|l| (l.epsilon0, l.law))
                        .collect(),
                    input.sampler,
                )?,
                input.options,
            ));
        }
    }
    Ok(LoadedMaterial::Linear(load(
        path,
        grid,
        fixed,
        cylindrical_storage,
    )?))
}

/// Load a SIMION cell-permittivity PA, or a JSON SDF material specification.
/// `cylindrical_storage` transposes SIMION axial-x/radial-y PA data to (r,z).
/// JSON coordinates are always the solver's Cartesian or (r,z,0) coordinates.
pub fn load(
    path: impl AsRef<Path>,
    grid: Grid,
    fixed: &[bool],
    cylindrical_storage: bool,
) -> GpuResult<Permittivity> {
    let path = path.as_ref();
    if path
        .extension()
        .is_some_and(|s| s.eq_ignore_ascii_case("json"))
    {
        let spec: Spec = serde_json::from_slice(&std::fs::read(path)?)?;
        let shapes = spec
            .regions
            .iter()
            .map(|r| r.shape.sdf())
            .collect::<GpuResult<Vec<_>>>()?;
        let regions: Vec<_> = spec
            .regions
            .iter()
            .zip(&shapes)
            .map(|(r, shape)| Region {
                relative_permittivity: r.epsilon,
                shape,
            })
            .collect();
        return Ok(Permittivity::from_regions(
            grid,
            fixed,
            spec.origin_mm,
            spec.background,
            &regions,
        )?);
    }
    Ok(Permittivity::from_cells(
        grid,
        fixed,
        &cell_values(path, grid, cylindrical_storage)?,
    )?)
}

fn cell_values(path: &Path, grid: Grid, cylindrical_storage: bool) -> GpuResult<Vec<f64>> {
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
    let cylindrical = matches!(grid.geometry, Geometry::Cylindrical { .. });
    if dims != grid.dims.map(|n| n.saturating_sub(1).max(1))
        || spacing != grid.spacing
        || (pa.symmetry == Symmetry::Cylindrical) != cylindrical
        || pa.electrodes.iter().any(|&e| e)
    {
        return Err("dielectric PA requires matching symmetry/spacing, one fewer point per active dimension, and no electrode points".into());
    }
    Ok(values)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Spec {
    background: f64,
    #[serde(default)]
    origin_mm: [f64; 3],
    regions: Vec<Material>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Material {
    epsilon: f64,
    shape: Shape,
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Shape {
    Sphere {
        center: [f64; 3],
        radius: f64,
    },
    Cylinder {
        center: [f64; 3],
        radius: f64,
        axis: usize,
    },
    Box {
        center: [f64; 3],
        half_extent: [f64; 3],
    },
    Plane {
        normal: [f64; 3],
        offset: f64,
    },
    Union {
        a: Box<Shape>,
        b: Box<Shape>,
    },
    Intersection {
        a: Box<Shape>,
        b: Box<Shape>,
    },
    Difference {
        a: Box<Shape>,
        b: Box<Shape>,
    },
}

impl Shape {
    fn sdf(&self) -> GpuResult<Sdf> {
        let finite = |v: &[f64]| v.iter().all(|v| v.is_finite());
        Ok(match self {
            Self::Sphere { center, radius } if finite(center) && radius.is_finite() && *radius > 0.0 =>
                Sdf::Sphere { center: *center, radius: *radius },
            Self::Cylinder { center, radius, axis } if finite(center) && radius.is_finite() && *radius > 0.0 && *axis < 3 =>
                Sdf::Cylinder { center: *center, radius: *radius, axis: *axis },
            Self::Box { center, half_extent } if finite(center) && finite(half_extent) && half_extent.iter().all(|v| *v > 0.0) =>
                Sdf::Box { center: *center, half_extent: *half_extent },
            Self::Plane { normal, offset } if finite(normal) && offset.is_finite() && (normal.iter().map(|v| v*v).sum::<f64>() - 1.0).abs() < 1e-10 =>
                Sdf::Plane { normal: *normal, offset: *offset },
            Self::Union { a, b } => a.sdf()?.union(b.sdf()?),
            Self::Intersection { a, b } => a.sdf()?.intersection(b.sdf()?),
            Self::Difference { a, b } => a.sdf()?.difference(b.sdf()?),
            _ => return Err("invalid material SDF: positive dimensions/radii, axis 0..2, unit plane normal required".into()),
        })
    }
}
