//! Arrays the UI has open, kept native-side so refine / fast adjust / flying never re-send them.
//!
//! Each scene object (id chosen by the UI) has one `Session`: the file as read, its fast-adjust
//! solutions (.pa0 + .pa1..N) when present, and a cache of the last voltage set's superposed array
//! and interpolation field, so moving a slider and re-flying share one superposition.
//!
//! A refined array with a sibling `<stem>.cuts.json` (import wizard cut-cell geometry) also flies
//! against those exact electrode surfaces (`Session::exact`, the same sidecar `simrep fly` picks up).

use std::collections::{HashMap, VecDeque};
use std::fs::File;
use std::io::{self, BufReader};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::SystemTime;

use simrep_core::PotentialArray;
use simrep_fly::surface::{SurfaceAccuracy, SurfaceModel};
use simrep_fly::{GridField, InstanceField, Particle, Vector};
use simrep_io::{FastScalable, PaFile, read_pa};
use simrep_solve::gpu::Gpu;

use crate::fly::{Collision, Placement};
use crate::live::{BenchKey, Fields, FlyDevice, Slot};
use crate::pa::{FastElectrode, FastStatus, PaError, Reason, file_name};

#[derive(Clone, Default)]
pub struct AppState {
    sessions: Arc<Mutex<HashMap<String, Arc<Session>>>>,
    /// Cancel flags of running refine jobs.
    pub jobs: Arc<Mutex<HashMap<u32, Arc<AtomicBool>>>>,
    pub next_job: Arc<AtomicU32>,
    /// Vulkan device for the GPU multigrid, created in the background at start-up (main.rs).
    pub gpu: Arc<OnceLock<Result<Arc<Gpu>, crate::pa::Reason>>>,
    /// Vulkan flyer device, created in the background at start-up and kept (live.rs).
    pub fly_gpu: Arc<OnceLock<Result<FlyDevice, crate::pa::Reason>>>,
    /// Cut-cell geometry of `.pa#` files built by the import wizard (geometry.rs), by path.
    pub geometries: Arc<Mutex<HashMap<PathBuf, Arc<crate::geometry::Prepared>>>>,
}

impl AppState {
    pub fn insert(&self, id: String, session: Arc<Session>) {
        self.sessions.lock().unwrap().insert(id, session);
    }

    pub fn remove(&self, id: &str) {
        self.sessions.lock().unwrap().remove(id);
    }

    pub fn get(&self, id: &str) -> Option<Arc<Session>> {
        self.sessions.lock().unwrap().get(id).cloned()
    }
}

/// One fast-adjust solution: the potential with its electrode at the reference voltage.
pub struct Basis {
    pub number: usize,
    pub potentials: Vec<f64>,
    /// Voltage the solution represents (PA0 reference, or the solution's peak for PA_ / scalable).
    reference: f64,
    /// Voltage this electrode has in the PA0 base.
    current: f64,
    /// Largest |electrode potential| in the solution, to recognize the electrode's own points.
    peak: f64,
}

/// SIMION fast adjustment, held in memory. Same rules as `simrep_io::read_fast_adjust`: voltage
/// changes are superposed on the PA0 base, `(requested - current) / reference * solution`.
pub struct FastAdjust {
    pub bases: Vec<Basis>,
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

fn peak_electrode(potentials: &[f64], electrodes: &[bool]) -> Option<(usize, f64)> {
    potentials
        .iter()
        .enumerate()
        .filter(|&(i, _)| electrodes[i])
        .max_by(|(_, a), (_, b)| a.abs().total_cmp(&b.abs()))
        .filter(|&(_, &v)| v != 0.0)
        .map(|(i, &v)| (i, v))
}

fn same_geometry(a: &PotentialArray, b: &PotentialArray) -> bool {
    (
        a.nx,
        a.ny,
        a.nz,
        a.symmetry,
        a.mirrors,
        a.anti_mirrors,
        a.field_type,
    ) == (
        b.nx,
        b.ny,
        b.nz,
        b.symmetry,
        b.mirrors,
        b.anti_mirrors,
        b.field_type,
    ) && [a.dx_mm, a.dy_mm, a.dz_mm] == [b.dx_mm, b.dy_mm, b.dz_mm]
        && a.electrodes == b.electrodes
}

impl FastAdjust {
    /// Reads the sibling solutions of a .pa0 (`.pa1`, `.pa_`, ... with the PA0's extension case).
    pub fn load(path: &Path, base: &PaFile) -> io::Result<Self> {
        let metadata = base
            .pa0
            .as_ref()
            .ok_or_else(|| invalid("no PA0 electrode table"))?;
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("pa0");
        let prefix = &ext[..ext.len().min(2)];
        let mut solutions = Vec::new();
        for (number, &index) in metadata.electrode_indices.iter().enumerate() {
            if index == -1 {
                continue;
            }
            let suffix = if number == 0 {
                "_".to_owned()
            } else {
                number.to_string()
            };
            let sibling = path.with_extension(format!("{prefix}{suffix}"));
            let solution = read_pa(&sibling).map_err(|e| {
                io::Error::new(e.kind(), format!("{}: {e}", crate::pa::file_name(&sibling)))
            })?;
            if !same_geometry(&base.array, &solution.array) {
                return Err(invalid(format!(
                    "{}: grid differs from the .pa0",
                    crate::pa::file_name(&sibling)
                )));
            }
            solutions.push((number, solution.array.potentials));
        }
        Self::new(base, solutions)
    }

    /// `solutions` are (electrode number, potentials) with the PA0's geometry.
    pub fn new(base: &PaFile, solutions: Vec<(usize, Vec<f64>)>) -> io::Result<Self> {
        let metadata = base
            .pa0
            .as_ref()
            .ok_or_else(|| invalid("no PA0 electrode table"))?;
        if metadata.reference_voltage <= 0.0 {
            return Err(invalid("PA0 reference_voltage must be positive"));
        }
        let mut bases = Vec::new();
        for (number, potentials) in solutions {
            let (peak_index, peak) = peak_electrode(&potentials, &base.array.electrodes)
                .ok_or_else(|| invalid(format!("solution {number} has no nonzero electrode")))?;
            let (current, reference) =
                if let Some(FastScalable::Voltages(values)) = &metadata.scalable {
                    (values.get(number).copied().unwrap_or(0.0), peak)
                } else if number == 0 {
                    (base.array.potentials[peak_index], peak)
                } else {
                    let index = usize::try_from(metadata.electrode_indices[number])
                        .map_err(|_| invalid("bad PA0 electrode index"))?;
                    (base.array.potentials[index], metadata.reference_voltage)
                };
            bases.push(Basis {
                number,
                potentials,
                reference,
                current,
                peak: peak.abs(),
            });
        }
        if bases.is_empty() {
            return Err(invalid("the .pa0 lists no adjustable electrodes"));
        }
        Ok(Self { bases })
    }

    pub fn status(&self) -> FastStatus {
        self.status_at(&[])
    }

    /// Electrode voltages with `voltages` (number, V) applied over the PA0's own.
    pub fn status_at(&self, voltages: &[(usize, f64)]) -> FastStatus {
        let voltage = |b: &Basis| {
            voltages
                .iter()
                .find(|v| v.0 == b.number)
                .map_or(b.current, |v| v.1)
        };
        FastStatus::Ready {
            electrodes: self
                .bases
                .iter()
                .map(|b| FastElectrode {
                    number: b.number,
                    voltage: voltage(b),
                })
                .collect(),
        }
    }

    /// PA0 base with `voltages` (electrode number, V) applied; unlisted electrodes keep theirs.
    pub fn superpose(
        &self,
        base: &PotentialArray,
        voltages: &[(usize, f64)],
    ) -> io::Result<PotentialArray> {
        let mut out = base.clone();
        for &(number, voltage) in voltages {
            let basis = self
                .bases
                .iter()
                .find(|b| b.number == number)
                .ok_or_else(|| invalid(format!("electrode {number} is not adjustable")))?;
            let delta = (voltage - basis.current) / basis.reference;
            if !delta.is_finite() {
                return Err(invalid("non-finite fast-adjust voltage"));
            }
            if delta != 0.0 {
                out.potentials
                    .iter_mut()
                    .zip(&basis.potentials)
                    .for_each(|(p, &b)| *p += delta * b);
            }
        }
        Ok(out)
    }

    /// Unit-voltage solutions (solution / reference), in `bases` order: the GPU flyer's bases.
    pub fn unit_bases(&self) -> impl Iterator<Item = Vec<f64>> + '_ {
        self.bases
            .iter()
            .map(|b| b.potentials.iter().map(|v| v / b.reference).collect())
    }

    /// Volts to add to each unit basis (`bases` order) for `voltages`; unlisted electrodes add 0.
    pub fn deltas(&self, voltages: &[(usize, f64)]) -> io::Result<Vec<f64>> {
        if let Some(&(number, _)) = voltages
            .iter()
            .find(|v| !self.bases.iter().any(|b| b.number == v.0))
        {
            return Err(invalid(format!("electrode {number} is not adjustable")));
        }
        let deltas: Vec<f64> = self
            .bases
            .iter()
            .map(|b| {
                voltages
                    .iter()
                    .find(|v| v.0 == b.number)
                    .map_or(0.0, |v| v.1 - b.current)
            })
            .collect();
        if deltas.iter().any(|d| !d.is_finite()) {
            return Err(invalid("non-finite fast-adjust voltage"));
        }
        Ok(deltas)
    }

    /// Adjustable electrode a grid point belongs to (its own solution is at full voltage there).
    pub fn electrode_number(&self, index: usize) -> Option<usize> {
        self.bases
            .iter()
            .filter(|b| b.number > 0)
            .find(|b| {
                b.potentials
                    .get(index)
                    .is_some_and(|v| v.abs() >= 0.5 * b.peak)
            })
            .map(|b| b.number)
    }
}

/// The `<stem>.cuts.json` next to a refined array (not a .pa#), as `simrep fly` picks it up.
pub fn sidecar(path: &Path) -> Option<PathBuf> {
    let ext = path.extension()?.to_str()?;
    if ext.eq_ignore_ascii_case("pa#") {
        return None;
    }
    let side = path.with_extension("cuts.json");
    side.is_file().then_some(side)
}

/// The array at one voltage set, placed on the workbench, with the sidecar's exact electrode
/// surfaces replacing the grid's electrode points for collisions (the field is unchanged).
#[derive(Clone)]
pub struct Exact {
    pub field: Arc<InstanceField>,
    pub accuracy: SurfaceAccuracy,
    pub file: String,
}

/// A loaded sidecar: file modification time and root tolerance (f64 bits) it was loaded for.
struct Surface {
    key: (Option<SystemTime>, u64),
    model: Arc<SurfaceModel>,
}

struct Live {
    voltages: Vec<(usize, f64)>,
    array: Arc<PotentialArray>,
    field: Option<(Collision, Arc<GridField>)>,
}

pub struct Session {
    pub path: PathBuf,
    pub file: PaFile,
    pub fast: Option<FastAdjust>,
    live: Mutex<Option<Live>>,
    /// Last ion source (JSON key) and its particles, so re-flying after a slider move reuses them.
    pub particles: Mutex<Option<(String, Arc<Vec<Particle>>)>>,
    /// Field resident on the flyer device (prepared on the first GPU flight).
    pub gpu: Mutex<Option<Slot>>,
    /// CPU side of the resident field and its workbench box: single-ion traces read it without
    /// waiting for `gpu`, which a GPU flight holds while it runs.
    pub live_fields: Mutex<Option<(BenchKey, Arc<Fields>)>>,
    /// Final velocities of the last few flights by token (f64 vx, vy, vz per ion), which flight
    /// packets leave out (`fly::flight_velocities`).
    pub velocities: Mutex<VecDeque<(u64, Arc<Vec<f64>>)>>,
    /// The sibling sidecar, loaded by the first exact-surface flight.
    surface: Mutex<Option<Surface>>,
    /// Exact-surface field of the last voltage set / workbench box / placement flown.
    exact: Mutex<Option<(String, Exact)>>,
}

impl Session {
    pub fn new(path: PathBuf, file: PaFile, fast: Option<FastAdjust>) -> Self {
        Self {
            path,
            file,
            fast,
            live: Mutex::new(None),
            particles: Mutex::new(None),
            gpu: Mutex::new(None),
            live_fields: Mutex::new(None),
            velocities: Mutex::new(VecDeque::new()),
            surface: Mutex::new(None),
            exact: Mutex::new(None),
        }
    }

    /// The field with exact electrode surfaces at `voltages`, inside `bounds`, placed by `place`;
    /// None when the array has no sidecar. `tolerance_mm` is the flight's position tolerance
    /// (workbench mm): surface roots are localized to it in source mm, as `simrep fly` does.
    /// A sidecar that does not load or does not match the array is an error, never ignored.
    pub fn exact(
        &self,
        voltages: &[(usize, f64)],
        bounds: Option<[Vector; 2]>,
        place: &Placement,
        tolerance_mm: f64,
    ) -> Result<Option<Exact>, PaError> {
        let Some(path) = sidecar(&self.path) else {
            return Ok(None);
        };
        let name = file_name(&path);
        let invalid = |e: &dyn std::fmt::Display| {
            PaError::coded(
                "fly",
                Reason::new("cutsInvalid").with("file", &name).detail(e),
            )
        };
        let modified = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
        let tolerance = tolerance_mm / place.scale;
        let model = {
            let mut cache = self.surface.lock().unwrap();
            let key = (modified, tolerance.to_bits());
            match cache.as_ref().filter(|s| s.key == key) {
                Some(s) => s.model.clone(),
                None => {
                    let reader = BufReader::new(File::open(&path).map_err(|e| invalid(&e))?);
                    let model = SurfaceModel::from_reader(reader)
                        .map_err(|e| invalid(&e))?
                        .with_tolerance(tolerance)
                        .map_err(|e| invalid(&e))?;
                    let model = Arc::new(model);
                    *cache = Some(Surface {
                        key,
                        model: model.clone(),
                    });
                    model
                }
            }
        };
        let fly_err = |e: io::Error| PaError::new("fly", e.to_string());
        let voltages = self.normalize(voltages).map_err(fly_err)?;
        let key = format!("{modified:?} {tolerance:e} {voltages:?} {bounds:?} {place:?}");
        if let Some((_, exact)) = self
            .exact
            .lock()
            .unwrap()
            .as_ref()
            .filter(|(k, _)| *k == key)
        {
            return Ok(Some(exact.clone()));
        }
        let array = self.array(&voltages).map_err(fly_err)?;
        let grid =
            GridField::new((*array).clone()).map_err(|e| PaError::new("fly", e.to_string()))?;
        let field = InstanceField::new(grid, place.origin, place.axes, place.scale, bounds, None)
            .map_err(|e| PaError::new("fly", e.to_string()))?
            .with_surface(model.clone())
            .map_err(|e| invalid(&e))?;
        let exact = Exact {
            field: Arc::new(field),
            accuracy: model.accuracy(),
            file: name.clone(),
        };
        *self.exact.lock().unwrap() = Some((key, exact.clone()));
        Ok(Some(exact))
    }

    pub fn is_pa_hash(&self) -> bool {
        self.path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("pa#"))
    }

    fn normalize(&self, voltages: &[(usize, f64)]) -> io::Result<Vec<(usize, f64)>> {
        if voltages.is_empty() {
            return Ok(Vec::new());
        }
        if self.fast.is_none() {
            return Err(invalid("this array has no fast-adjust solutions"));
        }
        let mut v = voltages.to_vec();
        v.sort_by_key(|&(n, _)| n);
        v.dedup_by_key(|&mut (n, _)| n);
        Ok(v)
    }

    /// Array at `voltages` (empty = as stored), cached for the next call with the same voltages.
    pub fn array(&self, voltages: &[(usize, f64)]) -> io::Result<Arc<PotentialArray>> {
        let key = self.normalize(voltages)?;
        let mut live = self.live.lock().unwrap();
        if let Some(l) = live.as_ref().filter(|l| l.voltages == key) {
            return Ok(l.array.clone());
        }
        let array = Arc::new(match &self.fast {
            Some(fast) if !key.is_empty() => fast.superpose(&self.file.array, &key)?,
            _ => self.file.array.clone(),
        });
        *live = Some(Live {
            voltages: key,
            array: array.clone(),
            field: None,
        });
        Ok(array)
    }

    /// Interpolation field at `voltages` with the `collision` rule, built once per voltage set.
    pub fn field(
        &self,
        voltages: &[(usize, f64)],
        collision: Collision,
    ) -> Result<Arc<GridField>, String> {
        let array = self.array(voltages).map_err(|e| e.to_string())?;
        let key = self.normalize(voltages).map_err(|e| e.to_string())?;
        let cached = self
            .live
            .lock()
            .unwrap()
            .as_ref()
            .filter(|l| l.voltages == key)
            .and_then(|l| l.field.clone());
        if let Some((_, field)) = cached.filter(|(c, _)| *c == collision) {
            return Ok(field);
        }
        let field =
            Arc::new(collision.apply(GridField::new((*array).clone()).map_err(|e| e.to_string())?));
        if let Some(l) = self
            .live
            .lock()
            .unwrap()
            .as_mut()
            .filter(|l| l.voltages == key)
        {
            l.field = Some((collision, field.clone()));
        }
        Ok(field)
    }
}
