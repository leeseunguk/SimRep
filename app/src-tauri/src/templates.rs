//! Starter geometries for the template gallery: small .pa# arrays built from boxes and cylinders.
//!
//! Each template is a list of parts painted in order onto an empty grid (later parts win, an
//! electrode number of 0 clears points, e.g. to drill an aperture). Coordinates are mm from grid
//! point (0, 0, 0). Cylindrical arrays are 2D in (x, r): a box there is an annulus about X.
//! The UI refines the result right away (they are small, so "auto" picks the CPU) and sets the
//! template's voltages and ion source (src/scene/templates.ts).

use serde::Serialize;
use simrep_core::{MirrorFlags, PotentialArray, Symmetry};
use simrep_io::{PaFile, write_pa};
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Manager};

use crate::pa::{PaError, file_name};

enum Shape {
    /// Axis-aligned box, [min, max] in mm (x, y, z). Cylindrical arrays use (x, r) only.
    Box([f64; 3], [f64; 3]),
    /// Cylinder along X through (y, z), from x0 to x1 (3D arrays).
    Rod { y: f64, z: f64, radius: f64, x0: f64, x1: f64 },
}

struct Part {
    electrode: u32,
    shape: Shape,
}

const fn part(electrode: u32, shape: Shape) -> Part {
    Part { electrode, shape }
}

pub(crate) struct Template {
    file: &'static str,
    cylindrical: bool,
    /// Grid points per axis and cell size (mm).
    dims: [usize; 3],
    cell: f64,
    mirror: MirrorFlags,
    parts: Vec<Part>,
}

const BIG: f64 = 1e6;

pub(crate) fn template(kind: &str) -> Option<Template> {
    // SIMION's cylindrical arrays are mirrored in y (the radius).
    let radial = MirrorFlags { x: false, y: true, z: false };
    let annulus = |x0: f64, x1: f64, r0: f64, r1: f64| Shape::Box([x0, r0, -BIG], [x1, r1, BIG]);
    Some(match kind {
        // Three tubes on the axis: ground, focus, ground (100 x 20 mm, 0.5 mm cells).
        "einzel" => Template {
            file: "einzel.pa#",
            cylindrical: true,
            dims: [201, 41, 1],
            cell: 0.5,
            mirror: radial,
            parts: vec![
                part(1, annulus(14.0, 38.0, 10.0, 12.0)),
                part(2, annulus(42.0, 58.0, 10.0, 12.0)),
                part(3, annulus(62.0, 86.0, 10.0, 12.0)),
            ],
        },
        // Four rods along X (r0 = 5 mm), grounded end plates with 4 mm apertures. 3D, mirrored in
        // y and z, so only the y, z >= 0 quadrant is stored (60 x 18 x 18 mm, 0.5 mm cells).
        "quadrupole" => {
            let rod = 5.75;
            let at = 5.0 + rod;
            Template {
                file: "quadrupole.pa#",
                cylindrical: false,
                dims: [121, 37, 37],
                cell: 0.5,
                mirror: MirrorFlags { x: false, y: true, z: true },
                parts: vec![
                    part(1, Shape::Rod { y: at, z: 0.0, radius: rod, x0: 12.0, x1: 48.0 }),
                    part(2, Shape::Rod { y: 0.0, z: at, radius: rod, x0: 12.0, x1: 48.0 }),
                    part(3, Shape::Box([6.0, -BIG, -BIG], [7.0, BIG, BIG])),
                    part(3, Shape::Box([53.0, -BIG, -BIG], [54.0, BIG, BIG])),
                    part(0, Shape::Rod { y: 0.0, z: 0.0, radius: 4.0, x0: 5.0, x1: 55.0 }),
                ],
            }
        }
        // Gridless ion mirror: a grounded entrance ring, five ramp rings and a back plate
        // (120 x 30 mm, 0.5 mm cells).
        "reflectron" => {
            let mut parts = vec![part(1, annulus(30.0, 32.0, 18.0, 22.0))];
            for k in 0..5 {
                let x = 42.0 + 10.0 * f64::from(k);
                parts.push(part(2 + k, annulus(x, x + 2.0, 18.0, 22.0)));
            }
            parts.push(part(7, annulus(100.0, 102.0, 0.0, 22.0)));
            Template { file: "reflectron.pa#", cylindrical: true, dims: [241, 61, 1], cell: 0.5, mirror: radial, parts }
        }
        // Cylindrical trap: a ring between two end caps with 2 mm injection holes (40 x 20 mm).
        "trap" => Template {
            file: "trap.pa#",
            cylindrical: true,
            dims: [161, 81, 1],
            cell: 0.25,
            mirror: radial,
            parts: vec![
                part(1, annulus(14.0, 26.0, 9.0, 12.0)),
                part(2, annulus(9.0, 11.0, 2.0, 12.0)),
                part(2, annulus(29.0, 31.0, 2.0, 12.0)),
            ],
        },
        _ => return None,
    })
}

impl Shape {
    fn contains(&self, p: [f64; 3], cylindrical: bool) -> bool {
        match *self {
            Shape::Box(lo, hi) => (0..if cylindrical { 2 } else { 3 }).all(|a| p[a] >= lo[a] - 1e-9 && p[a] <= hi[a] + 1e-9),
            Shape::Rod { y, z, radius, x0, x1 } => {
                p[0] >= x0 - 1e-9 && p[0] <= x1 + 1e-9 && (p[1] - y).hypot(p[2] - z) <= radius + 1e-9
            }
        }
    }
}

fn build(t: &Template) -> PaFile {
    let [nx, ny, nz] = t.dims;
    let mut a = PotentialArray::new(nx, ny, nz);
    a.symmetry = if t.cylindrical { Symmetry::Cylindrical } else { Symmetry::Planar };
    a.mirrors = t.mirror;
    (a.dx_mm, a.dy_mm, a.dz_mm) = (t.cell, t.cell, t.cell);
    for k in 0..nz {
        for j in 0..ny {
            for i in 0..nx {
                let p = [i as f64 * t.cell, j as f64 * t.cell, k as f64 * t.cell];
                let Some(owner) = t.parts.iter().rev().find(|part| part.shape.contains(p, t.cylindrical)) else { continue };
                let index = (k * ny + j) * nx + i;
                a.electrodes[index] = owner.electrode != 0;
                a.potentials[index] = f64::from(owner.electrode);
            }
        }
    }
    PaFile::new(a)
}

/// First free `<base>`, `<base>-2`, ... folder under `root`.
fn fresh_dir(root: &Path, base: &str) -> PathBuf {
    (1..)
        .map(|n| if n == 1 { root.join(base) } else { root.join(format!("{base}-{n}")) })
        .find(|p| !p.exists())
        .expect("unbounded range")
}

#[derive(Serialize)]
pub struct Created {
    path: String,
    name: String,
    size: u64,
    dir: String,
}

/// Writes template `kind` as a .pa# into a new folder `Documents/SimRep/<kind>[-n]` and returns it.
#[tauri::command]
pub async fn create_template(app: AppHandle, kind: String) -> Result<Created, PaError> {
    let t = template(&kind).ok_or_else(|| PaError::new("unexpected", format!("unknown template {kind}")))?;
    let documents = app.path().document_dir().map_err(|e| PaError::new("unexpected", e.to_string()))?;
    tauri::async_runtime::spawn_blocking(move || {
        let dir = fresh_dir(&documents.join("SimRep"), &kind);
        std::fs::create_dir_all(&dir)?;
        let path = dir.join(t.file);
        write_pa(&path, &build(&t))?;
        let size = std::fs::metadata(&path)?.len();
        Ok(Created { name: file_name(&path), path: path.display().to_string(), size, dir: dir.display().to_string() })
    })
    .await
    .map_err(|e| PaError::new("unexpected", e.to_string()))?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_template_has_its_numbered_electrodes() {
        for (kind, numbers) in [("einzel", 3), ("quadrupole", 3), ("reflectron", 7), ("trap", 2)] {
            let file = build(&template(kind).unwrap());
            let mut seen: Vec<u32> = file.array.potentials.iter().zip(&file.array.electrodes).filter(|e| *e.1).map(|e| *e.0 as u32).collect();
            seen.sort_unstable();
            seen.dedup();
            assert_eq!(seen, (1..=numbers).collect::<Vec<_>>(), "{kind}");
        }
    }

    #[test]
    fn quadrupole_aperture_is_open() {
        let file = build(&template("quadrupole").unwrap());
        let a = &file.array;
        // End plate at x = 6.5 mm (i = 13): open on the axis, closed outside the 4 mm aperture.
        assert!(!a.electrodes[a.index(13, 0, 0).unwrap()]);
        assert!(a.electrodes[a.index(13, 20, 0).unwrap()]);
    }
}
