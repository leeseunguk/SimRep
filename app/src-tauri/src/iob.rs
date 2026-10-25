//! SIMION workbench files (.iob): workbench bounds and the placement of each PA instance.
//!
//! The binary layout is not documented. It was read from the installed SIMION-2024 examples and
//! checked against SIMION's documented Lua API (`simion.wb:load`, `simion.wb.bounds`,
//! `inst:pa_to_wb_coords`) on all 148 example IOBs whose PAs load, with no mismatch (bounds,
//! instance count, file names and the full PA-to-workbench transform). Only these fields are read;
//! everything else (view settings, FLY/REC/Lua references) is ignored.
//!
//! ```text
//! [52 bytes "SIMIOB..." prefix, newer files only]
//! f64 bounds[6] (xl, yl, zl, xr, yr, zr; mm) | i32 instance count
//! records[count], 748 bytes each (772 in "SIMIOB" files):
//!   +180  f64 m[4][4], row major: workbench mm = m * (PA grid units, 1)
//!   +692  12-byte 8.3 file name, space padded
//! after the fixed block (20296 / 20828 bytes): the long file names, NUL-separated, in order
//! ```
//!
//! Candidate for promotion into `simrep-io` (the app keeps it until a backend owner takes it).

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::pa::{PaError, Reason};

const PREFIX: &[u8] = b"SIMIOB";
const PREFIX_LEN: usize = 52;
const HEADER: usize = 52;
const MATRIX: usize = 180;
const NAME: usize = 692;
const NAME_LEN: usize = 12;

struct Layout {
    base: usize,
    stride: usize,
    fixed: usize,
}

const CLASSIC: Layout = Layout { base: 0, stride: 748, fixed: 20296 };
const SIMIOB: Layout = Layout { base: PREFIX_LEN, stride: 772, fixed: 20828 };

#[derive(Debug, Clone, PartialEq)]
pub struct Instance {
    pub file_name: String,
    /// Rows of the PA-to-workbench transform: workbench mm = m * (grid units, 1).
    pub matrix: [[f64; 4]; 3],
}

#[derive(Debug, Clone, PartialEq)]
pub struct Workbench {
    /// xl, yl, zl, xr, yr, zr (mm).
    pub bounds: [f64; 6],
    pub instances: Vec<Instance>,
}

fn f64_at(bytes: &[u8], at: usize) -> f64 {
    f64::from_le_bytes(bytes[at..at + 8].try_into().unwrap())
}

pub fn parse(bytes: &[u8]) -> Result<Workbench, Reason> {
    let layout = if bytes.starts_with(PREFIX) { SIMIOB } else { CLASSIC };
    if bytes.len() < layout.base + HEADER {
        return Err(Reason::new("iobShort"));
    }
    let bounds: [f64; 6] = std::array::from_fn(|i| f64_at(bytes, layout.base + 8 * i));
    let count = i32::from_le_bytes(bytes[layout.base + 48..layout.base + 52].try_into().unwrap());
    let records = layout.base + HEADER;
    let max = (layout.fixed.min(bytes.len()) - records) / layout.stride;
    if count < 1 || count as usize > max {
        return Err(Reason::new("iobCount").with("count", count).with("max", max));
    }
    if bounds.iter().any(|v| !v.is_finite()) || (0..3).any(|a| bounds[a] >= bounds[a + 3]) {
        return Err(Reason::new("iobBounds"));
    }
    let long: Vec<&[u8]> = bytes.get(layout.fixed..).map(|t| t.split(|&b| b == 0).collect()).unwrap_or_default();
    let mut instances = Vec::with_capacity(count as usize);
    for i in 0..count as usize {
        let rec = records + layout.stride * i;
        let m: [f64; 16] = std::array::from_fn(|k| f64_at(bytes, rec + MATRIX + 8 * k));
        if m.iter().any(|v| !v.is_finite()) || m[12..] != [0.0, 0.0, 0.0, 1.0] {
            return Err(Reason::new("iobPlacement").with("n", i + 1));
        }
        let short = &bytes[rec + NAME..rec + NAME + NAME_LEN];
        let short = short.split(|&b| b == 0).next().unwrap_or_default();
        let short = String::from_utf8_lossy(short).trim().to_owned();
        let file_name = long
            .get(i)
            .filter(|n| !n.is_empty())
            .map(|n| String::from_utf8_lossy(n).into_owned())
            .unwrap_or(short);
        if file_name.is_empty() {
            return Err(Reason::new("iobName").with("n", i + 1));
        }
        instances.push(Instance { file_name, matrix: [[m[0], m[1], m[2], m[3]], [m[4], m[5], m[6], m[7]], [m[8], m[9], m[10], m[11]]] });
    }
    Ok(Workbench { bounds, instances })
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IobInstance {
    file_name: String,
    /// The PA next to the .iob (case-insensitive match), when it exists.
    path: Option<String>,
    /// Workbench position of PA grid point (0, 0, 0), mm.
    origin: [f64; 3],
    /// PA x, y, z unit directions in workbench coordinates.
    axes: [[f64; 3]; 3],
    /// Workbench mm per PA grid unit along each PA axis (instance scale x PA spacing).
    grid_unit_mm: [f64; 3],
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IobInfo {
    path: String,
    file_name: String,
    bounds: [f64; 6],
    instances: Vec<IobInstance>,
}

/// `name` in `dir`, matching case-insensitively (SIMION files come from Windows).
fn resolve(dir: &Path, name: &str) -> Option<PathBuf> {
    let exact = dir.join(name);
    if exact.is_file() {
        return Some(exact);
    }
    let lower = name.to_lowercase();
    std::fs::read_dir(dir).ok()?.filter_map(Result::ok).map(|e| e.path()).find(|p| p.is_file() && p.file_name().is_some_and(|n| n.to_string_lossy().to_lowercase() == lower))
}

#[tauri::command]
pub async fn open_iob(path: String) -> Result<IobInfo, PaError> {
    tauri::async_runtime::spawn_blocking(move || {
        let path = PathBuf::from(path);
        let bench = parse(&std::fs::read(&path)?).map_err(|e| PaError::coded("iob", e))?;
        let dir = path.parent().map(Path::to_path_buf).unwrap_or_default();
        let instances = bench
            .instances
            .into_iter()
            .map(|inst| {
                let m = inst.matrix;
                let column = |c: usize| [m[0][c], m[1][c], m[2][c]];
                let length = |v: [f64; 3]| (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
                let grid_unit_mm = [0, 1, 2].map(|c| length(column(c)));
                let axes = [0, 1, 2].map(|c| column(c).map(|v| v / grid_unit_mm[c]));
                IobInstance {
                    path: resolve(&dir, &inst.file_name).map(|p| p.display().to_string()),
                    file_name: inst.file_name,
                    origin: column(3),
                    axes,
                    grid_unit_mm,
                }
            })
            .collect();
        Ok(IobInfo { file_name: crate::pa::file_name(&path), path: path.display().to_string(), bounds: bench.bounds, instances })
    })
    .await
    .map_err(|e| PaError::new("unexpected", e.to_string()))?
}
