//! Export dialog back end: electrode geometry, field grids, isopotential surfaces and SIMION PAs
//! of an open array, through `simrep-mesh` and `simrep-io` (trajectories: fly.rs).
//!
//! Coordinates follow the workbench: the array's placement (src/fly.rs `Placement`) is applied to
//! meshes in full and to grids as a translation (VTK image data cannot rotate). Every command
//! answers the files it wrote.

use std::path::{Path, PathBuf};

use serde::Deserialize;
use simrep_core::{PotentialArray, Symmetry};
use simrep_io::{PaFile, write_pa};
use simrep_mesh::{
    Mesh, SurfaceOptions, Transform, electrode_surface_with_options, pa_isosurface, write_mesh,
    write_revolved_vts, write_vti,
};
use tauri::State;

use crate::fly::Placement;
use crate::pa::{PaError, Reason, file_name};
use crate::session::{AppState, Session};

fn fail(detail: impl Into<String>) -> PaError {
    PaError::new("export", detail)
}

/// An export error the UI states in its language (`reason.<code>`).
fn fail_with(reason: Reason) -> PaError {
    PaError::coded("export", reason)
}

#[derive(Deserialize, Clone, Copy)]
#[serde(rename_all = "camelCase")]
pub struct Surface {
    angular_segments: usize,
    sweep_degrees: f64,
    extrusion_depth_mm: f64,
}

impl From<Surface> for SurfaceOptions {
    fn from(s: Surface) -> Self {
        Self {
            angular_segments: s.angular_segments,
            sweep_degrees: s.sweep_degrees,
            extrusion_depth_mm: s.extrusion_depth_mm,
        }
    }
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum What {
    /// Electrode surfaces, one part per electrode (named by `names`: (electrode number, name)).
    #[serde(rename_all = "camelCase")]
    Geometry {
        surface: Surface,
        names: Vec<(usize, String)>,
    },
    /// Potential (+ field) grid: .vti (array chart) or .vts (cylindrical, revolved).
    #[serde(rename_all = "camelCase")]
    Grid {
        field: bool,
        angular_segments: usize,
        sweep_degrees: f64,
    },
    #[serde(rename_all = "camelCase")]
    Isosurface { potential: f64, surface: Surface },
    /// SIMION array: "pa" (field at the voltages), "paHash" (electrode numbers), "pa0" (fast-adjust set).
    #[serde(rename_all = "camelCase")]
    Pa { format: String },
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportRequest {
    id: String,
    path: String,
    voltages: Vec<(usize, f64)>,
    placement: Option<Placement>,
    what: What,
}

/// Electrode number per point (fast-adjust .pa0, else the stored values: .pa# numbers / volts).
fn numbered(session: &Session) -> PotentialArray {
    let mut a = session.file.array.clone();
    if let Some(fast) = &session.fast {
        for (i, v) in a.potentials.iter_mut().enumerate() {
            *v = if a.electrodes[i] {
                fast.electrode_number(i).map_or(0.0, |n| n as f64)
            } else {
                0.0
            };
        }
    }
    a
}

/// Workbench transform of a placement, for meshes in PA mm.
fn transform(p: &Placement) -> Transform {
    let mut m = Transform::IDENTITY.0;
    for (row, m_row) in m.iter_mut().enumerate().take(3) {
        for (col, axis) in p.axes.iter().enumerate() {
            m_row[col] = axis[row] * p.scale;
        }
        m_row[3] = p.origin[row];
    }
    Transform(m)
}

fn place(mesh: &mut Mesh, placement: Option<&Placement>) -> Result<(), PaError> {
    if let Some(p) = placement.filter(|p| **p != Placement::IDENTITY) {
        for part in &mut mesh.parts {
            part.transform(transform(p))
                .map_err(|e| fail(e.to_string()))?;
        }
    }
    Ok(())
}

fn sibling(path: &Path, suffix: &str) -> PathBuf {
    path.with_extension(format!("pa{suffix}"))
}

#[tauri::command]
pub async fn export_array(
    state: State<'_, AppState>,
    request: ExportRequest,
) -> Result<Vec<String>, PaError> {
    let session = state
        .get(&request.id)
        .ok_or_else(|| PaError::new("noSession", request.id.clone()))?;
    tauri::async_runtime::spawn_blocking(move || run(&session, request))
        .await
        .map_err(|e| PaError::new("unexpected", e.to_string()))?
}

fn run(session: &Session, r: ExportRequest) -> Result<Vec<String>, PaError> {
    let path = PathBuf::from(&r.path);
    let origin = r.placement.map_or([0.0; 3], |p| p.origin);
    let solved = || -> Result<_, PaError> {
        if session.is_pa_hash() {
            return Err(fail_with(Reason::new("unrefined")));
        }
        Ok(session.array(&r.voltages)?)
    };
    match r.what {
        What::Geometry { surface, names } => {
            let mut mesh =
                electrode_surface_with_options(&numbered(session), [0.0; 3], None, surface.into())
                    .map_err(|e| fail(e.to_string()))?;
            for part in &mut mesh.parts {
                if let Some((_, name)) = names
                    .iter()
                    .find(|(n, _)| part.name == format!("electrode-{n}"))
                {
                    part.name.clone_from(name);
                }
            }
            place(&mut mesh, r.placement.as_ref())?;
            write_mesh(&path, &mesh).map_err(|e| fail(e.to_string()))?;
        }
        What::Grid {
            field,
            angular_segments,
            sweep_degrees,
        } => {
            let array = solved()?;
            let vts = path
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("vts"));
            if vts {
                if array.symmetry != Symmetry::Cylindrical || array.nz != 1 {
                    return Err(fail_with(Reason::new("vtsNeedsCylindrical")));
                }
                write_revolved_vts(
                    &path,
                    &array,
                    origin,
                    field,
                    angular_segments,
                    sweep_degrees,
                )
                .map_err(|e| fail(e.to_string()))?;
            } else {
                write_vti(&path, &array, origin, field).map_err(|e| fail(e.to_string()))?;
            }
        }
        What::Isosurface { potential, surface } => {
            let array = solved()?;
            let mut mesh = pa_isosurface(&array, [0.0; 3], potential, surface.into())
                .map_err(|e| fail(e.to_string()))?;
            if mesh.parts.iter().all(|p| p.triangles.is_empty()) {
                return Err(fail_with(Reason::new("noIsosurface").with("v", potential)));
            }
            place(&mut mesh, r.placement.as_ref())?;
            write_mesh(&path, &mesh).map_err(|e| fail(e.to_string()))?;
        }
        What::Pa { format } => return write_simion(session, &path, &format, &r.voltages),
    }
    Ok(vec![file_name(&path)])
}

pub(crate) fn write_simion(
    session: &Session,
    path: &Path,
    format: &str,
    voltages: &[(usize, f64)],
) -> Result<Vec<String>, PaError> {
    let io = |p: &Path, e: std::io::Error| fail(format!("{}: {e}", p.display()));
    match format {
        "pa" => {
            if session.is_pa_hash() {
                return Err(fail_with(Reason::new("unrefined")));
            }
            let mut file = session.file.clone();
            file.array = (*session.array(voltages)?).clone();
            file.pa0 = None;
            write_pa(path, &file).map_err(|e| io(path, e))?;
            Ok(vec![file_name(path)])
        }
        "paHash" => {
            let mut file = PaFile::new(numbered(session));
            file.array.refinable = true;
            file.ng = session.file.ng;
            write_pa(path, &file).map_err(|e| io(path, e))?;
            Ok(vec![file_name(path)])
        }
        "pa0" => {
            let fast = session
                .fast
                .as_ref()
                .ok_or_else(|| fail_with(Reason::new("noFastAdjustSet")))?;
            let mut base = session.file.clone();
            base.array = fast.superpose(&session.file.array, voltages)?;
            write_pa(path, &base).map_err(|e| io(path, e))?;
            let mut files = vec![file_name(path)];
            let mut basis = session.file.clone();
            basis.pa0 = None;
            basis.max_voltage = basis.max_voltage.max(100_000.0);
            for b in &fast.bases {
                let out = sibling(
                    path,
                    &if b.number == 0 {
                        "_".to_owned()
                    } else {
                        b.number.to_string()
                    },
                );
                basis.array.potentials.clone_from(&b.potentials);
                write_pa(&out, &basis).map_err(|e| io(&out, e))?;
                files.push(file_name(&out));
            }
            Ok(files)
        }
        other => Err(fail(format!("unknown PA format {other}"))),
    }
}
