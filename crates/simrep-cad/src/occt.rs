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
    fn simrep_cad_read_measurement(
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
    fn simrep_cad_face_count(model: *mut c_void, index: usize) -> usize;
    fn simrep_cad_measure(
        model: *mut c_void,
        a: usize,
        faces_a: *const usize,
        count_a: usize,
        b: usize,
        faces_b: *const usize,
        count_b: usize,
        values: *mut f64,
        closest_faces: *mut usize,
        error: *mut c_char,
        size: usize,
    ) -> bool;
    fn simrep_cad_tessellate_faces(
        model: *mut c_void,
        index: usize,
        faces: *const usize,
        count: usize,
        tolerance: f64,
        error: *mut c_char,
        size: usize,
    ) -> *mut c_void;
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

// Original implementation of Iteration-CFD's documented complete-face contract,
// not copied/adapted code. Reference licences: Prosperity-3.0.0 / Apache-2.0;
// see licenses/THIRD_PARTY.md for primary source and NOTICE references.
/// Face numbering is zero-based OCCT face-map order within this imported file.
#[derive(Clone, Debug)]
pub struct MeasurementPart {
    pub index: usize,
    pub name: String,
    pub face_count: usize,
}

#[derive(Clone, Copy, Debug)]
pub struct MeasureOptions {
    /// Absolute allowed discrepancy between exact and tessellated minima (mm).
    pub cad_tolerance_mm: f64,
    /// Distance threshold only; not proof of electrical contact.
    pub touch_tolerance_mm: f64,
}

impl Default for MeasureOptions {
    fn default() -> Self {
        Self {
            cad_tolerance_mm: 0.01,
            touch_tolerance_mm: 1e-7,
        }
    }
}

impl MeasureOptions {
    fn validate(self) -> io::Result<()> {
        if !self.cad_tolerance_mm.is_finite()
            || self.cad_tolerance_mm <= 0.0
            || !self.touch_tolerance_mm.is_finite()
            || self.touch_tolerance_mm < 0.0
        {
            return Err(invalid(
                "CAD tolerance must be positive and touch tolerance nonnegative; both must be finite",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct GapMeasurement {
    pub part_a: usize,
    pub part_b: usize,
    pub faces_a: Vec<usize>,
    pub faces_b: Vec<usize>,
    pub min_distance_mm: f64,
    pub closest_points_mm: [Point; 2],
    pub closest_faces: [usize; 2],
    pub touching: bool,
    pub tessellated_distance_mm: f64,
    pub cross_check_error_mm: f64,
}

/// An owned placed B-rep model for guarded face-set measurements, without SDF
/// import, saturation, or an electrode grid. Access to OCCT is serialized.
pub struct MeasurementModel {
    native: Mutex<Native>,
    parts: Vec<MeasurementPart>,
}

impl MeasurementModel {
    pub fn parts(&self) -> &[MeasurementPart] {
        &self.parts
    }

    pub fn read(path: impl AsRef<Path>) -> io::Result<Self> {
        Self::read_with_units(path, None)
    }

    /// Same declared-unit override and placement scaling as [`read_cad`].
    pub fn read_with_units(path: impl AsRef<Path>, units: Option<Unit>) -> io::Result<Self> {
        let path = path.as_ref();
        let iges = match path
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_ascii_lowercase()
            .as_str()
        {
            "step" | "stp" => false,
            "iges" | "igs" => true,
            _ => return Err(invalid("CAD input must be .step, .stp, .iges, or .igs")),
        };
        let path = c_path(path)?;
        let mut error = [0; 1024];
        let pointer = {
            let _exchange = EXCHANGE.lock().unwrap();
            unsafe {
                simrep_cad_read_measurement(
                    path.as_ptr(),
                    iges,
                    units.map_or(0.0, Unit::millimeters),
                    error.as_mut_ptr(),
                    error.len(),
                )
            }
        };
        let native = Native(NonNull::new(pointer).ok_or_else(|| native_error(&error))?);
        let parts = (0..unsafe { simrep_cad_count(pointer) })
            .map(|index| MeasurementPart {
                index,
                name: unsafe { CStr::from_ptr(simrep_cad_name(pointer, index)) }
                    .to_string_lossy()
                    .into_owned(),
                face_count: unsafe { simrep_cad_face_count(pointer, index) },
            })
            .collect();
        Ok(Self {
            native: Mutex::new(native),
            parts,
        })
    }

    fn select(&self, selector: &str) -> io::Result<(usize, Vec<usize>)> {
        let (name, subset) = selector
            .rsplit_once(":faces=")
            .map_or((selector, None), |(n, f)| (n, Some(f)));
        let matches: Vec<_> = self
            .parts
            .iter()
            .filter(|p| p.name == name || p.index.to_string() == name)
            .collect();
        if matches.len() != 1 {
            return Err(invalid(format!(
                "CAD selector {name:?} must match exactly one part"
            )));
        }
        let part = matches[0];
        let mut faces = match subset {
            None => (0..part.face_count).collect(),
            Some(value) => value
                .split(',')
                .map(|s| {
                    if s.is_empty() || !s.bytes().all(|c| c.is_ascii_digit()) {
                        return Err(invalid(
                            "CAD face subset must be a nonempty list of zero-based indices",
                        ));
                    }
                    s.parse::<usize>()
                        .map_err(|_| invalid("CAD face index is out of range"))
                })
                .collect::<io::Result<Vec<_>>>()?,
        };
        faces.sort_unstable();
        if faces.is_empty() || faces.iter().any(|&i| i >= part.face_count) {
            return Err(invalid(format!(
                "CAD face subset is empty or out of range for part {}",
                part.index
            )));
        }
        if faces.windows(2).any(|pair| pair[0] == pair[1]) {
            return Err(invalid("duplicate CAD face index"));
        }
        Ok((part.index, faces))
    }

    /// Minimum over every selected trimmed face, including its edges/vertices.
    /// Omitting `:faces=...` selects the COMPLETE face set of the placed body.
    /// An independent triangle minimum must agree within cad_tolerance_mm.
    pub fn gap(&self, a: &str, b: &str, options: MeasureOptions) -> io::Result<GapMeasurement> {
        options.validate()?;
        let (part_a, faces_a) = self.select(a)?;
        let (part_b, faces_b) = self.select(b)?;
        self.measure(part_a, faces_a, part_b, faces_b, options)
    }

    fn measure(
        &self,
        part_a: usize,
        faces_a: Vec<usize>,
        part_b: usize,
        faces_b: Vec<usize>,
        options: MeasureOptions,
    ) -> io::Result<GapMeasurement> {
        let native = self.native.lock().unwrap();
        let pointer = native.0.as_ptr();
        let mut values = [0.0; 7];
        let mut closest_faces = [0; 2];
        let mut error = [0; 1024];
        if !unsafe {
            simrep_cad_measure(
                pointer,
                part_a,
                faces_a.as_ptr(),
                faces_a.len(),
                part_b,
                faces_b.as_ptr(),
                faces_b.len(),
                values.as_mut_ptr(),
                closest_faces.as_mut_ptr(),
                error.as_mut_ptr(),
                error.len(),
            )
        } {
            return Err(native_error(&error));
        }
        if values.iter().any(|x| !x.is_finite()) || values[0] < 0.0 {
            return Err(invalid(
                "CAD minimum or closest points are nonfinite or invalid",
            ));
        }
        // Each surface gets one quarter of the discrepancy budget. Do not
        // infer success from the requested deflection: measure the mesh itself.
        let mesh_a = measurement_mesh(pointer, part_a, &faces_a, options.cad_tolerance_mm / 4.0)?;
        let mesh_b = measurement_mesh(pointer, part_b, &faces_b, options.cad_tolerance_mm / 4.0)?;
        let tessellated_distance_mm = crate::measure_mesh::minimum(mesh_a, mesh_b);
        let cross_check_error_mm =
            check_discrepancy(values[0], tessellated_distance_mm, options.cad_tolerance_mm)?;
        Ok(GapMeasurement {
            part_a,
            part_b,
            faces_a,
            faces_b,
            min_distance_mm: values[0],
            closest_points_mm: [
                [values[1], values[2], values[3]],
                [values[4], values[5], values[6]],
            ],
            closest_faces,
            touching: values[0] <= options.touch_tolerance_mm,
            tessellated_distance_mm,
            cross_check_error_mm,
        })
    }

    /// Unique off-diagonal pairs, in body-index order.
    pub fn all_pairs(&self, options: MeasureOptions) -> io::Result<Vec<GapMeasurement>> {
        options.validate()?;
        let mut measurements = Vec::new();
        for a in 0..self.parts.len() {
            for b in a + 1..self.parts.len() {
                measurements.push(self.measure(
                    a,
                    (0..self.parts[a].face_count).collect(),
                    b,
                    (0..self.parts[b].face_count).collect(),
                    options,
                )?);
            }
        }
        Ok(measurements)
    }
}

fn check_discrepancy(exact: f64, tessellated: f64, tolerance: f64) -> io::Result<f64> {
    let error = (exact - tessellated).abs();
    if !error.is_finite() || error > tolerance {
        return Err(invalid(format!(
            "CAD tessellation cross-check disagrees: exact={exact} mm, tessellated={tessellated} mm, error={error} mm exceeds CAD tolerance={tolerance} mm"
        )));
    }
    Ok(error)
}

fn measurement_mesh(
    model: *mut c_void,
    part: usize,
    faces: &[usize],
    tolerance: f64,
) -> io::Result<Vec<[Point; 3]>> {
    let mut error = [0; 1024];
    let mesh = unsafe {
        simrep_cad_tessellate_faces(
            model,
            part,
            faces.as_ptr(),
            faces.len(),
            tolerance,
            error.as_mut_ptr(),
            error.len(),
        )
    };
    let mesh = NonNull::new(mesh).ok_or_else(|| native_error(&error))?;
    let data = unsafe {
        std::slice::from_raw_parts(
            simrep_cad_triangle_data(mesh.as_ptr()),
            simrep_cad_triangle_values(mesh.as_ptr()),
        )
    };
    let triangles: Vec<_> = data
        .as_chunks::<9>()
        .0
        .iter()
        .map(|t| [[t[0], t[1], t[2]], [t[3], t[4], t[5]], [t[6], t[7], t[8]]])
        .collect();
    unsafe { simrep_cad_triangles_free(mesh.as_ptr()) };
    if triangles.is_empty() || triangles.iter().flatten().flatten().any(|x| !x.is_finite()) {
        return Err(invalid(
            "CAD cross-check triangulation is empty or nonfinite",
        ));
    }
    Ok(triangles)
}

#[cfg(test)]
mod measurement_guards {
    use super::*;

    #[test]
    fn cross_check_rejects_disagreement_and_nonfinite_results() {
        assert!(check_discrepancy(0.58, 0.9, 0.01).is_err());
        assert!(check_discrepancy(0.58, f64::NAN, 0.01).is_err());
        assert!(check_discrepancy(0.58, f64::INFINITY, 0.01).is_err());
        assert!(check_discrepancy(0.58, 0.581, 0.01).unwrap() < 0.01);
    }

    #[test]
    fn all_pairs_uses_indices_even_when_a_name_is_numeric() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pair.step");
        write_fixture(&path, 3, "AP242DIS", "MM").unwrap();
        let mut model = MeasurementModel::read(&path).unwrap();
        model.parts[0].name = "1".into();
        assert!(model.gap("0", "1", MeasureOptions::default()).is_err());
        let pairs = model.all_pairs(MeasureOptions::default()).unwrap();
        assert_eq!(pairs.len(), 1);
        assert!((pairs[0].min_distance_mm - 0.58).abs() < 1e-9);
    }
}

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
/// 4 = touching radius-0.31 spheres centered at x=-0.31/+0.31 mm.
pub fn write_fixture(
    path: impl AsRef<Path>,
    shape: u8,
    schema: &str,
    units: &str,
) -> io::Result<()> {
    if shape > 4
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
