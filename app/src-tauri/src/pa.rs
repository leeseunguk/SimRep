//! Opening SIMION .pa/.pa#/.pa0 files by path and handing the grid to the UI.
//!
//! The UI gets the native file path (Tauri drag-drop event or the open dialog), so the file is read
//! here and only the decoded grid crosses IPC, as one binary packet:
//!
//! ```text
//! "SPA1" | u32 LE header length | header JSON (UTF-8, space-padded to 4 bytes)
//!        | f32 LE potentials [nx*ny*nz] | u8 electrode flags [nx*ny*nz]
//! ```
//!
//! Mirrored by `parsePacket` / `encodePacket` in src/scene/pa.ts. The same packet carries refined
//! arrays back to the UI (src/refine.rs). Fast-adjusted arrays (live voltage edits) come as the
//! smaller `potentials_packet` instead.

use serde::Serialize;
use simrep_core::{FieldType, PotentialArray, Symmetry};
use simrep_io::{FastScalable, PaFile, PaMode};
use std::collections::HashSet;
use std::fs::File;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tauri::State;
use tauri::ipc::Response;

use crate::session::{AppState, FastAdjust, Session};

/// Largest array the viewer accepts (keep in sync with `PA_MAX_POINTS` in src/scene/pa.ts).
const MAX_POINTS: u64 = 40_000_000;
/// Distinct electrode potentials listed in the header (the count is exact up to `DISTINCT_CAP`).
const LISTED_VALUES: usize = 64;
const DISTINCT_CAP: usize = 1001;

/// A condition the UI states in the user's language: `code` selects the message `reason.<code>`
/// in src/i18n (en.ts, ko.ts) and `params` fill its placeholders (`reasonMessage` in
/// src/scene/backend.ts). Text from the OS, parsers and solvers is not translated: it goes in a
/// `detail` param (or `PaError::detail`) and the UI labels it as technical detail.
#[derive(Debug, Clone, Serialize)]
pub struct Reason {
    pub code: &'static str,
    #[serde(skip_serializing_if = "serde_json::Map::is_empty")]
    pub params: serde_json::Map<String, serde_json::Value>,
}

impl Reason {
    pub fn new(code: &'static str) -> Self {
        Self { code, params: serde_json::Map::new() }
    }

    pub fn with(mut self, name: &str, value: impl Serialize) -> Self {
        self.params.insert(name.to_owned(), serde_json::to_value(value).unwrap_or_default());
        self
    }

    /// Technical text (English, from a library or the OS) shown next to the localized message.
    pub fn detail(self, detail: impl std::fmt::Display) -> Self {
        self.with("detail", detail.to_string())
    }
}

/// Error shape the UI turns into an actionable toast (codes match `PaErrorCode` in pa.ts).
/// `reason`, when set, is what the UI shows (localized); `detail` is the technical text.
#[derive(Debug, Serialize)]
pub struct PaError {
    code: &'static str,
    detail: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<Reason>,
}

impl PaError {
    pub fn new(code: &'static str, detail: impl Into<String>) -> Self {
        Self { code, detail: detail.into(), reason: None }
    }

    /// An error the UI states through `reason` (localized); `detail` keeps the reason code for logs.
    pub fn coded(code: &'static str, reason: Reason) -> Self {
        Self { code, detail: reason.code.to_owned(), reason: Some(reason) }
    }

    /// The localized reason, or `fallback` carrying the technical detail.
    pub fn into_reason(self, fallback: &'static str) -> Reason {
        self.reason.unwrap_or_else(|| Reason::new(fallback).detail(self.detail))
    }
}

impl From<io::Error> for PaError {
    fn from(e: io::Error) -> Self {
        let detail = e.to_string();
        let code = match e.kind() {
            io::ErrorKind::NotFound => "notFound",
            io::ErrorKind::UnexpectedEof => "truncated",
            io::ErrorKind::InvalidData if detail.starts_with("unsupported PA header mode") => "notPa",
            io::ErrorKind::InvalidData if detail.contains("PA0") => "badPa0",
            io::ErrorKind::InvalidData => "badHeader",
            _ => "unexpected",
        };
        Self::new(code, detail)
    }
}

#[derive(Serialize)]
struct Flags {
    x: bool,
    y: bool,
    z: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Pa0Info {
    adjustable: usize,
    /// Electrode numbers that have a fast-adjust solution (index into the PA0 table).
    electrode_numbers: Vec<usize>,
    reference_voltage: f64,
    fast_scalable: bool,
}

/// Live fast adjustment: ready (siblings loaded, current voltages) or why it is unavailable.
#[derive(Serialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum FastStatus {
    Ready { electrodes: Vec<FastElectrode> },
    Missing { detail: String },
}

#[derive(Serialize)]
pub struct FastElectrode {
    pub number: usize,
    pub voltage: f64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PaHeader {
    mode: i32,
    symmetry: &'static str,
    nx: usize,
    ny: usize,
    nz: usize,
    dx: f64,
    dy: f64,
    dz: f64,
    mirror: Flags,
    anti_mirror: Flags,
    field: &'static str,
    refinable: bool,
    max_voltage: f64,
    ng: u32,
    points: usize,
    electrode_points: usize,
    potential_min: f64,
    potential_max: f64,
    electrode_min: Option<f64>,
    electrode_max: Option<f64>,
    electrode_values: Vec<f64>,
    distinct_electrode_values: usize,
    pa0: Option<Pa0Info>,
    /// Desktop only: where the array lives and its live fast-adjust state.
    path: String,
    file_name: String,
    file_size: u64,
    fast: Option<FastStatus>,
    /// Sibling `<stem>.cuts.json` (exact electrode surfaces for flights, session.rs), file name.
    cuts: Option<String>,
}

/// Where a packet's array came from (sent with every packet so the UI can relabel the object).
pub struct Origin<'a> {
    pub path: &'a Path,
    pub file_size: u64,
    pub fast: Option<FastStatus>,
}

/// Binary packet for `array` with `file`'s header fields (mode, PA0 table, ...).
pub fn packet(file: &PaFile, array: &PotentialArray, origin: Origin) -> Result<Vec<u8>, PaError> {
    let header = serde_json::to_vec(&describe(file, array, origin)).map_err(|e| PaError::new("unexpected", e.to_string()))?;
    let n = array.potentials.len();
    let padded = header.len().div_ceil(4) * 4;
    let mut out = Vec::with_capacity(8 + padded + n * 5);
    out.extend_from_slice(b"SPA1");
    out.extend_from_slice(&(padded as u32).to_le_bytes());
    out.extend_from_slice(&header);
    out.resize(8 + padded, b' ');
    for &p in &array.potentials {
        out.extend_from_slice(&(p as f32).to_le_bytes());
    }
    out.extend(array.electrodes.iter().map(|&e| u8::from(e)));
    Ok(out)
}

/// Live fast-adjust packet for an array the UI already holds: the same header, then only what a
/// voltage change alters. The electrode flags never change with the voltages (the UI keeps them
/// from the array's SPA1 packet), and the potentials are quantized to u16 over
/// [potential_min, potential_max]: the error is at most 1/131070 of the range, far below what the
/// colour map, the contour lines and the hover readout (range / 1000) resolve. 2 bytes per point
/// instead of 5 (quadrupole template: 333 KB instead of 830 KB).
///
/// ```text
/// "SPQ1" | u32 LE header length | header JSON (space-padded to 8 bytes)
///        | f64 LE offset | f64 LE step | u16 LE codes [nx*ny*nz]     potential = offset + code * step
/// ```
///
/// Mirrored by `parsePotentialsPacket` in src/scene/pa.ts.
pub fn potentials_packet(file: &PaFile, array: &PotentialArray, origin: Origin) -> Result<Vec<u8>, PaError> {
    let info = describe(file, array, origin);
    let (offset, hi) = (info.potential_min, info.potential_max);
    let step = if hi > offset { (hi - offset) / f64::from(u16::MAX) } else { 0.0 };
    let header = serde_json::to_vec(&info).map_err(|e| PaError::new("unexpected", e.to_string()))?;
    let padded = header.len().div_ceil(8) * 8;
    let mut out = Vec::with_capacity(8 + padded + 16 + array.potentials.len() * 2);
    out.extend_from_slice(b"SPQ1");
    out.extend_from_slice(&(padded as u32).to_le_bytes());
    out.extend_from_slice(&header);
    out.resize(8 + padded, b' ');
    out.extend_from_slice(&offset.to_le_bytes());
    out.extend_from_slice(&step.to_le_bytes());
    for &p in &array.potentials {
        let code = if step > 0.0 { ((p - offset) / step).round() as u16 } else { 0 };
        out.extend_from_slice(&code.to_le_bytes());
    }
    Ok(out)
}

fn describe(file: &PaFile, a: &PotentialArray, origin: Origin) -> PaHeader {
    let mut potential = (f64::INFINITY, f64::NEG_INFINITY);
    let mut electrode: Option<(f64, f64)> = None;
    let mut electrode_points = 0;
    let mut distinct = HashSet::new();
    for (&p, &e) in a.potentials.iter().zip(&a.electrodes) {
        potential = (potential.0.min(p), potential.1.max(p));
        if e {
            electrode_points += 1;
            electrode = Some(electrode.map_or((p, p), |(lo, hi)| (lo.min(p), hi.max(p))));
            if distinct.len() < DISTINCT_CAP {
                distinct.insert((p + 0.0).to_bits()); // + 0.0 folds -0 into 0
            }
        }
    }
    let mut electrode_values: Vec<f64> = distinct.iter().map(|&b| f64::from_bits(b)).collect();
    electrode_values.sort_by(f64::total_cmp);
    electrode_values.truncate(LISTED_VALUES);

    PaHeader {
        mode: match file.mode {
            PaMode::Standard => -1,
            PaMode::Extended => -2,
        },
        symmetry: match a.symmetry {
            Symmetry::Planar => "planar",
            Symmetry::Cylindrical => "cylindrical",
        },
        nx: a.nx,
        ny: a.ny,
        nz: a.nz,
        dx: a.dx_mm,
        dy: a.dy_mm,
        dz: a.dz_mm,
        mirror: Flags { x: a.mirrors.x, y: a.mirrors.y, z: a.mirrors.z },
        anti_mirror: Flags { x: a.anti_mirrors.x, y: a.anti_mirrors.y, z: a.anti_mirrors.z },
        field: match a.field_type {
            FieldType::Electrostatic => "electrostatic",
            FieldType::Magnetic => "magnetic",
        },
        refinable: a.refinable,
        max_voltage: file.max_voltage,
        ng: file.ng,
        points: a.potentials.len(),
        electrode_points,
        potential_min: potential.0,
        potential_max: potential.1,
        electrode_min: electrode.map(|e| e.0),
        electrode_max: electrode.map(|e| e.1),
        electrode_values,
        distinct_electrode_values: distinct.len(),
        pa0: file.pa0.as_ref().map(|m| Pa0Info {
            adjustable: m.electrode_indices.iter().filter(|&&i| i != -1).count(),
            electrode_numbers: (0..m.electrode_indices.len()).filter(|&i| m.electrode_indices[i] != -1).collect(),
            reference_voltage: m.reference_voltage,
            fast_scalable: matches!(m.scalable, Some(FastScalable::Voltages(_))),
        }),
        path: origin.path.display().to_string(),
        file_name: file_name(origin.path),
        file_size: origin.file_size,
        fast: origin.fast,
        cuts: crate::session::sidecar(origin.path).map(|p| file_name(&p)),
    }
}

pub fn file_name(path: &Path) -> String {
    path.file_name().map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned())
}

/// Refuse huge arrays before simrep-io reserves memory for them (only for a real PA header;
/// anything else gets simrep-io's own error).
fn check_size(path: &Path) -> Result<u64, PaError> {
    let mut f = File::open(path)?;
    let size = f.metadata()?.len();
    if size == 0 {
        return Err(PaError::new("empty", "0 bytes"));
    }
    let mut head = [0u8; 28];
    if f.read_exact(&mut head).is_ok() {
        let int = |at: usize| i32::from_le_bytes(head[at..at + 4].try_into().unwrap());
        if matches!(int(0), -1 | -2) {
            let dim = |at: usize| int(at).max(0) as u64;
            let points = dim(16).saturating_mul(dim(20)).saturating_mul(dim(24));
            if points > MAX_POINTS {
                return Err(PaError::new("tooLarge", points.to_string()));
            }
        }
    }
    Ok(size)
}

fn is_pa0(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("pa0"))
}

/// Opens a PA by path, registers it as session `id` and returns its packet. A .pa0 also loads its
/// sibling .pa1..N solutions for live fast adjustment (if they are missing the array still opens).
#[tauri::command]
pub async fn open_pa(state: State<'_, AppState>, id: String, path: String) -> Result<Response, PaError> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let path = PathBuf::from(path);
        let size = check_size(&path)?;
        let file = simrep_io::read_pa(&path)?;
        let fast = (is_pa0(&path) && file.pa0.is_some()).then(|| FastAdjust::load(&path, &file));
        let status = fast.as_ref().map(|f| match f {
            Ok(f) => f.status(),
            Err(e) => FastStatus::Missing { detail: e.to_string() },
        });
        let out = packet(&file, &file.array, Origin { path: &path, file_size: size, fast: status })?;
        state.insert(id, Arc::new(Session::new(path, file, fast.and_then(Result::ok))));
        Ok(Response::new(out))
    })
    .await
    .map_err(|e| PaError::new("unexpected", e.to_string()))?
}

/// Raw bytes of any file (STL and other formats parsed in the UI).
#[tauri::command]
pub async fn read_file(path: String) -> Result<Response, PaError> {
    tauri::async_runtime::spawn_blocking(move || Ok(Response::new(std::fs::read(path)?)))
        .await
        .map_err(|e| PaError::new("unexpected", e.to_string()))?
}

#[derive(Serialize)]
pub struct PathInfo {
    path: String,
    name: String,
    size: u64,
}

/// Name and size of dropped / picked paths (directories and missing files are skipped).
#[tauri::command]
pub fn path_info(paths: Vec<String>) -> Vec<PathInfo> {
    paths
        .into_iter()
        .filter_map(|p| {
            let meta = std::fs::metadata(&p).ok().filter(|m| m.is_file())?;
            Some(PathInfo { name: file_name(Path::new(&p)), size: meta.len(), path: p })
        })
        .collect()
}

#[tauri::command]
pub fn close_session(state: State<'_, AppState>, id: String) {
    state.remove(&id);
}

/// Adjustable electrode of every grid point of a fast-adjust .pa0, as u16 LE per point (0 for
/// non-electrode points and fixed electrodes). Arrays without fast adjust answer an empty body;
/// the UI then groups electrodes by their stored value.
#[tauri::command]
pub async fn electrode_labels(state: State<'_, AppState>, id: String) -> Result<Response, PaError> {
    let session = state.get(&id).ok_or_else(|| PaError::new("noSession", id))?;
    tauri::async_runtime::spawn_blocking(move || {
        let Some(fast) = &session.fast else { return Response::new(Vec::new()) };
        let electrodes = &session.file.array.electrodes;
        let mut out = Vec::with_capacity(electrodes.len() * 2);
        for (i, &e) in electrodes.iter().enumerate() {
            let label = if e { fast.electrode_number(i).map_or(0, |n| n.min(u16::MAX as usize) as u16) } else { 0 };
            out.extend_from_slice(&label.to_le_bytes());
        }
        Response::new(out)
    })
    .await
    .map_err(|e| PaError::new("unexpected", e.to_string()))
}
