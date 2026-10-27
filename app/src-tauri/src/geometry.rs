//! Import wizard back end: GEM, mesh (STL/OBJ/PLY/3MF/glTF/GLB) and CAD (STEP/IGES, cargo feature
//! `cad`, src/cad.rs) geometry to a refinable PA.
//!
//! `inspect_geometry` reads a file, checks/repairs the mesh parts and answers a preview packet plus
//! a grid suggestion. `build_geometry` turns the user's part assignments and grid into a `.pa#`:
//!
//! - **aligned**: the staircase electrode-number array (GEM rasterizer / mesh voxelizer); the
//!   normal refine (src/refine.rs) solves it.
//! - **cut-cell**: the physical nodal mask (`SDF <= 0`), the `.cuts.json` sidecar (version 3: the
//!   link cuts plus each part's source geometry, GEM CSG / mesh / CAD tessellation, which flights
//!   use as exact electrode surfaces, session.rs), and the SDFs kept in `AppState::geometries`, so
//!   refine solves each basis with the cut-cell operator.
//!
//! Same conventions as `simrep refine-geometry` (crates/simrep-cli/src/refine_geometry.rs, see
//! docs/geometry-pipeline.md): GEM parts are the exact CSG of `Gem::electrode_sdfs` (no sampled
//! lattice), seen by the solver through `SolverShape` (snapped to 0 within the surface tolerance,
//! CSG segment bounds kept for interval tracing); mesh parts use `Solid::signed_distance` directly;
//! CAD parts use `simrep_cad::CadPart` (exact B-rep or its fine tessellation, band 2 x spacing);
//! root tolerance `1e-9 * h_min`; cylindrical arrays are transposed to the solver's radial-x layout.
//!
//! Preview packet (mirrored by `parseGeometryPacket` in src/scene/geometry.ts):
//!
//! ```text
//! "SGP1" | u32 header length | header JSON (space-padded to 4 bytes)
//!        | f32 [px, py, pz, nx, ny, nz] per vertex, 3 vertices per triangle, parts in order
//! ```

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Arc;
use std::time::Instant;

use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use simrep_core::{FieldType, PotentialArray, Symmetry};
use simrep_fly::surface::{SourceGeometry, SourceSurface};
use simrep_geom::{Electrode, SignedDistance};
use simrep_io::{PaFile, write_pa};
use simrep_mesh::{
    GridSpec, ImportOptions, Mesh, MeshReport, Part, Solid, SurfaceOptions, Unit,
    electrode_surface_with_options, inspect, read_mesh, repair, voxelize,
};
use simrep_solve::cutcell::{CutCells, surface_fraction};
use simrep_solve::mg::{Geometry, Grid};
use tauri::State;
use tauri::ipc::Response;

use crate::pa::{PaError, Reason, file_name};
use crate::session::AppState;

/// Most preview triangles sent to the UI (larger meshes are drawn with every n-th triangle).
const PREVIEW_TRIANGLES: usize = 2_000_000;
/// The suggested grid stays below this many points.
const SUGGEST_POINTS: f64 = 4.0e6;
/// Largest grid the wizard builds (same as the PA viewer limit, src/pa.rs).
const MAX_POINTS: usize = 40_000_000;

/// Per-node cut fractions and electrode numbers along the six grid directions.
type NodeCuts = ([Option<f64>; 6], [Option<usize>; 6]);

fn fail(detail: impl Into<String>) -> PaError {
    PaError::new("geometry", detail)
}

/// A geometry error the UI states in its language (`reason.<code>`).
fn fail_with(reason: Reason) -> PaError {
    PaError::coded("geometry", reason)
}

fn is_gem(path: &Path) -> bool {
    path.extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("gem"))
}

fn is_cad(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| {
        ["step", "stp", "iges", "igs"]
            .iter()
            .any(|x| e.eq_ignore_ascii_case(x))
    })
}

fn units(text: &Option<String>) -> Result<Option<Unit>, PaError> {
    text.as_deref()
        .map(Unit::from_str)
        .transpose()
        .map_err(|e| fail(e.to_string()))
}

/// Default length unit of a mesh format (simrep-mesh's rules), for the units picker.
fn default_units(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("gltf" | "glb") => "m",
        Some("3mf" | "step" | "stp" | "iges" | "igs") => "model",
        _ => "mm",
    }
}

// --------------------------------------------------------------------------- inspection

#[derive(Serialize, Clone, Copy, Default)]
#[serde(rename_all = "camelCase")]
struct Issues {
    duplicate_vertices: usize,
    degenerate_triangles: usize,
    duplicate_triangles: usize,
    holes: usize,
    open_edges: usize,
    non_manifold_edges: usize,
    /// Inconsistent winding, inward shells and orientation conflicts.
    flipped: usize,
    closed: bool,
}

impl From<&MeshReport> for Issues {
    fn from(r: &MeshReport) -> Self {
        Self {
            duplicate_vertices: r.duplicate_vertices,
            degenerate_triangles: r.degenerate_triangles,
            duplicate_triangles: r.duplicate_triangles,
            holes: r.boundary_loops,
            open_edges: r.boundary_edges,
            non_manifold_edges: r.non_manifold_edges,
            flipped: r.inconsistent_edges + r.inward_components + r.orientation_conflicts,
            closed: r.is_closed_manifold() && r.inconsistent_edges == 0,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Report {
    before: Issues,
    /// After the safe repair (when it ran).
    after: Option<Issues>,
    removed_triangles: usize,
    flipped_triangles: usize,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PartInfo {
    index: usize,
    name: String,
    /// GEM electrode value (e.g. 2 for `e(2)`), None for meshes.
    source_value: Option<f64>,
    /// Triangles in the preview for this part.
    triangles: usize,
    bounds: Option<[[f64; 3]; 2]>,
    report: Option<Report>,
    /// Can be sampled as a solid (closed manifold after the optional repair).
    closed: bool,
    /// CAD only: the B-rep is valid, so the exact surface can be used (else its tessellation).
    exact: Option<bool>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct GemInfo {
    dims: [usize; 3],
    spacing: [f64; 3],
    symmetry: &'static str,
    mirrors: [bool; 3],
    refinable: bool,
    electrostatic: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Suggestion {
    spacing: f64,
    dims: [usize; 3],
    origin: [f64; 3],
    /// Thinnest wall (10th percentile over sampled surface points), mm.
    feature_mm: Option<f64>,
    /// Smallest gap between different parts, mm.
    gap_mm: Option<f64>,
    /// "pa_define" (GEM), "feature", "gap" or "budget".
    limited_by: &'static str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct InspectHeader {
    kind: &'static str,
    path: String,
    file_name: String,
    /// Effective source unit ("mm", ...); "model" for a 3MF without an override.
    units: String,
    default_units: &'static str,
    parts: Vec<PartInfo>,
    bounds: Option<[[f64; 3]; 2]>,
    gem: Option<GemInfo>,
    suggestion: Suggestion,
    preview_triangles: usize,
    /// Every n-th triangle is in the preview.
    preview_step: usize,
    ms: f64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InspectRequest {
    path: String,
    units: Option<String>,
    repair: bool,
}

fn union(a: Option<[[f64; 3]; 2]>, b: [[f64; 3]; 2]) -> [[f64; 3]; 2] {
    match a {
        None => b,
        Some(a) => [
            std::array::from_fn(|i| a[0][i].min(b[0][i])),
            std::array::from_fn(|i| a[1][i].max(b[1][i])),
        ],
    }
}

fn part_bounds(part: &Part) -> Option<[[f64; 3]; 2]> {
    part.vertices
        .iter()
        .fold(None, |b, &v| Some(union(b, [v, v])))
}

/// 1, 2 or 5 x 10^k at or below v.
fn nice_floor(v: f64) -> f64 {
    let p = 10f64.powf(v.log10().floor());
    let m = v / p;
    (if m >= 5.0 {
        5.0
    } else if m >= 2.0 {
        2.0
    } else {
        1.0
    }) * p
}

/// Next 1-2-5 step above v.
fn nice_next(v: f64) -> f64 {
    let p = 10f64.powf(v.log10().floor());
    let m = (v / p * 1e6).round() / 1e6;
    (if m < 2.0 {
        2.0
    } else if m < 5.0 {
        5.0
    } else {
        10.0
    }) * p
}

/// Wall thickness at evenly spread triangle centroids: from just inside the surface, the
/// distance to where the ray along the inward normal leaves the solid.
fn thicknesses(part: &Part, solid: &Solid, out: &mut Vec<f64>) {
    let Some(b) = part_bounds(part) else { return };
    let diagonal = (0..3)
        .map(|a| (b[1][a] - b[0][a]).powi(2))
        .sum::<f64>()
        .sqrt();
    let eps = 1e-6 * diagonal.max(1e-9);
    let n = part.triangles.len();
    let samples = n.min(256);
    for k in 0..samples {
        let [a, bb, c] = part.triangles[k * n / samples].map(|i| part.vertices[i]);
        let centroid: [f64; 3] = std::array::from_fn(|i| (a[i] + bb[i] + c[i]) / 3.0);
        let u: [f64; 3] = std::array::from_fn(|i| bb[i] - a[i]);
        let v: [f64; 3] = std::array::from_fn(|i| c[i] - a[i]);
        let normal = [
            u[1] * v[2] - u[2] * v[1],
            u[2] * v[0] - u[0] * v[2],
            u[0] * v[1] - u[1] * v[0],
        ];
        let length = normal.iter().map(|x| x * x).sum::<f64>().sqrt();
        if length == 0.0 {
            continue;
        }
        let along = |d: f64, t: f64| -> [f64; 3] {
            std::array::from_fn(|i| centroid[i] + normal[i] / length * d * t)
        };
        let inward = if solid.contains(along(-1.0, 4.0 * eps))
            && !solid.contains(along(1.0, 4.0 * eps))
        {
            -1.0
        } else if solid.contains(along(1.0, 4.0 * eps)) && !solid.contains(along(-1.0, 4.0 * eps)) {
            1.0
        } else {
            continue;
        };
        let mut lo = 4.0 * eps;
        let mut hi = lo;
        while solid.contains(along(inward, hi)) {
            lo = hi;
            hi *= 2.0;
            if hi > 2.0 * diagonal {
                break;
            }
        }
        for _ in 0..24 {
            let mid = 0.5 * (lo + hi);
            if solid.contains(along(inward, mid)) {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        out.push(hi);
    }
}

/// Smallest positive distance from up to 512 vertices of each part to every other part.
fn smallest_gap(parts: &[Part], distance: impl Fn(usize, [f64; 3]) -> Option<f64>) -> Option<f64> {
    let mut gap: Option<f64> = None;
    for (i, part) in parts.iter().enumerate() {
        for j in (0..parts.len()).filter(|&j| j != i) {
            let n = part.vertices.len();
            let samples = n.min(512);
            for k in 0..samples {
                if let Some(d) = distance(j, part.vertices[k * n / samples])
                    && d > 0.0
                    && gap.is_none_or(|g| d < g)
                {
                    gap = Some(d);
                }
            }
        }
    }
    gap
}

fn suggest_mesh(mesh: &Mesh, solids: &[Option<Solid>], bounds: [[f64; 3]; 2]) -> Suggestion {
    let mut walls = Vec::new();
    for (part, solid) in mesh.parts.iter().zip(solids) {
        if let Some(solid) = solid {
            thicknesses(part, solid, &mut walls);
        }
    }
    walls.sort_by(f64::total_cmp);
    let feature = walls.get(walls.len() / 10).copied();
    let gap = smallest_gap(&mesh.parts, |j, p| {
        solids[j].as_ref().map(|s| s.signed_distance(p))
    });
    suggest(feature, gap, bounds)
}

/// Grid spacing from the thinnest feature / smallest gap (4 points across), within the budget.
fn suggest(feature: Option<f64>, gap: Option<f64>, bounds: [[f64; 3]; 2]) -> Suggestion {
    let extent: [f64; 3] = std::array::from_fn(|a| (bounds[1][a] - bounds[0][a]).max(1e-9));
    let largest = extent.iter().copied().fold(0.0, f64::max);
    let (wanted, mut limited_by) = match (feature, gap) {
        (Some(f), Some(g)) if g < f => (g / 4.0, "gap"),
        (Some(f), _) => (f / 4.0, "feature"),
        (None, Some(g)) => (g / 4.0, "gap"),
        (None, None) => (largest / 64.0, "budget"),
    };
    let mut spacing = nice_floor(wanted.max(largest * 1e-4));
    let dims_for = |h: f64| -> ([usize; 3], [f64; 3]) {
        // Margin: 10% of the extent (at least two cells) on every side.
        let margin: [f64; 3] = std::array::from_fn(|a| (0.1 * extent[a]).max(2.0 * h));
        let dims = std::array::from_fn(|a| ((extent[a] + 2.0 * margin[a]) / h).ceil() as usize + 1);
        let origin = std::array::from_fn(|a| ((bounds[0][a] - margin[a]) / h).floor() * h);
        (dims, origin)
    };
    while dims_for(spacing)
        .0
        .iter()
        .map(|&n| n as f64)
        .product::<f64>()
        > SUGGEST_POINTS
    {
        spacing = nice_next(spacing);
        limited_by = "budget";
    }
    let (dims, origin) = dims_for(spacing);
    Suggestion {
        spacing,
        dims,
        origin,
        feature_mm: feature,
        gap_mm: gap,
        limited_by,
    }
}

fn push_triangles(out: &mut Vec<f32>, part: &Part, step: usize) -> usize {
    let mut count = 0;
    for face in part.triangles.iter().step_by(step) {
        let [a, b, c] = face.map(|i| part.vertices[i]);
        let u: [f64; 3] = std::array::from_fn(|i| b[i] - a[i]);
        let v: [f64; 3] = std::array::from_fn(|i| c[i] - a[i]);
        let n = [
            u[1] * v[2] - u[2] * v[1],
            u[2] * v[0] - u[0] * v[2],
            u[0] * v[1] - u[1] * v[0],
        ];
        let l = n.iter().map(|x| x * x).sum::<f64>().sqrt().max(1e-300);
        for p in [a, b, c] {
            out.extend(p.map(|x| x as f32));
            out.extend(n.map(|x| (x / l) as f32));
        }
        count += 1;
    }
    count
}

/// Electrode values of a GEM, in `sample_electrode_sdfs` order.
fn gem_values(gem: &simrep_gem::Gem) -> Vec<f64> {
    let mut values = Vec::new();
    for fill in &gem.fills {
        if fill.electrode && !values.contains(&fill.potential) {
            values.push(fill.potential);
        }
    }
    values
}

fn loaded_mesh(
    path: &Path,
    units: Option<Unit>,
    do_repair: bool,
) -> Result<(Mesh, Vec<Report>), PaError> {
    let mut mesh = read_mesh(
        path,
        ImportOptions {
            units,
            ..Default::default()
        },
    )
    .map_err(|e| fail(e.to_string()))?;
    let mut reports = Vec::new();
    for part in &mut mesh.parts {
        let before = inspect(part).map_err(|e| fail(e.to_string()))?;
        reports.push(if do_repair {
            let r = repair(part).map_err(|e| fail(e.to_string()))?;
            Report {
                before: (&before).into(),
                after: Some((&r.after).into()),
                removed_triangles: r.removed_triangles,
                flipped_triangles: r.flipped_triangles,
            }
        } else {
            Report {
                before: (&before).into(),
                after: None,
                removed_triangles: 0,
                flipped_triangles: 0,
            }
        });
    }
    Ok((mesh, reports))
}

fn part_name(part: &Part, index: usize) -> String {
    if part.name.is_empty() {
        format!("part {}", index + 1)
    } else {
        part.name.clone()
    }
}

#[tauri::command]
pub async fn inspect_geometry(request: InspectRequest) -> Result<Response, PaError> {
    tauri::async_runtime::spawn_blocking(move || {
        let start = Instant::now();
        let path = PathBuf::from(&request.path);
        let mut vertices: Vec<f32> = Vec::new();
        let mut parts = Vec::new();
        let mut bounds = None;
        let (kind, gem_info, suggestion, step, units_label) = if is_gem(&path) {
            let gem = simrep_gem::read_gem(&path).map_err(|e| fail(e.0))?;
            let raster = gem.rasterize().map_err(|e| fail(e.0))?;
            let a = &raster.pa.array;
            let cylindrical = a.symmetry == Symmetry::Cylindrical;
            let options = SurfaceOptions {
                angular_segments: 64,
                sweep_degrees: 360.0,
                extrusion_depth_mm: (a.ny as f64 * a.dy_mm).max(1.0),
            };
            let surface = electrode_surface_with_options(a, [0.0; 3], None, options)
                .map_err(|e| fail(e.to_string()))?;
            let total = surface.triangle_count();
            let step = total.div_ceil(PREVIEW_TRIANGLES).max(1);
            for (index, value) in gem_values(&gem).into_iter().enumerate() {
                let name = format!("electrode-{value}");
                let triangles = surface
                    .parts
                    .iter()
                    .find(|p| p.name == name)
                    .map_or(0, |p| push_triangles(&mut vertices, p, step));
                let b = surface
                    .parts
                    .iter()
                    .find(|p| p.name == name)
                    .and_then(part_bounds);
                if let Some(b) = b {
                    bounds = Some(union(bounds, b));
                }
                parts.push(PartInfo {
                    index,
                    name: format!("e({value})"),
                    source_value: Some(value),
                    triangles,
                    bounds: b,
                    report: None,
                    closed: true,
                    exact: None,
                });
            }
            let info = GemInfo {
                dims: gem.dimensions,
                spacing: gem.spacing_mm,
                symmetry: if cylindrical { "cylindrical" } else { "planar" },
                mirrors: [gem.mirrors.x, gem.mirrors.y, gem.mirrors.z],
                refinable: gem.refinable,
                electrostatic: gem.field_type == FieldType::Electrostatic,
            };
            let suggestion = Suggestion {
                spacing: gem.spacing_mm[0],
                dims: gem.dimensions,
                origin: [0.0; 3],
                feature_mm: None,
                gap_mm: None,
                limited_by: "pa_define",
            };
            ("gem", Some(info), suggestion, step, "mm".to_owned())
        } else if is_cad(&path) {
            let (suggestion, step) = inspect_cad(
                &path,
                units(&request.units)?,
                &mut vertices,
                &mut parts,
                &mut bounds,
            )?;
            let label = request.units.clone().unwrap_or_else(|| "model".to_owned());
            ("cad", None, suggestion, step, label)
        } else {
            let units = units(&request.units)?;
            let (mesh, reports) = loaded_mesh(&path, units, request.repair)?;
            let total = mesh.triangle_count();
            let step = total.div_ceil(PREVIEW_TRIANGLES).max(1);
            let solids: Vec<Option<Solid>> =
                mesh.parts.par_iter().map(|p| Solid::new(p).ok()).collect();
            for (index, ((part, report), solid)) in
                mesh.parts.iter().zip(reports).zip(&solids).enumerate()
            {
                let b = part_bounds(part);
                if let Some(b) = b {
                    bounds = Some(union(bounds, b));
                }
                let triangles = push_triangles(&mut vertices, part, step);
                parts.push(PartInfo {
                    index,
                    name: part_name(part, index),
                    source_value: None,
                    triangles,
                    bounds: b,
                    report: Some(report),
                    closed: solid.is_some(),
                    exact: None,
                });
            }
            let suggestion = suggest_mesh(&mesh, &solids, bounds.unwrap_or([[0.0; 3], [1.0; 3]]));
            let label = match (&request.units, default_units(&path)) {
                (Some(u), _) => u.clone(),
                (None, d) => d.to_owned(),
            };
            ("mesh", None, suggestion, step, label)
        };
        let header = InspectHeader {
            kind,
            file_name: file_name(&path),
            path: request.path,
            units: units_label,
            default_units: if kind == "gem" {
                "mm"
            } else {
                default_units(&path)
            },
            parts,
            bounds,
            gem: gem_info,
            suggestion,
            preview_triangles: vertices.len() / 18,
            preview_step: step,
            ms: start.elapsed().as_secs_f64() * 1e3,
        };
        let json = serde_json::to_vec(&header).map_err(|e| fail(e.to_string()))?;
        let padded = json.len().div_ceil(4) * 4;
        let mut out = Vec::with_capacity(8 + padded + vertices.len() * 4);
        out.extend_from_slice(b"SGP1");
        out.extend_from_slice(&(padded as u32).to_le_bytes());
        out.extend_from_slice(&json);
        out.resize(8 + padded, b' ');
        for v in vertices {
            out.extend_from_slice(&v.to_le_bytes());
        }
        Ok(Response::new(out))
    })
    .await
    .map_err(|e| PaError::new("unexpected", e.to_string()))?
}

// --------------------------------------------------------------------------------- CAD

/// Preview grid points along the longest axis of a CAD part (the preview is its zero isosurface).
#[cfg(feature = "cad")]
const CAD_PREVIEW_CELLS: f64 = 64.0;

#[cfg(feature = "cad")]
fn read_cad(
    path: &Path,
    units: Option<Unit>,
    mode: simrep_cad::Mode,
    band_mm: f64,
    chordal_tolerance_mm: f64,
) -> Result<Vec<simrep_cad::CadPart>, PaError> {
    crate::cad::ensure()?;
    simrep_cad::read_cad(
        path,
        simrep_cad::ImportOptions {
            mode,
            band_mm,
            chordal_tolerance_mm,
            units,
        },
    )
    .map_err(|e| fail(e.to_string()))
}

/// CAD inspection: B-rep validity and bounds (exact read), then a preview surface and the grid
/// suggestion from a fine tessellation's signed distance on a preview grid per part.
#[cfg(feature = "cad")]
fn inspect_cad(
    path: &Path,
    units: Option<Unit>,
    vertices: &mut Vec<f32>,
    parts: &mut Vec<PartInfo>,
    bounds: &mut Option<[[f64; 3]; 2]>,
) -> Result<(Suggestion, usize), PaError> {
    use simrep_mesh::isosurface;
    let exact = read_cad(path, units, simrep_cad::Mode::Auto, 1.0, 0.01)?;
    if exact.is_empty() {
        return Err(fail_with(Reason::new("noSolids")));
    }
    let all = exact.iter().fold(None, |b, p| Some(union(b, p.bounds_mm)));
    let all = all.unwrap_or([[0.0; 3], [1.0; 3]]);
    let diagonal = (0..3)
        .map(|a| (all[1][a] - all[0][a]).powi(2))
        .sum::<f64>()
        .sqrt()
        .max(1e-9);
    let tessellated = read_cad(
        path,
        units,
        simrep_cad::Mode::Tessellated,
        diagonal,
        diagonal / 2000.0,
    )?;
    let mut surfaces = Vec::with_capacity(tessellated.len());
    let mut feature: Option<f64> = None;
    for cad in &tessellated {
        let b = cad.bounds_mm;
        let extent: [f64; 3] = std::array::from_fn(|a| (b[1][a] - b[0][a]).max(1e-9));
        let mut h = extent.iter().copied().fold(0.0, f64::max) / CAD_PREVIEW_CELLS;
        let dims_for = |h: f64| -> [usize; 3] {
            std::array::from_fn(|a| ((extent[a] + 4.0 * h) / h).ceil() as usize + 1)
        };
        while dims_for(h).iter().product::<usize>() > 1_500_000 {
            h *= 1.25;
        }
        let grid = GridSpec {
            dims: dims_for(h),
            origin_mm: std::array::from_fn(|a| b[0][a] - 2.0 * h),
            spacing_mm: [h; 3],
        };
        let points = grid.dims.iter().product::<usize>();
        let values: Vec<f64> = (0..points)
            .into_par_iter()
            .map(|i| cad.distance(grid.point(i)))
            .collect();
        if values.iter().any(|v| !v.is_finite()) {
            return Err(fail_with(
                Reason::new("cadClassify").with("part", &cad.name),
            ));
        }
        // Thickest point of the body: twice its deepest inside distance.
        let depth = values.iter().fold(0.0f64, |m, v| m.max(-v));
        if depth > 0.0 {
            feature = Some(feature.map_or(2.0 * depth, |f: f64| f.min(2.0 * depth)));
        }
        let mesh = isosurface(&values, grid, 0.0).map_err(|e| fail(e.to_string()))?;
        let mut part = Part {
            name: cad.name.clone(),
            vertices: Vec::new(),
            triangles: Vec::new(),
        };
        for p in mesh.parts {
            let offset = part.vertices.len();
            part.vertices.extend(p.vertices);
            part.triangles
                .extend(p.triangles.into_iter().map(|t| t.map(|i| i + offset)));
        }
        surfaces.push(part);
    }
    let total: usize = surfaces.iter().map(|p| p.triangles.len()).sum();
    let step = total.div_ceil(PREVIEW_TRIANGLES).max(1);
    for (index, ((part, cad), body)) in surfaces.iter().zip(&tessellated).zip(&exact).enumerate() {
        *bounds = Some(union(*bounds, cad.bounds_mm));
        let triangles = push_triangles(vertices, part, step);
        let name = if cad.name.is_empty() {
            format!("body {}", index + 1)
        } else {
            cad.name.clone()
        };
        let valid = body.mode == simrep_cad::Mode::Exact;
        parts.push(PartInfo {
            index,
            name,
            source_value: None,
            triangles,
            bounds: Some(cad.bounds_mm),
            report: None,
            closed: true,
            exact: Some(valid),
        });
    }
    let gap = smallest_gap(&surfaces, |j, p| Some(tessellated[j].distance(p)));
    Ok((suggest(feature, gap, all), step))
}

#[cfg(not(feature = "cad"))]
fn inspect_cad(
    _: &Path,
    _: Option<Unit>,
    _: &mut Vec<f32>,
    _: &mut Vec<PartInfo>,
    _: &mut Option<[[f64; 3]; 2]>,
) -> Result<(Suggestion, usize), PaError> {
    crate::cad::ensure()?;
    unreachable!("cad::ensure fails without the cad feature")
}

/// A CAD body: name, signed distance for the build, portable flight geometry for the sidecar.
type CadSource = (String, Arc<dyn SignedDistance>, SourceGeometry);

/// A part of the build: name, electrode number, signed distance, flight geometry for the sidecar.
type Source = (String, usize, Arc<dyn SignedDistance>, SourceGeometry);

/// CAD bodies as signed distances for the build (exact B-rep, or a fine tessellation), with the
/// fine tessellation as their portable flight geometry (v3 sidecars never store B-reps).
#[cfg(feature = "cad")]
fn cad_sources(
    path: &Path,
    units: Option<Unit>,
    surface: CadSurface,
    spacing: [f64; 3],
) -> Result<Vec<CadSource>, PaError> {
    let h_min = spacing.into_iter().fold(f64::INFINITY, f64::min);
    let h_max = spacing.into_iter().fold(0.0, f64::max);
    // Same choices as `simrep refine-geometry` (docs/step-import.md): exact = Auto (tessellation
    // only for invalid B-reps), band 2 x spacing, chordal tolerance h_min / 100.
    let mode = match surface {
        CadSurface::Exact => simrep_cad::Mode::Auto,
        CadSurface::Tessellated => simrep_cad::Mode::Tessellated,
    };
    let chordal = h_min / 100.0;
    let parts = read_cad(path, units, mode, 2.0 * h_max, chordal)?;
    parts
        .into_iter()
        .enumerate()
        .map(|(i, part)| {
            let name = if part.name.is_empty() {
                format!("body {}", i + 1)
            } else {
                part.name.clone()
            };
            let source = SourceGeometry::mesh(&part.tessellation, Some(chordal))
                .map_err(|e| fail(e.to_string()))?;
            Ok((name, Arc::new(part) as Arc<dyn SignedDistance>, source))
        })
        .collect()
}

#[cfg(not(feature = "cad"))]
fn cad_sources(
    _: &Path,
    _: Option<Unit>,
    _: CadSurface,
    _: [f64; 3],
) -> Result<Vec<CadSource>, PaError> {
    crate::cad::ensure()?;
    unreachable!("cad::ensure fails without the cad feature")
}

// ------------------------------------------------------------------------------- build

/// Cut-cell geometry of a built `.pa#`, kept for its refine (src/refine.rs).
pub struct Prepared {
    /// Solver layout (cylindrical arrays: radial x, axial y).
    pub grid: Grid,
    pub origin: [f64; 3],
    parts: Vec<(usize, Arc<dyn SignedDistance>)>,
    /// Physical surface tolerance (mm), also the root tolerance.
    tolerance: f64,
    cylindrical: bool,
}

pub const REFERENCE: f64 = 10_000.0;

/// A part as the cut-cell solver sees it, built like `simrep refine-geometry`'s solver shape:
/// solver layout (cylindrical: radial x), distances within the surface tolerance snapped to 0, and
/// the source's segment bounds forwarded so exact GEM CSG keeps its interval tracing.
struct SolverShape {
    source: Arc<dyn SignedDistance>,
    cylindrical: bool,
    tolerance: f64,
}

impl SolverShape {
    fn point(&self, mut p: [f64; 3]) -> [f64; 3] {
        if self.cylindrical {
            p.swap(0, 1);
        }
        p
    }

    fn snap(&self, distance: f64) -> f64 {
        if distance.abs() <= self.tolerance {
            0.0
        } else {
            distance
        }
    }
}

impl SignedDistance for SolverShape {
    fn distance(&self, p: [f64; 3]) -> f64 {
        self.snap(self.source.distance(self.point(p)))
    }

    fn segment_distance_bounds(&self, from: [f64; 3], to: [f64; 3]) -> Option<[f64; 2]> {
        self.source
            .segment_distance_bounds(self.point(from), self.point(to))
            .map(|bounds| bounds.map(|d| self.snap(d)))
    }

    fn segment_single_interval(&self, from: [f64; 3], to: [f64; 3], level: f64) -> bool {
        self.source.segment_single_interval(
            self.point(from),
            self.point(to),
            if level.abs() <= self.tolerance {
                self.tolerance
            } else {
                level
            },
        )
    }
}

impl Prepared {
    /// Electrodes at their numbers (None) or at the reference voltage for `basis` (others 0 V).
    pub fn electrodes(&self, basis: Option<usize>) -> Vec<Electrode> {
        self.parts
            .iter()
            .map(|(number, shape)| {
                let voltage =
                    basis.map_or(
                        *number as f64,
                        |n| if *number == n { REFERENCE } else { 0.0 },
                    );
                Electrode::new(
                    voltage,
                    SolverShape {
                        source: Arc::clone(shape),
                        cylindrical: self.cylindrical,
                        tolerance: self.tolerance,
                    },
                )
            })
            .collect()
    }

    pub fn cut_cells(&self, basis: Option<usize>) -> Result<CutCells, String> {
        CutCells::new_with_root_tolerance(
            self.grid,
            self.origin,
            &self.electrodes(basis),
            self.tolerance,
        )
        .map_err(|e| e.to_string())
    }
}

#[derive(Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Method {
    Aligned,
    Cutcell,
}

/// Which CAD surface the cut cells see.
#[derive(Deserialize, Clone, Copy, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum CadSurface {
    /// The B-rep itself (its tessellation only where the B-rep is invalid). Slow to prepare.
    Exact,
    /// A tessellation at spacing / 100 chordal deflection.
    #[default]
    Tessellated,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Assignment {
    index: usize,
    number: usize,
    voltage: f64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BuildRequest {
    path: String,
    units: Option<String>,
    repair: bool,
    method: Method,
    parts: Vec<Assignment>,
    dims: [usize; 3],
    spacing: [f64; 3],
    origin: [f64; 3],
    output_dir: String,
    stem: String,
    /// Write `<stem>.cuts.json` (cut-cell only).
    sidecar: bool,
    #[serde(default)]
    cad_surface: CadSurface,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BuildResult {
    pa_hash: String,
    files: Vec<String>,
    points: usize,
    /// (electrode number, grid points) of the physical electrode mask.
    electrode_points: Vec<(usize, usize)>,
    cut_nodes: Option<usize>,
    ms: f64,
}

/// The `.cuts.json` schema of docs/geometry-pipeline.md (snake_case keys, as `SurfaceModel` reads).
#[derive(Serialize)]
struct Sidecar<'a> {
    version: u32,
    dims: [usize; 3],
    origin_mm: [f64; 3],
    spacing_mm: [f64; 3],
    symmetry: &'static str,
    mirrors: [bool; 3],
    electrode_numbers: Vec<usize>,
    /// Exact flight surfaces (GEM CSG, mesh triangles, CAD tessellation): `SurfaceModel` uses these
    /// instead of reconstructing surfaces from the link cuts below.
    source_geometry: Vec<SourceSurface>,
    assignments: Vec<SidecarAssignment<'a>>,
    node_electrodes: &'a [Option<usize>],
    solver_fixed: Vec<bool>,
    cut_fractions: Vec<[Option<f64>; 6]>,
    cut_electrodes: Vec<[Option<usize>; 6]>,
    surface_tolerance_mm: f64,
    snap_distance_mm: f64,
}

#[derive(Serialize)]
struct SidecarAssignment<'a> {
    part: &'a str,
    number: usize,
    voltage: f64,
}

fn point(i: usize, dims: [usize; 3], origin: [f64; 3], h: [f64; 3]) -> [f64; 3] {
    let c = [i % dims[0], i / dims[0] % dims[1], i / (dims[0] * dims[1])];
    std::array::from_fn(|a| origin[a] + c[a] as f64 * h[a])
}

fn permute<T: Copy>(v: &[T], nx: usize, ny: usize) -> Vec<T> {
    (0..v.len()).map(|i| v[i / ny + nx * (i % ny)]).collect()
}

fn validate_grid(dims: [usize; 3], spacing: [f64; 3], origin: [f64; 3]) -> Result<usize, PaError> {
    let points = dims
        .iter()
        .try_fold(1usize, |n, &d| n.checked_mul(d))
        .unwrap_or(usize::MAX);
    if dims[0] < 2
        || dims[1] < 2
        || dims[2] == 0
        || spacing.iter().any(|h| !h.is_finite() || *h <= 0.0)
        || origin.iter().any(|v| !v.is_finite())
    {
        return Err(fail_with(Reason::new("gridInvalid")));
    }
    if points > MAX_POINTS {
        return Err(fail_with(
            Reason::new("gridTooLarge")
                .with("points", points)
                .with("max", MAX_POINTS),
        ));
    }
    Ok(points)
}

#[tauri::command]
pub async fn build_geometry(
    state: State<'_, AppState>,
    request: BuildRequest,
) -> Result<BuildResult, PaError> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || build(&state, request))
        .await
        .map_err(|e| PaError::new("unexpected", e.to_string()))?
}

fn build(state: &AppState, r: BuildRequest) -> Result<BuildResult, PaError> {
    let start = Instant::now();
    let path = PathBuf::from(&r.path);
    let dir = PathBuf::from(&r.output_dir);
    std::fs::create_dir_all(&dir).map_err(|e| {
        fail_with(
            Reason::new("outputFolder")
                .with("dir", dir.display().to_string())
                .detail(e),
        )
    })?;
    let stem = r.stem.trim();
    if stem.is_empty() || stem.contains(['/', '\\', ':', '*', '?', '"', '<', '>', '|']) {
        return Err(fail_with(Reason::new("badFileName")));
    }
    let out = dir.join(format!("{stem}.pa#"));
    let gem_input = is_gem(&path);

    // Assignments: every part exactly once, numbers 1..=1000, a shared number shares its voltage.
    let mut voltages = BTreeMap::new();
    for a in &r.parts {
        if !(1..=1000).contains(&a.number) || !a.voltage.is_finite() {
            return Err(fail_with(Reason::new("partNumbers")));
        }
        if voltages
            .insert(a.number, a.voltage)
            .is_some_and(|v| v != a.voltage)
        {
            return Err(fail_with(Reason::new("sharedVoltage").with("n", a.number)));
        }
    }
    let number_of = |index: usize| r.parts.iter().find(|a| a.index == index).map(|a| a.number);

    // Sources: (name, number, shape, flight geometry for the v3 sidecar) per part, plus the PA template.
    let (mut pa, sources): (PaFile, Vec<Source>) = if gem_input {
        let gem = simrep_gem::read_gem(&path).map_err(|e| fail(e.0))?;
        if gem.field_type != FieldType::Electrostatic || !gem.refinable {
            return Err(fail_with(Reason::new("gemKind")));
        }
        let values = gem_values(&gem);
        if r.method == Method::Aligned {
            // The GEM's own grid: its staircase is defined on pa_define's points.
            let raster = gem.rasterize().map_err(|e| fail(e.0))?;
            let mut pa = raster.pa;
            for (i, (v, e)) in pa
                .array
                .potentials
                .iter_mut()
                .zip(&pa.array.electrodes)
                .enumerate()
            {
                if *e {
                    let k = values.iter().position(|x| x == v).ok_or_else(|| {
                        fail(format!("grid point {i} has an unknown electrode value {v}"))
                    })?;
                    *v = number_of(k).ok_or_else(|| {
                        fail_with(Reason::new("noNumber").with("part", format!("e({})", values[k])))
                    })? as f64;
                }
            }
            return finish_aligned(state, &out, pa, start);
        }
        validate_grid(r.dims, r.spacing, r.origin)?;
        let mut a = PotentialArray::new(r.dims[0], r.dims[1], r.dims[2]);
        a.symmetry = gem.symmetry;
        a.mirrors = gem.mirrors;
        [a.dx_mm, a.dy_mm, a.dz_mm] = r.spacing;
        // The exact CSG per electrode value, for the solve and as the sidecar's flight surface
        // (`simrep refine-geometry`'s GEM sources; `gem_values` order).
        let mut sources = Vec::new();
        for (k, csg) in gem
            .electrode_sdfs()
            .map_err(|e| fail(e.0))?
            .into_iter()
            .enumerate()
        {
            let name = format!("e({})", csg.voltage);
            let number = number_of(k)
                .ok_or_else(|| fail_with(Reason::new("noNumber").with("part", &name)))?;
            let source = SourceGeometry::gem(&csg).map_err(|e| fail(e.to_string()))?;
            sources.push((
                name,
                number,
                Arc::new(csg) as Arc<dyn SignedDistance>,
                source,
            ));
        }
        let mut pa = PaFile::new(a);
        pa.ng = gem.ng;
        (pa, sources)
    } else if is_cad(&path) {
        validate_grid(r.dims, r.spacing, r.origin)?;
        if r.dims[2] < 2 {
            return Err(fail_with(Reason::new("need3d")));
        }
        let mut sources = Vec::new();
        for (i, (name, shape, source)) in
            cad_sources(&path, units(&r.units)?, r.cad_surface, r.spacing)?
                .into_iter()
                .enumerate()
        {
            let number = number_of(i)
                .ok_or_else(|| fail_with(Reason::new("noNumber").with("part", &name)))?;
            sources.push((name, number, shape, source));
        }
        let mut a = PotentialArray::new(r.dims[0], r.dims[1], r.dims[2]);
        [a.dx_mm, a.dy_mm, a.dz_mm] = r.spacing;
        (PaFile::new(a), sources)
    } else {
        validate_grid(r.dims, r.spacing, r.origin)?;
        if r.dims[2] < 2 {
            return Err(fail_with(Reason::new("need3d")));
        }
        let (mesh, _) = loaded_mesh(&path, units(&r.units)?, r.repair)?;
        if r.method == Method::Aligned {
            let numbers = (0..mesh.parts.len())
                .map(|i| {
                    number_of(i).map(|n| n as u32).ok_or_else(|| {
                        fail_with(
                            Reason::new("noNumber").with("part", part_name(&mesh.parts[i], i)),
                        )
                    })
                })
                .collect::<Result<Vec<_>, _>>()?;
            let grid = GridSpec {
                dims: r.dims,
                origin_mm: r.origin,
                spacing_mm: r.spacing,
            };
            let v = voxelize(&mesh, grid, &numbers)
                .map_err(|e| fail_with(Reason::new("voxelize").detail(e)))?;
            return finish_aligned(state, &out, PaFile::new(v.array), start);
        }
        let mut sources = Vec::new();
        for (i, part) in mesh.parts.iter().enumerate() {
            let number = number_of(i).ok_or_else(|| {
                fail_with(Reason::new("noNumber").with("part", part_name(part, i)))
            })?;
            let solid = Solid::new(part).map_err(|_| {
                fail_with(Reason::new("notClosed").with("part", part_name(part, i)))
            })?;
            let source = SourceGeometry::mesh(part, None).map_err(|e| fail(e.to_string()))?;
            sources.push((
                part_name(part, i),
                number,
                Arc::new(move |p| solid.signed_distance(p)) as Arc<dyn SignedDistance>,
                source,
            ));
        }
        let mut a = PotentialArray::new(r.dims[0], r.dims[1], r.dims[2]);
        [a.dx_mm, a.dy_mm, a.dz_mm] = r.spacing;
        (PaFile::new(a), sources)
    };

    // Cut-cell: the physical nodal mask, the solver geometry and the sidecar.
    let a = &pa.array;
    let dims = [a.nx, a.ny, a.nz];
    let spacing = [a.dx_mm, a.dy_mm, a.dz_mm];
    let cylindrical = a.symmetry == Symmetry::Cylindrical;
    if cylindrical && (dims[2] != 1 || r.origin[1] != 0.0 || r.origin[2] != 0.0) {
        return Err(fail_with(Reason::new("cylindricalOrigin")));
    }
    let h_min = (0..3)
        .filter(|&k| dims[k] > 1)
        .map(|k| spacing[k])
        .fold(f64::INFINITY, f64::min);
    let tolerance = 1e-9 * h_min;
    let owners: Vec<Option<usize>> = (0..a.potentials.len())
        .into_par_iter()
        .map(|i| {
            let p = point(i, dims, r.origin, spacing);
            let mut owner = None;
            for (name, number, shape, _) in &sources {
                let d = shape.distance(p);
                if !d.is_finite() {
                    return Err(Reason::new("distanceFailed")
                        .with("part", name)
                        .with("x", format!("{:.4}", p[0]))
                        .with("y", format!("{:.4}", p[1]))
                        .with("z", format!("{:.4}", p[2])));
                }
                if d <= tolerance {
                    if owner.is_some_and(|n| n != *number) {
                        return Err(Reason::new("partsOverlap"));
                    }
                    owner = Some(*number);
                }
            }
            Ok(owner)
        })
        .collect::<Result<_, Reason>>()
        .map_err(fail_with)?;
    let mut counts: BTreeMap<usize, usize> = BTreeMap::new();
    for o in owners.iter().flatten() {
        *counts.entry(*o).or_default() += 1;
    }
    for number in voltages.keys() {
        if !counts.contains_key(number) {
            return Err(fail_with(Reason::new("electrodeEmpty").with("n", number)));
        }
    }
    if r.method == Method::Aligned {
        // CAD staircase (GEM and meshes returned above): the physical mask, refined as a .pa#.
        pa.array.electrodes = owners.iter().map(Option::is_some).collect();
        pa.array.potentials = owners.iter().map(|n| n.unwrap_or(0) as f64).collect();
        return finish_aligned(state, &out, pa, start);
    }
    let prepared = Prepared {
        grid: Grid {
            dims: if cylindrical {
                [dims[1], dims[0], 1]
            } else {
                dims
            },
            spacing: if cylindrical {
                [spacing[1], spacing[0], spacing[2]]
            } else {
                spacing
            },
            geometry: if cylindrical {
                Geometry::Cylindrical { radial_origin: 0.0 }
            } else {
                Geometry::Planar
            },
        },
        origin: if cylindrical {
            [0.0, r.origin[0], 0.0]
        } else {
            r.origin
        },
        parts: sources
            .iter()
            .map(|(_, n, s, _)| (*n, Arc::clone(s)))
            .collect(),
        tolerance,
        cylindrical,
    };
    let cuts = prepared.cut_cells(None).map_err(fail)?;
    let mut files = Vec::new();
    if r.sidecar {
        let cuts_per_node: Vec<NodeCuts> = (0..owners.len())
            .into_par_iter()
            .map(|i| {
                let mut fractions = [None; 6];
                let mut electrodes = [None; 6];
                if owners[i].is_some() {
                    return (fractions, electrodes);
                }
                let p = point(i, dims, r.origin, spacing);
                let c = [i % dims[0], i / dims[0] % dims[1], i / (dims[0] * dims[1])];
                for s in 0..6 {
                    let axis = s / 2;
                    if (s % 2 == 0 && c[axis] == 0) || (s % 2 == 1 && c[axis] + 1 == dims[axis]) {
                        continue;
                    }
                    let mut q = p;
                    q[axis] += if s % 2 == 0 {
                        -spacing[axis]
                    } else {
                        spacing[axis]
                    };
                    for (_, number, shape, _) in &sources {
                        if let Some(t) = surface_fraction(shape.as_ref(), p, q, tolerance)
                            && fractions[s].is_none_or(|previous| t < previous)
                        {
                            fractions[s] = Some(t);
                            electrodes[s] = Some(*number);
                        }
                    }
                }
                (fractions, electrodes)
            })
            .collect();
        let (cut_fractions, cut_electrodes) = cuts_per_node.into_iter().unzip();
        let sidecar = Sidecar {
            version: 3,
            dims,
            origin_mm: r.origin,
            spacing_mm: spacing,
            symmetry: if cylindrical { "cylindrical" } else { "planar" },
            mirrors: [a.mirrors.x, a.mirrors.y, a.mirrors.z],
            electrode_numbers: voltages.keys().copied().collect(),
            source_geometry: sources
                .iter()
                .map(|(_, number, _, geometry)| SourceSurface {
                    number: *number,
                    geometry: geometry.clone(),
                })
                .collect(),
            assignments: sources
                .iter()
                .map(|(name, number, _, _)| SidecarAssignment {
                    part: name,
                    number: *number,
                    voltage: voltages[number],
                })
                .collect(),
            node_electrodes: &owners,
            solver_fixed: if cylindrical {
                permute(&cuts.fixed, prepared.grid.dims[0], prepared.grid.dims[1])
            } else {
                cuts.fixed.clone()
            },
            cut_fractions,
            cut_electrodes,
            surface_tolerance_mm: tolerance,
            snap_distance_mm: cuts.snap_distance,
        };
        let side = dir.join(format!("{stem}.cuts.json"));
        let mut writer = BufWriter::new(
            File::create(&side).map_err(|e| fail(format!("{}: {e}", side.display())))?,
        );
        serde_json::to_writer(&mut writer, &sidecar).map_err(|e| fail(e.to_string()))?;
        writer.flush().map_err(|e| fail(e.to_string()))?;
        files.push(file_name(&side));
    } else {
        // No sidecar asked for: an older one would give the refined array stale exact surfaces.
        let side = dir.join(format!("{stem}.cuts.json"));
        if side.is_file() {
            std::fs::remove_file(&side).map_err(|e| fail(format!("{}: {e}", side.display())))?;
        }
    }
    pa.array.electrodes = owners.iter().map(Option::is_some).collect();
    pa.array.potentials = owners.iter().map(|n| n.unwrap_or(0) as f64).collect();
    write_pa(&out, &pa).map_err(|e| fail(format!("{}: {e}", out.display())))?;
    files.insert(0, file_name(&out));
    state
        .geometries
        .lock()
        .unwrap()
        .insert(out.clone(), Arc::new(prepared));
    Ok(BuildResult {
        pa_hash: out.display().to_string(),
        files,
        points: owners.len(),
        electrode_points: counts.into_iter().collect(),
        cut_nodes: Some(cuts.cut_nodes()),
        ms: start.elapsed().as_secs_f64() * 1e3,
    })
}

fn finish_aligned(
    state: &AppState,
    out: &Path,
    pa: PaFile,
    start: Instant,
) -> Result<BuildResult, PaError> {
    let mut counts: BTreeMap<usize, usize> = BTreeMap::new();
    for (v, e) in pa.array.potentials.iter().zip(&pa.array.electrodes) {
        if *e {
            *counts.entry(*v as usize).or_default() += 1;
        }
    }
    if counts.is_empty() {
        return Err(fail_with(Reason::new("noInside")));
    }
    write_pa(out, &pa).map_err(|e| fail(format!("{}: {e}", out.display())))?;
    // A rebuilt staircase replaces any cut-cell geometry kept for the same file, including its
    // sidecar: the refined .pa0 would otherwise fly against the old exact surfaces.
    state.geometries.lock().unwrap().remove(out);
    let side = out.with_extension("cuts.json");
    if side.is_file() {
        std::fs::remove_file(&side).map_err(|e| fail(format!("{}: {e}", side.display())))?;
    }
    Ok(BuildResult {
        pa_hash: out.display().to_string(),
        files: vec![file_name(out)],
        points: pa.array.potentials.len(),
        electrode_points: counts.into_iter().collect(),
        cut_nodes: None,
        ms: start.elapsed().as_secs_f64() * 1e3,
    })
}
