use std::ffi::{CStr, CString, c_char, c_void};
use std::io;
use std::path::Path;
use std::ptr::NonNull;
use std::sync::{Arc, Mutex};

use simrep_geom::{Point, SignedDistance};
use simrep_mesh::{Part, Solid, Unit, repair};

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
    fn simrep_cad_bounds(model: *mut c_void, index: usize, bounds: *mut f64);
    fn simrep_cad_distance(model: *mut c_void, index: usize, point: *const f64) -> f64;
    fn simrep_cad_tessellate(
        model: *mut c_void,
        index: usize,
        tolerance: f64,
        error: *mut c_char,
        size: usize,
    ) -> *mut c_void;
    fn simrep_cad_triangle_values(triangles: *mut c_void) -> usize;
    fn simrep_cad_triangle_data(triangles: *mut c_void) -> *const f64;
    fn simrep_cad_triangles_free(triangles: *mut c_void);
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
    let _exchange = EXCHANGE.lock().unwrap();
    let pointer = unsafe {
        simrep_cad_read(
            path.as_ptr(),
            iges,
            options.units.map_or(0.0, Unit::millimeters),
            error.as_mut_ptr(),
            error.len(),
        )
    };
    let model = Arc::new(Mutex::new(Native(
        NonNull::new(pointer).ok_or_else(|| native_error(&error))?,
    )));
    let native = model.lock().unwrap();
    let count = unsafe { simrep_cad_count(pointer) };
    let mut parts = Vec::with_capacity(count);
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
        let part = {
            let triangles = unsafe {
                simrep_cad_tessellate(
                    pointer,
                    index,
                    options.chordal_tolerance_mm,
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
            let mut part = Part {
                name: name.clone(),
                vertices: data.chunks_exact(3).map(|p| [p[0], p[1], p[2]]).collect(),
                triangles: (0..data.len() / 9)
                    .map(|i| [3 * i, 3 * i + 1, 3 * i + 2])
                    .collect(),
            };
            unsafe { simrep_cad_triangles_free(triangles.as_ptr()) };
            repair(&mut part)?;
            part
        };
        let triangle_count = part.triangles.len();
        let solid = Solid::new(&part)?;
        let shape = if tessellated {
            Shape::Mesh(solid)
        } else {
            Shape::Exact {
                model: Arc::clone(&model),
                index,
                bounds,
            }
        };
        parts.push(CadPart {
            name,
            mode: if tessellated {
                Mode::Tessellated
            } else {
                Mode::Exact
            },
            triangle_count,
            bounds_mm: [
                [bounds[0], bounds[1], bounds[2]],
                [bounds[3], bounds[4], bounds[5]],
            ],
            tessellation: part,
            band: options.band_mm,
            shape,
        });
    }
    drop(native);
    Ok(parts)
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
