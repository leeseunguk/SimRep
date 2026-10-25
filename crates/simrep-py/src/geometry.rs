use std::{collections::BTreeMap, path::Path, sync::Arc, time::Instant};

use pyo3::prelude::*;
use serde::Deserialize;
use simrep_core::{FieldType, PotentialArray, Symmetry};
use simrep_geom::{Electrode, SampledSdf, Sdf, SignedDistance};
use simrep_io::{Pa0Metadata, PaFile};
use simrep_mesh::{ImportOptions, Solid, read_mesh, repair};
use simrep_solve::cutcell::CutCells;

use crate::{
    pa::{self, Arrays, Result, error},
    refine::{self, Settings, transpose},
};

type Source = (String, Arc<dyn SignedDistance>);
type GeometryArrays<'py> = (Arrays<'py>, Vec<(usize, Arrays<'py>)>, String);

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ShapeInput {
    part: String,
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
    fn sdf(self) -> Result<Sdf> {
        let finite = |v: &[f64]| v.iter().all(|v| v.is_finite());
        Ok(match self {
            Self::Sphere { center, radius } if finite(&center) && radius.is_finite() && radius > 0.0 => Sdf::Sphere { center, radius },
            Self::Cylinder { center, radius, axis } if finite(&center) && radius.is_finite() && radius > 0.0 && axis < 3 => Sdf::Cylinder { center, radius, axis },
            Self::Box { center, half_extent } if finite(&center) && finite(&half_extent) && half_extent.iter().all(|v| *v > 0.0) => Sdf::Box { center, half_extent },
            Self::Plane { normal, offset } if finite(&normal) && offset.is_finite() && (normal.iter().map(|v| v * v).sum::<f64>() - 1.0).abs() < 1e-10 => Sdf::Plane { normal, offset },
            Self::Union { a, b } => a.sdf()?.union(b.sdf()?),
            Self::Intersection { a, b } => a.sdf()?.intersection(b.sdf()?),
            Self::Difference { a, b } => a.sdf()?.difference(b.sdf()?),
            _ => return Err("invalid analytic SDF: dimensions/radii must be positive, axis 0..2 and plane normal unit length".into()),
        })
    }
}

#[derive(Deserialize)]
#[serde(default, deny_unknown_fields)]
struct GeometrySettings {
    solve: Settings,
    dims: Option<[usize; 3]>,
    spacing_mm: Option<[f64; 3]>,
    origin_mm: [f64; 3],
    symmetry: String,
    units: Option<String>,
    repair: bool,
}

impl Default for GeometrySettings {
    fn default() -> Self {
        Self {
            solve: Settings::default(),
            dims: None,
            spacing_mm: None,
            origin_mm: [0.0; 3],
            symmetry: "planar".into(),
            units: None,
            repair: false,
        }
    }
}

fn validate(d: [usize; 3], h: [f64; 3], o: [f64; 3]) -> Result<()> {
    if d[0] < 2
        || d[1] < 2
        || d[2] == 0
        || d.into_iter().try_fold(1usize, usize::checked_mul).is_none()
        || h.iter().any(|v| !v.is_finite() || *v <= 0.0)
        || o.iter().any(|v| !v.is_finite())
    {
        return Err("geometry needs nx,ny>=2, nz>=1, positive spacing and finite origin".into());
    }
    Ok(())
}

fn point(i: usize, dims: [usize; 3], h: [f64; 3], o: [f64; 3]) -> [f64; 3] {
    let c = [i % dims[0], i / dims[0] % dims[1], i / (dims[0] * dims[1])];
    std::array::from_fn(|a| o[a] + c[a] as f64 * h[a])
}

#[pyfunction]
pub fn refine_geometry<'py>(
    py: Python<'py>,
    path: String,
    shapes: &str,
    assignments: Vec<(String, usize, f64)>,
    settings: &str,
) -> PyResult<GeometryArrays<'py>> {
    let s: GeometrySettings = serde_json::from_str(settings).map_err(error)?;
    let shapes: Vec<ShapeInput> = serde_json::from_str(shapes).map_err(error)?;
    let (file, bases, report) = py.detach(|| -> Result<_> {
        let start = Instant::now();
        if !matches!(s.symmetry.as_str(), "planar" | "cylindrical") {
            return Err("symmetry must be planar or cylindrical with nz=1".into());
        }
        if path.is_empty() && (s.units.is_some() || s.repair) {
            return Err("units and repair apply only to mesh input, not analytic shapes".into());
        }
        let gem_input = Path::new(&path).extension().is_some_and(|v| v.eq_ignore_ascii_case("gem"));
        if gem_input && s.symmetry != "planar" {
            return Err("GEM pa_define determines symmetry; omit the Python symmetry override (accepted only for mesh/analytic input: planar or cylindrical with nz=1)".into());
        }
        let mesh_input = !path.is_empty() && !gem_input;
        let (mut file, sources, tracing_scale): (PaFile, Vec<Source>, f64) = if gem_input {
            if s.units.is_some() || s.repair { return Err("units and repair apply only to mesh input".into()); }
            let gem = simrep_gem::read_gem(&path)?;
            if !gem.refinable || gem.field_type != FieldType::Electrostatic {
                return Err("refine_geometry needs refinable electrostatic GEM geometry".into());
            }
            let dims = s.dims.unwrap_or(gem.dimensions);
            let h = s.spacing_mm.unwrap_or(gem.spacing_mm);
            validate(dims, h, s.origin_mm)?;
            let sampled_dims = dims.map(|n| 4 * (n - 1) + 1);
            let sources = gem.sample_electrode_sdfs(sampled_dims, s.origin_mm, h.map(|h| h / 4.0))?
                .into_iter().map(|sample| {
                    Ok((sample.voltage.to_string(), Arc::new(SampledSdf::new(sample.dims, sample.origin, sample.spacing, sample.values)?) as Arc<dyn SignedDistance>))
                }).collect::<Result<Vec<_>>>()?;
            let mut a = PotentialArray::new(dims[0], dims[1], dims[2]);
            a.symmetry = gem.symmetry;
            a.mirrors = gem.mirrors;
            [a.dx_mm, a.dy_mm, a.dz_mm] = h;
            let mut file = PaFile::new(a);
            file.ng = gem.ng;
            (file, sources, (dims.iter().filter(|&&n| n > 1).count() as f64).sqrt())
        } else {
            let dims = s.dims.ok_or("mesh and analytic SDF inputs require dims=(nx,ny,nz)")?;
            let h = s.spacing_mm.unwrap_or([1.0; 3]);
            validate(dims, h, s.origin_mm)?;
            let sources = if path.is_empty() {
                shapes.into_iter().map(|v| Ok((v.part, Arc::new(v.shape.sdf()?) as Arc<dyn SignedDistance>))).collect::<Result<Vec<_>>>()?
            } else {
                if !shapes.is_empty() { return Err("provide a geometry path or analytic shapes, not both".into()); }
                if dims[2] < 2 || s.symmetry != "planar" { return Err("mesh input requires a planar 3D grid".into()); }
                let mut mesh = read_mesh(&path, ImportOptions { units: s.units.as_deref().map(str::parse).transpose()?, ..Default::default() })?;
                let mut sources = Vec::new();
                for (i, part) in mesh.parts.iter_mut().enumerate() {
                    if s.repair { repair(part)?; }
                    let solid = Solid::new(part)?;
                    let name = if part.name.is_empty() { i.to_string() } else { part.name.clone() };
                    sources.push((name, Arc::new(move |p| solid.signed_distance(p)) as Arc<dyn SignedDistance>));
                }
                sources
            };
            let mut a = PotentialArray::new(dims[0], dims[1], dims[2]);
            [a.dx_mm, a.dy_mm, a.dz_mm] = h;
            a.symmetry = match s.symmetry.as_str() {
                "planar" => Symmetry::Planar,
                "cylindrical" if dims[2] == 1 => Symmetry::Cylindrical,
                _ => return Err("symmetry must be planar or cylindrical with nz=1".into()),
            };
            a.mirrors.y = a.symmetry == Symmetry::Cylindrical;
            (PaFile::new(a), sources, 1.0)
        };
        if sources.is_empty() { return Err("geometry has no electrode parts".into()); }
        let dims = [file.array.nx, file.array.ny, file.array.nz];
        let h = [file.array.dx_mm, file.array.dy_mm, file.array.dz_mm];
        let cylindrical = file.array.symmetry == Symmetry::Cylindrical;
        if cylindrical && (s.origin_mm[1] != 0.0 || s.origin_mm[2] != 0.0) {
            return Err("cylindrical radial and z origins must be zero".into());
        }
        let mut used = vec![false; assignments.len()];
        let mut parts = Vec::new();
        let mut voltages = BTreeMap::new();
        for (i, (name, shape)) in sources.into_iter().enumerate() {
            let matches: Vec<_> = assignments.iter().enumerate().filter(|(_, (p, _, _))| *p == name || (mesh_input && *p == i.to_string())).collect();
            if matches.len() != 1 { return Err(format!("part {i} {name:?} needs exactly one electrode assignment").into()); }
            let (slot, (_, number, voltage)) = matches[0];
            if used[slot] || !(1..=1000).contains(number) || !voltage.is_finite() {
                return Err("assignments must be unambiguous, numbered 1..1000, with finite voltage".into());
            }
            used[slot] = true;
            if voltages.insert(*number, *voltage).is_some_and(|v| v != *voltage) { return Err("parts sharing a number need the same voltage".into()); }
            parts.push((*number, shape));
        }
        if used.contains(&false) { return Err("electrode assignment did not match any geometry part".into()); }
        let tolerance = 1e-9 * (0..3).filter(|&a| dims[a] > 1).map(|a| h[a]).fold(f64::INFINITY, f64::min);
        let mut indices = vec![-1; (*voltages.keys().max().unwrap() + 1).max(32)];
        for i in 0..file.array.potentials.len() {
            let mut owner = None;
            for (number, shape) in &parts {
                let d = shape.distance(point(i, dims, h, s.origin_mm));
                if !d.is_finite() { return Err("SDF returned a non-finite distance".into()); }
                if d <= tolerance {
                    if owner.is_some_and(|n| n != *number) { return Err("different electrode numbers overlap at a PA node".into()); }
                    owner = Some(*number);
                    if indices[*number] == -1 { indices[*number] = i as i64; }
                }
            }
            file.array.electrodes[i] = owner.is_some();
        }
        for number in voltages.keys() {
            if indices[*number] == -1 { return Err(format!("electrode {number} has no inside/on-surface node; refine the grid").into()); }
        }
        let grid = refine::grid(&file.array);
        let origin = if cylindrical { [0.0, s.origin_mm[0], 0.0] } else { s.origin_mm };
        let gpu = refine::gpu(&s.solve)?;
        if s.solve.device == "auto" {
            eprintln!("auto: {} ({})", if gpu.is_some() { "GPU" } else { "CPU" },
                if gpu.is_some() { "GPU adapter available for geometry refinement" } else { "no adapter available for the requested backend" });
        }
        let mut bases = Vec::new();
        let mut compute_ms = 0.0;
        let mut combined = vec![0.0; file.array.potentials.len()];
        for (&number, &voltage) in &voltages {
            let electrodes = parts.iter().map(|(n, shape)| {
                let shape = Arc::clone(shape);
                Electrode::new(if *n == number { 10_000.0 } else { 0.0 }, move |mut p: [f64; 3]| {
                    if cylindrical { p.swap(0, 1); }
                    let d = shape.distance(p);
                    if d.abs() <= tolerance { 0.0 } else { d / tracing_scale }
                })
            }).collect::<Vec<_>>();
            let cuts = CutCells::new_with_root_tolerance(grid, origin, &electrodes, tolerance / tracing_scale)?;
            let mut values = cuts.values.clone();
            let (converged, cycles, elapsed) = if let Some(gpu) = &gpu {
                let r = gpu.solve_sdf(&mut values, &cuts, refine::options(&s.solve))?;
                (r.converged, r.iterations, r.timings.solve)
            } else {
                let r = cuts.solve(&mut values, refine::options(&s.solve))?;
                (r.converged, r.iterations, r.timings.solve)
            };
            if !converged { return Err(format!("electrode {number} did not converge after {cycles} cycles; increase max_cycles").into()); }
            compute_ms += elapsed.as_secs_f64() * 1000.0;
            let mut basis = file.clone();
            basis.array.potentials = if cylindrical { transpose(&values, grid.dims[0], grid.dims[1]) } else { values };
            for (v, b) in combined.iter_mut().zip(&basis.array.potentials) { *v += b * voltage / 10_000.0; }
            bases.push((number, basis));
        }
        file.array.potentials = combined;
        file.pa0 = Some(Pa0Metadata { reference_voltage: 10_000.0, electrode_indices: indices, scalable: None });
        let report = serde_json::json!({"compute_ms":compute_ms,"total_ms":start.elapsed().as_secs_f64()*1000.0,"origin_mm":s.origin_mm,"device":if gpu.is_some() {"gpu"} else {"cpu"},"voltages":voltages}).to_string();
        Ok((file, bases, report))
    }).map_err(error)?;
    Ok((
        pa::arrays(py, file)?,
        bases
            .into_iter()
            .map(|(n, f)| Ok((n, pa::arrays(py, f)?)))
            .collect::<PyResult<_>>()?,
        report,
    ))
}
