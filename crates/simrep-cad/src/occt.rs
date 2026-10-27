use std::ffi::{CStr, CString, c_char, c_void};
use std::io;
use std::path::Path;
use std::ptr::NonNull;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use rayon::prelude::*;
use simrep_geom::{Point, SignedDistance};
use simrep_mesh::{Part, Solid, Unit, weld_and_orient};

unsafe extern "C" {
    fn simrep_cad_read(
        path: *const c_char,
        iges: bool,
        units: f64,
        error: *mut c_char,
        size: usize,
    ) -> *mut c_void;
    fn simrep_cad_free(model: *mut c_void);
    fn simrep_cad_count(model: *mut c_void) -> usize;
    fn simrep_cad_name(model: *mut c_void, index: usize) -> *const c_char;
    fn simrep_cad_valid(model: *mut c_void, index: usize) -> bool;
    fn simrep_cad_tshape(model: *mut c_void, index: usize) -> usize;
    fn simrep_cad_bounds(model: *mut c_void, index: usize, bounds: *mut f64);
    fn simrep_cad_distance(model: *mut c_void, index: usize, point: *const f64) -> f64;
    fn simrep_cad_tessellate(
        model: *mut c_void,
        index: usize,
        tolerance: f64,
        parallel: bool,
        error: *mut c_char,
        size: usize,
    ) -> *mut c_void;
    fn simrep_cad_triangle_values(triangles: *mut c_void) -> usize;
    fn simrep_cad_triangle_data(triangles: *mut c_void) -> *const f64;
    fn simrep_cad_triangles_free(triangles: *mut c_void);
    fn simrep_cad_parallel_tessellation_matches(
        model: *mut c_void,
        index: usize,
        tolerance: f64,
        error: *mut c_char,
        size: usize,
    ) -> bool;
    fn simrep_cad_fixture(
        path: *const c_char,
        kind: i32,
        schema: *const c_char,
        units: *const c_char,
        error: *mut c_char,
        size: usize,
    ) -> bool;
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

fn native_error(error: &[c_char]) -> io::Error {
    // All native error paths terminate the caller-owned buffer.
    invalid(unsafe { CStr::from_ptr(error.as_ptr()) }.to_string_lossy())
}

fn c_path(path: &Path) -> io::Result<CString> {
    CString::new(
        path.to_str()
            .ok_or_else(|| invalid("CAD path must be UTF-8"))?,
    )
    .map_err(|_| invalid("CAD path contains NUL"))
}

struct Native(NonNull<c_void>);
// The handle owns its OCCT objects and may move between threads. Every access
// (including destruction of the last Arc) is serialized by the model mutex.
unsafe impl Send for Native {}
impl Drop for Native {
    fn drop(&mut self) {
        unsafe { simrep_cad_free(self.0.as_ptr()) };
    }
}

/// OCCT exchange settings are process-global, including fixture writers.
static EXCHANGE: Mutex<()> = Mutex::new(());

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    /// Use B-rep distances for valid solids, tessellation for invalid B-reps.
    #[default]
    Auto,
    Exact,
    Tessellated,
}

#[derive(Clone, Copy, Debug)]
pub struct ImportOptions {
    pub mode: Mode,
    /// Positive physical half-band, normally twice the largest grid spacing.
    /// Distances are exact inside this band and saturated outside it.
    pub band_mm: f64,
    /// Absolute (not relative) OCCT chordal deflection, in millimeters.
    pub chordal_tolerance_mm: f64,
    /// Override the declared source length unit; None honors the CAD file.
    pub units: Option<Unit>,
}

impl Default for ImportOptions {
    fn default() -> Self {
        Self {
            mode: Mode::Auto,
            band_mm: 2.0,
            chordal_tolerance_mm: 0.01,
            units: None,
        }
    }
}

enum Shape {
    Exact {
        model: Arc<Mutex<Native>>,
        index: usize,
        bounds: [f64; 6],
    },
    Mesh(Solid),
}

pub struct CadPart {
    pub name: String,
    pub mode: Mode,
    pub triangle_count: usize,
    pub bounds_mm: [Point; 2],
    /// Closed fine tessellation at the requested chordal tolerance. Portable
    /// flight sidecars retain this even when refinement uses exact B-rep SDFs.
    pub tessellation: Part,
    band: f64,
    shape: Shape,
}

impl CadPart {
    pub fn try_distance(&self, point: Point) -> io::Result<f64> {
        if point.iter().any(|x| !x.is_finite()) {
            return Err(invalid("CAD query coordinates must be finite"));
        }
        let distance = match &self.shape {
            Shape::Exact {
                model,
                index,
                bounds,
            } => {
                let lower_bound = (0..3)
                    .map(|a| {
                        (bounds[a] - point[a])
                            .max(point[a] - bounds[a + 3])
                            .max(0.0)
                            .powi(2)
                    })
                    .sum::<f64>()
                    .sqrt();
                if lower_bound >= self.band {
                    return Ok(self.band);
                }
                let model = model.lock().unwrap();
                unsafe { simrep_cad_distance(model.0.as_ptr(), *index, point.as_ptr()) }
            }
            Shape::Mesh(solid) => solid.signed_distance(point),
        };
        if distance.is_finite() {
            Ok(distance.clamp(-self.band, self.band))
        } else {
            Err(invalid(format!(
                "OCCT distance/classification failed for {:?} at {point:?}; use --cad-mode tessellated",
                self.name
            )))
        }
    }
}

impl SignedDistance for CadPart {
    fn distance(&self, point: Point) -> f64 {
        // The interchange trait is infallible. CutCells rejects nonfinite nodal
        // distances; fallible callers can retain the detailed error above.
        self.try_distance(point).unwrap_or(f64::NAN)
    }

    fn segment_clear(&self, from: Point, to: Point, tolerance: f64) -> bool {
        match &self.shape {
            Shape::Mesh(solid) => SignedDistance::segment_clear(solid, from, to, tolerance),
            Shape::Exact { bounds, .. } => segment_box_gap(from, to, bounds) > tolerance,
        }
    }

    fn conductor_mask(
        &self,
        dims: [usize; 3],
        origin: Point,
        spacing: Point,
        threshold: f64,
        mask: &mut [bool],
    ) -> Result<(), &'static str> {
        match &self.shape {
            Shape::Mesh(solid) => solid.conductor_mask(dims, origin, spacing, threshold, mask),
            Shape::Exact { .. } => {
                let mut bad = false;
                for (i, flag) in mask.iter_mut().enumerate() {
                    let c = [i % dims[0], i / dims[0] % dims[1], i / (dims[0] * dims[1])];
                    let point = std::array::from_fn(|a| origin[a] + c[a] as f64 * spacing[a]);
                    let distance = self.distance(point);
                    if !distance.is_finite() {
                        bad = true;
                    }
                    *flag = distance <= threshold;
                }
                if bad {
                    Err("SDF returned a non-finite distance")
                } else {
                    Ok(())
                }
            }
        }
    }

    fn mark_near_nodes(
        &self,
        dims: [usize; 3],
        origin: Point,
        spacing: Point,
        band: f64,
        near: &mut [bool],
    ) {
        match &self.shape {
            Shape::Mesh(solid) => solid.mark_near_nodes(dims, origin, spacing, band, near),
            Shape::Exact { .. } => near.fill(true),
        }
    }
}

fn segment_box_gap(from: Point, to: Point, bounds: &[f64; 6]) -> f64 {
    let mut gap2 = 0.0;
    for a in 0..3 {
        let lo = from[a].min(to[a]);
        let hi = from[a].max(to[a]);
        let gap = (bounds[a] - hi).max(0.0).max(lo - bounds[a + 3]);
        gap2 += gap * gap;
    }
    gap2.sqrt()
}

/// Import each located XCAF leaf solid separately, retaining product/body names.
pub fn read_cad(path: impl AsRef<Path>, options: ImportOptions) -> io::Result<Vec<CadPart>> {
    if !options.band_mm.is_finite()
        || options.band_mm <= 0.0
        || !options.chordal_tolerance_mm.is_finite()
        || options.chordal_tolerance_mm <= 0.0
    {
        return Err(invalid(
            "CAD band and chordal tolerance must be finite and positive",
        ));
    }
    let path = path.as_ref();
    let extension = path.extension().and_then(|s| s.to_str()).unwrap_or("");
    let iges = match extension.to_ascii_lowercase().as_str() {
        "step" | "stp" => false,
        "iges" | "igs" => true,
        _ => return Err(invalid("CAD input must be .step, .stp, .iges, or .igs")),
    };
    let path = c_path(path)?;
    let mut error = [0; 1024];
    let read_started = Instant::now();
    let pointer = {
        let _exchange = EXCHANGE.lock().unwrap();
        unsafe {
            simrep_cad_read(
                path.as_ptr(),
                iges,
                options.units.map_or(0.0, Unit::millimeters),
                error.as_mut_ptr(),
                error.len(),
            )
        }
    };
    let read_ms = read_started.elapsed().as_secs_f64() * 1000.0;
    let model = Arc::new(Mutex::new(Native(
        NonNull::new(pointer).ok_or_else(|| native_error(&error))?,
    )));
    let count = unsafe { simrep_cad_count(pointer) };
    struct Meta {
        name: String,
        bounds: [f64; 6],
        tessellated: bool,
        tshape: usize,
    }
    let mut meta = Vec::with_capacity(count);
    for index in 0..count {
        let name = unsafe { CStr::from_ptr(simrep_cad_name(pointer, index)) }
            .to_string_lossy()
            .into_owned();
        let valid = unsafe { simrep_cad_valid(pointer, index) };
        let mut bounds = [0.0; 6];
        unsafe { simrep_cad_bounds(pointer, index, bounds.as_mut_ptr()) };
        let tessellated =
            options.mode == Mode::Tessellated || (!valid && options.mode == Mode::Auto);
        if !valid && options.mode == Mode::Exact {
            return Err(invalid(format!(
                "invalid OCCT B-rep {name:?}; use --cad-mode tessellated"
            )));
        }
        let tshape = unsafe { simrep_cad_tshape(pointer, index) };
        meta.push(Meta {
            name,
            bounds,
            tessellated,
            tshape,
        });
    }
    let unique_shapes = meta
        .iter()
        .map(|m| m.tshape)
        .collect::<std::collections::HashSet<_>>()
        .len()
        == meta.len();
    let tessellate_started = Instant::now();
    // Distinct TShapes do not share OCCT triangulation storage. The pointer is
    // only used to mesh those independent solids.
    struct SendModel(*mut c_void);
    // The model outlives this function's parallel section and each index touches
    // a distinct TShape. Sharing the pointer across Rayon workers is intentional.
    unsafe impl Send for SendModel {}
    unsafe impl Sync for SendModel {}
    impl SendModel {
        fn get(&self) -> *mut c_void {
            self.0
        }
    }
    let model_ptr = SendModel(pointer);
    let meshes = if unique_shapes && count > 1 {
        (0..count)
            .into_par_iter()
            .map(|index| {
                tessellate_part(
                    model_ptr.get(),
                    index,
                    options.chordal_tolerance_mm,
                    meta[index].name.clone(),
                )
            })
            .collect::<io::Result<Vec<_>>>()?
    } else {
        (0..count)
            .map(|index| {
                tessellate_part(
                    pointer,
                    index,
                    options.chordal_tolerance_mm,
                    meta[index].name.clone(),
                )
            })
            .collect::<io::Result<Vec<_>>>()?
    };
    let tessellate_ms = tessellate_started.elapsed().as_secs_f64() * 1000.0;
    let solid_started = Instant::now();
    let parts = meshes
        .into_par_iter()
        .zip(meta)
        .map(|(mut part, item)| {
            weld_and_orient(&mut part)?;
            let triangle_count = part.triangles.len();
            let solid = Solid::new(&part)?;
            let shape = if item.tessellated {
                Shape::Mesh(solid)
            } else {
                Shape::Exact {
                    model: Arc::clone(&model),
                    index: 0, // replaced below; index is recovered from enumerate
                    bounds: item.bounds,
                }
            };
            Ok(CadPart {
                name: item.name,
                mode: if item.tessellated {
                    Mode::Tessellated
                } else {
                    Mode::Exact
                },
                triangle_count,
                bounds_mm: [
                    [item.bounds[0], item.bounds[1], item.bounds[2]],
                    [item.bounds[3], item.bounds[4], item.bounds[5]],
                ],
                tessellation: part,
                band: options.band_mm,
                shape,
            })
        })
        .collect::<io::Result<Vec<_>>>()?;
    // Exact-mode bodies keep their import index. The parallel map above cannot
    // see the original index after zip consumes meta, so stamp it here.
    let mut parts = parts;
    for (index, part) in parts.iter_mut().enumerate() {
        if let Shape::Exact { index: slot, .. } = &mut part.shape {
            *slot = index;
        }
    }
    let solid_ms = solid_started.elapsed().as_secs_f64() * 1000.0;
    eprintln!(
        "cad_import_ms: read={read_ms:.3}, tessellate={tessellate_ms:.3}, solid={solid_ms:.3}, parts={count}, parallel_tessellation={}",
        unique_shapes && count > 1
    );
    Ok(parts)
}

fn tessellate_part(
    model: *mut c_void,
    index: usize,
    tolerance: f64,
    name: String,
) -> io::Result<Part> {
    let mut error = [0; 1024];
    let triangles = unsafe {
        simrep_cad_tessellate(
            model,
            index,
            tolerance,
            true,
            error.as_mut_ptr(),
            error.len(),
        )
    };
    let triangles = NonNull::new(triangles).ok_or_else(|| native_error(&error))?;
    let data = unsafe {
        std::slice::from_raw_parts(
            simrep_cad_triangle_data(triangles.as_ptr()),
            simrep_cad_triangle_values(triangles.as_ptr()),
        )
    };
    let part = Part {
        name,
        vertices: data.as_chunks::<3>().0.to_vec(),
        triangles: (0..data.len() / 9)
            .map(|i| [3 * i, 3 * i + 1, 3 * i + 2])
            .collect(),
    };
    unsafe { simrep_cad_triangles_free(triangles.as_ptr()) };
    Ok(part)
}

/// Whether OCCT's parallel face mesher reproduces the serial triangulation.
pub fn parallel_mesh_matches(path: impl AsRef<Path>, tolerance_mm: f64) -> io::Result<bool> {
    let parts_path = path.as_ref();
    let extension = parts_path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("");
    let iges = matches!(extension.to_ascii_lowercase().as_str(), "iges" | "igs");
    let cpath = c_path(parts_path)?;
    let mut error = [0; 1024];
    let _exchange = EXCHANGE.lock().unwrap();
    let pointer =
        unsafe { simrep_cad_read(cpath.as_ptr(), iges, 0.0, error.as_mut_ptr(), error.len()) };
    let pointer = NonNull::new(pointer).ok_or_else(|| native_error(&error))?;
    let matched = unsafe {
        simrep_cad_parallel_tessellation_matches(
            pointer.as_ptr(),
            0,
            tolerance_mm,
            error.as_mut_ptr(),
            error.len(),
        )
    };
    unsafe { simrep_cad_free(pointer.as_ptr()) };
    if matched {
        Ok(true)
    } else if error[0] != 0 {
        Err(native_error(&error))
    } else {
        Ok(false)
    }
}

/// Generate small original exchange fixtures, not externally sourced CAD files.
/// Shape 0 = radius-0.31 sphere, 1 = radius-0.31 capped z-cylinder (z=-2..2),
/// 2 = torus (major radius 0.55, minor radius 0.19),
/// 3 = assembly of two sphere instances centered at x=-0.6/+0.6 mm.
pub fn write_fixture(
    path: impl AsRef<Path>,
    shape: u8,
    schema: &str,
    units: &str,
) -> io::Result<()> {
    if shape > 3
        || !["AP203", "AP214IS", "AP242DIS"].contains(&schema)
        || !["MM", "M", "INCH"].contains(&units)
    {
        return Err(invalid("unsupported CAD fixture shape/schema/units"));
    }
    let path = c_path(path.as_ref())?;
    let schema = CString::new(schema).unwrap();
    let units = CString::new(units).unwrap();
    let mut error = [0; 1024];
    let _exchange = EXCHANGE.lock().unwrap();
    if unsafe {
        simrep_cad_fixture(
            path.as_ptr(),
            i32::from(shape),
            schema.as_ptr(),
            units.as_ptr(),
            error.as_mut_ptr(),
            error.len(),
        )
    } {
        Ok(())
    } else {
        Err(native_error(&error))
    }
}
