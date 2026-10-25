//! Fly ions through an open array and return trajectories + a summary.
//!
//! Two precision modes:
//! - **Live** (default): every ion flies on the app's Vulkan flyer (f32, see live.rs). The array's
//!   bases stay resident on the GPU, so a voltage change only rewrites a uniform. The same GPU
//!   flight samples the drawn subset's trajectories; if that fails they are flown on the CPU.
//!   Without a GPU (none, still starting, field too large) live mode falls back to the CPU and
//!   says why in the header.
//! - **Precise**: every ion flies on the CPU (f64) through the session's array fast-adjusted to the
//!   requested voltages (cached per voltage set, see session.rs).
//!
//! **Exact surfaces**: a refined array with a sibling `<stem>.cuts.json` (import wizard cut-cell
//! geometry) splats ions on those surfaces instead of its staircase electrode points (`surfaces`,
//! default on; `Session::exact`). Surface collisions run on the CPU only (`simrep fly` rules), so
//! live mode then flies on the CPU and says so (`exactSurfaces` fallback).
//!
//! The summary (surfaces, TOF, energy) always uses every ion; only `draw` ions (evenly spread)
//! carry trajectory points. Ions come
//! from a generated beam (deterministic, so re-flying after a slider move changes only what the
//! field changes) or from a SIMION .ion / literal .fly2 file. Workbench coordinates are the
//! array's own (grid point (0,0,0) at the origin, mm). An optional workbench box lets ions drift
//! field-free past the array, like SIMION's workbench bounds.
//!
//! Response (one binary packet, mirrored by `parseFlight` in src/scene/flight.ts):
//!
//! ```text
//! "STR3" | u32 header length | header JSON (space-padded to 4 bytes)
//!        | u32 drawn ions[drawn] (ascending indices) | u32 offsets[drawn + 1] (into points)
//!        | f32 points[3 * P] (x, y, z mm) | f32 ends[4 * ions] (x, y, z mm, TOF us)
//!        | f32 ke[ions] (final kinetic energy, eV)
//!        | u8 / u16 / u32 surface[ions] (index into header.surfaces; header.surfaceBytes wide)
//! ```
//!
//! Live re-flying is bounded by this packet's IPC (about 13 ms per MB) on fast fields, so it
//! carries only what the views draw: 21 B per ion with up to 256 end surfaces. The final
//! velocities (only the end-state CSV export uses them) stay here, under the header's `token`, for
//! `flight_velocities`.
//!
//! `ion_trace` re-flies one ion on the CPU (f64) through the same field and returns its full time
//! series (`parseTrace` in src/results/trace.ts):
//!
//! ```text
//! "SIT1" | u32 header length | header JSON (space-padded to 8 bytes)
//!        | f64 samples[9 * n] (t us, x, y, z mm, vx, vy, vz mm/us, kinetic energy eV, potential V)
//! ```

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use serde::{Deserialize, Serialize};
use simrep_core::Symmetry;
use simrep_fly::{ELECTRIC_ACCELERATION, Field, FieldSample, FlightOptions, FlightResult, GridField, Particle, SPEED_OF_LIGHT, SplatReason, Vector, fly as fly_one, fly_many};
use simrep_io::read_particles;
use tauri::State;
use tauri::ipc::Response;

use simrep_fly::gpu::TrajectorySampling;
use simrep_fly::surface::SurfaceAccuracy;

use crate::live;
use crate::pa::{PaError, Reason};
use crate::refine::{Device, cpu_device};
use crate::session::{AppState, Exact, Session};

const MAX_IONS: usize = 1_000_000;
/// Most trajectories drawn (each keeps every RK step on the CPU until thinned).
const MAX_DRAWN: usize = 10_000;
const DEFAULT_DRAWN: usize = 300;
/// Total trajectory points sent to the UI; long trajectories are thinned to fit.
const POINT_BUDGET: usize = 3_000_000;
const MAX_POINTS_PER_ION: usize = 1_500;

#[derive(Deserialize, Serialize, Clone, Copy, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
enum Precision {
    /// GPU (f32) for every ion when available; CPU otherwise.
    #[default]
    Live,
    /// CPU (f64) for every ion.
    Precise,
}

/// How PA electrode points absorb ions (`GridField` collision conventions, simrep-fly README).
#[derive(Deserialize, Serialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub enum Collision {
    /// SIMION: a grid cell is metal when all its corner points are electrode points.
    #[default]
    AllCorners,
    /// Each electrode point fills the half-cell box around it (earlier SimRep behaviour).
    NearestNode,
}

impl Collision {
    /// `field` with this rule (applied to every voltage basis alike).
    pub fn apply(self, field: GridField) -> GridField {
        match self {
            Self::AllCorners => field,
            Self::NearestNode => field.with_nearest_node_collisions(),
        }
    }
}

#[derive(Deserialize, Serialize, Clone, Copy)]
#[serde(rename_all = "camelCase")]
enum Shape {
    Point,
    Line,
    Disk,
}

#[derive(Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
enum Source {
    #[serde(rename_all = "camelCase")]
    Beam {
        count: usize,
        mass_amu: f64,
        charge_e: f64,
        energy_ev: f64,
        position_mm: Vector,
        direction: Vector,
        /// Half-angle of the direction cone, degrees (0 = all along `direction`).
        cone_deg: f64,
        shape: Shape,
        /// Line length or disk diameter, mm (perpendicular to `direction`).
        width_mm: f64,
    },
    File { path: String },
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FlyRequest {
    id: String,
    /// (electrode number, volts) for fast-adjust arrays; empty = the voltages stored in the file.
    voltages: Vec<(usize, f64)>,
    source: Source,
    /// xmin, ymin, zmin, xmax, ymax, zmax (mm): field-free workbench around the array.
    bounds: Option<[f64; 6]>,
    /// Where the array sits on the workbench (SIMION .iob instance); None = at the origin.
    #[serde(default)]
    placement: Option<Placement>,
    duration_us: f64,
    relativistic: bool,
    #[serde(default)]
    precision: Precision,
    /// Electrode collision rule (default: SIMION's all-corner cells).
    #[serde(default)]
    collision: Collision,
    /// Ions drawn as trajectories, evenly spread over all ions (the summary uses every ion).
    #[serde(default = "default_drawn")]
    draw: usize,
    /// Splat on the sibling `.cuts.json` surfaces when there is one (default on).
    #[serde(default = "default_surfaces")]
    surfaces: bool,
}

fn default_drawn() -> usize {
    DEFAULT_DRAWN
}

fn default_surfaces() -> bool {
    true
}

/// The exact surfaces a flight used (mirrored by `FlightHeader.surface` in src/scene/flight.ts).
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SurfaceInfo {
    file: String,
    /// "source": v3 source geometry (GEM CSG, mesh / CAD triangles); "sampled": v2 sampled
    /// distances; "approximate": v1 link cuts. Only "source" is exact.
    accuracy: &'static str,
}

impl SurfaceInfo {
    fn of(exact: &Exact) -> Self {
        let accuracy = match exact.accuracy {
            SurfaceAccuracy::SourceGeometry => "source",
            SurfaceAccuracy::SampledSdf => "sampled",
            SurfaceAccuracy::ApproximateV1 => "approximate",
        };
        Self { file: exact.file.clone(), accuracy }
    }
}

#[derive(Serialize, Clone, PartialEq)]
#[serde(tag = "kind", rename_all = "camelCase")]
enum Surface {
    /// Adjustable electrode number, or (plain arrays) the electrode's potential.
    Electrode { number: Option<usize>, voltage: Option<f64> },
    /// Workbench / array boundary face: "-x", "+x", ..., or "r" (cylinder radius).
    Wall { face: &'static str },
    /// Still flying when the time limit ran out.
    TimeLimit,
    /// The integrator gave up (step limit, step underflow, non-finite field).
    Stalled,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SurfaceCount {
    surface: Surface,
    count: usize,
    tof_mean: f64,
}

#[derive(Serialize, Default)]
struct Stats {
    n: usize,
    min: f64,
    max: f64,
    mean: f64,
    std: f64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Header {
    ions: usize,
    /// Ions with trajectory points (the rest have empty offsets ranges).
    drawn: usize,
    points: usize,
    surfaces: Vec<SurfaceCount>,
    tof: Stats,
    ke_mean: f64,
    /// Device that flew all ions.
    device: Device,
    precision: Precision,
    /// Why live mode ran on the CPU (no GPU, still starting, field does not fit, GPU error,
    /// exact surfaces).
    fallback: Option<Reason>,
    /// Exact electrode surfaces the ions splatted on (None: the grid's electrode points).
    surface: Option<SurfaceInfo>,
    /// Particles, plus the fast-adjusted CPU field for CPU flights.
    field_ms: f64,
    /// Background build + upload of the field to the flyer device, reported by the first GPU
    /// flight after it finished (0 afterwards).
    prepare_ms: f64,
    /// Compute: all ions, end states only (GPU: dispatch through completion; CPU: the flight).
    fly_ms: f64,
    /// GPU preparation around the compute: ion buffer setup/upload and result readback.
    transfer_ms: f64,
    /// Trajectories of the drawn subset on the CPU (concurrent with the GPU flight in live mode
    /// when GPU sampling failed); 0 when `paths_gpu`.
    paths_ms: f64,
    /// The drawn trajectories were sampled by the GPU flight itself.
    paths_gpu: bool,
    total_ms: f64,
    thinned: bool,
    /// GPU memory of the resident field incl. its pooled flight scratch, and the most ions per GPU
    /// dispatch (0 on the CPU).
    gpu_bytes: u64,
    dispatch_ions: usize,
    /// Width of the packet's surface indices: 1, 2 or 4 bytes.
    surface_bytes: usize,
    /// Key of this flight's final velocities for `flight_velocities`.
    token: u64,
}

/// A PA instance on the workbench (SIMION .iob): grid point (0,0,0) at `origin` (mm), PA unit
/// axes `axes` in workbench coordinates, PA spacing multiplied by `scale` (`InstanceField` rules).
#[derive(Deserialize, Serialize, Clone, Copy, PartialEq, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Placement {
    pub origin: Vector,
    pub axes: [Vector; 3],
    pub scale: f64,
}

impl Placement {
    pub const IDENTITY: Self = Self { origin: [0.0; 3], axes: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]], scale: 1.0 };

    /// Workbench mm to PA mm.
    pub fn local(&self, p: Vector) -> Vector {
        let d: Vector = std::array::from_fn(|a| (p[a] - self.origin[a]) / self.scale);
        self.axes.map(|axis| axis[0] * d[0] + axis[1] * d[1] + axis[2] * d[2])
    }

    /// A PA-frame field vector (per PA mm) in workbench coordinates (per workbench mm).
    pub fn world_field(&self, e: Vector) -> Vector {
        std::array::from_fn(|a| (0..3).map(|i| self.axes[i][a] * e[i]).sum::<f64>() / self.scale)
    }
}

/// The array placed on the workbench, optionally inside a field-free box (like `InstanceField`,
/// but borrowing the cached field instead of owning a copy).
struct Bench<'a> {
    grid: &'a GridField,
    bounds: Option<[Vector; 2]>,
    place: Placement,
}

impl Field for Bench<'_> {
    fn sample(&self, p: Vector, t: f64) -> FieldSample {
        let q = self.place.local(p);
        if self.bounds.is_some() && self.grid.status(q) == Some(SplatReason::OutsideGrid) {
            return FieldSample::default();
        }
        let s = self.grid.sample(q, t);
        FieldSample { potential_v: s.potential_v, electric_v_per_mm: self.place.world_field(s.electric_v_per_mm), magnetic_gauss: [0.0; 3] }
    }

    fn status(&self, p: Vector) -> Option<SplatReason> {
        if self.bounds.is_some_and(|b| (0..3).any(|a| p[a] < b[0][a] || p[a] > b[1][a])) {
            return Some(SplatReason::OutsideGrid);
        }
        self.grid.status(self.place.local(p)).filter(|s| self.bounds.is_none() || *s != SplatReason::OutsideGrid)
    }

    fn intersection(&self, from: Vector, to: Vector) -> Option<(f64, SplatReason)> {
        // Line fractions are unchanged by the affine placement.
        let (a, b) = (self.place.local(from), self.place.local(to));
        let Some(bounds) = self.bounds else {
            return self.grid.intersection(a, b);
        };
        let mut first = self.grid.electrode_intersection(a, b);
        for a in 0..3 {
            if to[a] < bounds[0][a] || to[a] > bounds[1][a] {
                let edge = if to[a] < bounds[0][a] { bounds[0][a] } else { bounds[1][a] };
                let f = (edge - from[a]) / (to[a] - from[a]);
                if (0.0..=1.0).contains(&f) && first.is_none_or(|(t, _)| f < t) {
                    first = Some((f, SplatReason::OutsideGrid));
                }
            }
        }
        first
    }
}

fn norm(v: Vector) -> f64 {
    (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt()
}

fn cross(a: Vector, b: Vector) -> Vector {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

/// A flight error the UI shows as technical text (OS, parser or integrator messages).
fn fly_err(e: impl ToString) -> PaError {
    PaError::new("fly", e.to_string())
}

/// A flight error the UI states in its language (`reason.<code>`).
fn fly_reason(reason: Reason) -> PaError {
    PaError::coded("fly", reason)
}

/// Deterministic beam: positions along a line / sunflower disk, directions on a Fibonacci cone.
fn beam(source: &Source, relativistic: bool) -> Result<Vec<Particle>, PaError> {
    let Source::Beam { count, mass_amu, charge_e, energy_ev, position_mm, direction, cone_deg, shape, width_mm } = *source else {
        unreachable!()
    };
    if !(1..=MAX_IONS).contains(&count) {
        return Err(fly_reason(Reason::new("ionCount").with("max", MAX_IONS)));
    }
    let length = norm(direction);
    if !(length.is_finite() && length > 0.0) {
        return Err(fly_reason(Reason::new("zeroDirection")));
    }
    if !(0.0..=180.0).contains(&cone_deg) || !(width_mm.is_finite() && width_mm >= 0.0) {
        return Err(fly_reason(Reason::new("coneWidth")));
    }
    let d = direction.map(|c| c / length);
    // u: +Y made perpendicular to the beam (+Z if the beam runs along Y); w completes the frame.
    let up = if d[1].abs() > 0.99 { [0.0, 0.0, 1.0] } else { [0.0, 1.0, 0.0] };
    let k = up[0] * d[0] + up[1] * d[1] + up[2] * d[2];
    let u = {
        let v = [up[0] - k * d[0], up[1] - k * d[1], up[2] - k * d[2]];
        let n = norm(v);
        v.map(|c| c / n)
    };
    let w = cross(d, u);
    let golden = std::f64::consts::PI * (3.0 - 5.0_f64.sqrt());
    let cos_cone = cone_deg.to_radians().cos();
    (0..count)
        .map(|i| {
            let t = (i as f64 + 0.5) / count as f64;
            let offset = match shape {
                Shape::Point => [0.0; 3],
                Shape::Line => u.map(|c| c * (t - 0.5) * width_mm),
                Shape::Disk => {
                    let r = 0.5 * width_mm * t.sqrt();
                    let (s, c) = (i as f64 * golden).sin_cos();
                    std::array::from_fn(|a| r * (c * u[a] + s * w[a]))
                }
            };
            // Decorrelate angle from position (a line source would otherwise fan out in order).
            let j = (i * 7919) % count;
            let tj = (j as f64 + 0.5) / count as f64;
            let cos_theta = 1.0 - (1.0 - cos_cone) * tj;
            let sin_theta = (1.0 - cos_theta * cos_theta).max(0.0).sqrt();
            let (sp, cp) = (j as f64 * golden).sin_cos();
            let dir: Vector = std::array::from_fn(|a| d[a] * cos_theta + (cp * u[a] + sp * w[a]) * sin_theta);
            let position: Vector = std::array::from_fn(|a| position_mm[a] + offset[a]);
            if relativistic {
                Particle::from_relativistic_energy(mass_amu, charge_e, position, energy_ev, dir, 0.0)
            } else {
                Particle::from_energy(mass_amu, charge_e, position, energy_ev, dir, 0.0)
            }
            .map_err(|e| fly_reason(Reason::new("badParticle").detail(e)))
        })
        .collect()
}

/// Particles for the request, reused from the last call while the source is unchanged.
fn cached_particles(session: &Session, request: &FlyRequest) -> Result<Arc<Vec<Particle>>, PaError> {
    let modified = match &request.source {
        Source::File { path } => std::fs::metadata(path).and_then(|m| m.modified()).ok(),
        Source::Beam { .. } => None,
    };
    let key = serde_json::to_string(&(&request.source, request.relativistic, modified)).map_err(fly_err)?;
    let mut cache = session.particles.lock().unwrap();
    if let Some((k, ions)) = cache.as_ref()
        && *k == key
    {
        return Ok(ions.clone());
    }
    let ions = Arc::new(particles(&request.source, request.relativistic)?);
    *cache = Some((key, ions.clone()));
    Ok(ions)
}

fn particles(source: &Source, relativistic: bool) -> Result<Vec<Particle>, PaError> {
    match source {
        Source::Beam { .. } => beam(source, relativistic),
        Source::File { path } => {
            let defs = read_particles(path).map_err(|e| fly_reason(Reason::new("ionFileRead").detail(e)))?;
            if defs.is_empty() || defs.len() > MAX_IONS {
                return Err(fly_reason(Reason::new("ionFileCount").with("count", defs.len()).with("max", MAX_IONS)));
            }
            defs.into_iter()
                .map(|p| {
                    if relativistic {
                        Particle::from_relativistic_energy(p.mass_amu, p.charge_e, p.position_mm, p.energy_ev, p.direction, p.birth_time_us)
                    } else {
                        Particle::from_energy(p.mass_amu, p.charge_e, p.position_mm, p.energy_ev, p.direction, p.birth_time_us)
                    }
                    .map_err(|e| fly_reason(Reason::new("badParticle").detail(e)))
                })
                .collect()
        }
    }
}

/// Which face of the box (explicit workbench, else the array's extent) a wall hit is on.
fn wall_face(session: &Session, bounds: Option<[Vector; 2]>, place: &Placement, p: Vector) -> &'static str {
    const FACES: [[&str; 2]; 3] = [["-x", "+x"], ["-y", "+y"], ["-z", "+z"]];
    let mut best = ("+x", f64::INFINITY);
    let mut consider = |distance: f64, face| {
        if distance < best.1 {
            best = (face, distance);
        }
    };
    if let Some(b) = bounds {
        for a in 0..3 {
            consider((p[a] - b[0][a]).abs(), FACES[a][0]);
            consider((p[a] - b[1][a]).abs(), FACES[a][1]);
        }
        return best.0;
    }
    let p = place.local(p);
    let a = &session.file.array;
    let n = [a.nx, a.ny, a.nz];
    let h = [a.dx_mm, a.dy_mm, a.dz_mm];
    let m = [a.mirrors.x, a.mirrors.y, a.mirrors.z];
    let cylindrical = a.symmetry == Symmetry::Cylindrical;
    for axis in 0..3 {
        if (cylindrical && axis > 0) || n[axis] == 1 {
            continue;
        }
        let hi = (n[axis] - 1) as f64 * h[axis];
        consider((p[axis] - if m[axis] { -hi } else { 0.0 }).abs(), FACES[axis][0]);
        consider((p[axis] - hi).abs(), FACES[axis][1]);
    }
    if cylindrical {
        consider((p[1].hypot(p[2]) - (a.ny - 1) as f64 * a.dy_mm).abs(), "r");
    }
    best.0
}

fn stats(values: impl Iterator<Item = f64> + Clone) -> Stats {
    let n = values.clone().count();
    if n == 0 {
        return Stats::default();
    }
    let mean = values.clone().sum::<f64>() / n as f64;
    let var = values.clone().map(|v| (v - mean) * (v - mean)).sum::<f64>() / n as f64;
    Stats {
        n,
        min: values.clone().fold(f64::INFINITY, f64::min),
        max: values.fold(f64::NEG_INFINITY, f64::max),
        mean,
        std: var.sqrt(),
    }
}

fn kinetic_energy(mass: f64, v: Vector, relativistic: bool) -> f64 {
    let v2 = v.iter().map(|c| c * c).sum::<f64>();
    if relativistic {
        let inverse_gamma = (1.0 - v2 / SPEED_OF_LIGHT.powi(2)).sqrt();
        mass * v2 / (ELECTRIC_ACCELERATION * inverse_gamma * (1.0 + inverse_gamma))
    } else {
        mass * v2 / (2.0 * ELECTRIC_ACCELERATION)
    }
}

#[tauri::command]
pub async fn fly(state: State<'_, AppState>, request: FlyRequest) -> Result<Response, PaError> {
    let session = state.get(&request.id).ok_or_else(|| PaError::new("noSession", request.id.clone()))?;
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || run(&state, &session, &request))
        .await
        .map_err(|e| PaError::new("unexpected", e.to_string()))?
        .map(Response::new)
}

fn ms(since: Instant) -> f64 {
    since.elapsed().as_secs_f64() * 1e3
}

/// End states of every ion, plus trajectories of the picked ions.
struct Flown {
    results: Vec<FlightResult>,
    /// Trajectories in `picks` order; None when `results` themselves carry them (every ion drawn,
    /// or sampled by the GPU flight).
    paths: Option<Vec<FlightResult>>,
    paths_gpu: bool,
    device: Device,
    /// CPU field build (0 on the GPU: its field is prepared in the background).
    setup_ms: f64,
    prepare_ms: f64,
    fly_ms: f64,
    transfer_ms: f64,
    paths_ms: f64,
    gpu_bytes: u64,
    dispatch_ions: usize,
}

/// One flight request, resolved: particles, drawn ions and both option sets.
struct Job<'a> {
    session: &'a Arc<Session>,
    request: &'a FlyRequest,
    bounds: Option<[Vector; 2]>,
    ions: &'a [Particle],
    /// Drawn ions, ascending indices into `ions`.
    picks: &'a [usize],
    terminal: &'a FlightOptions,
    sampled: &'a FlightOptions,
    /// Most GPU trajectory samples per drawn ion (the points kept after thinning, plus both ends).
    capacity: usize,
    /// The array with its exact electrode surfaces (CPU only).
    exact: Option<&'a Exact>,
}

impl Job<'_> {
    fn subset(&self) -> Vec<Particle> {
        self.picks.iter().map(|&i| self.ions[i].clone()).collect()
    }
}

/// Factors of the resident bases for the requested voltages: 1 for the PA0 base, then volts to add
/// per adjustable electrode.
fn factors(session: &Session, request: &FlyRequest) -> Result<Vec<f64>, PaError> {
    let deltas = match &session.fast {
        Some(fast) => fast.deltas(&request.voltages).map_err(fly_err)?,
        None if request.voltages.is_empty() => Vec::new(),
        None => return Err(fly_reason(Reason::new("noFastAdjust"))),
    };
    Ok(std::iter::once(1.0).chain(deltas).collect())
}

/// Live mode on the app's flyer device: all ions on the GPU, which also samples the picked ones'
/// trajectories. If sampling fails (capacity), the end states fly again without it and the picked
/// ones on the CPU meanwhile. The error is why the flight falls back to the CPU.
fn fly_gpu(device: &live::FlyDevice, job: &Job) -> Result<Flown, Reason> {
    let Job { session, request, ions, picks, terminal, sampled, capacity, .. } = *job;
    let factors = factors(session, request).map_err(|e| e.into_reason("gpuFlightFailed"))?;
    let key = (request.bounds, request.placement, request.collision);
    if session.gpu.lock().unwrap().as_ref().is_none_or(|s| s.key != key) {
        live::prepare_in_background(device.gpu, session.clone(), key);
    }
    let failed = |e: String| Reason::new("gpuFlightFailed").detail(e);
    let flown = |report: simrep_fly::gpu::FlightReport, gpu: (u64, usize), prepare_ms: f64, paths: Option<Vec<FlightResult>>, paths_ms: f64| {
        let t = &report.timings;
        let ms = |d: std::time::Duration| d.as_secs_f64() * 1e3;
        Flown {
            fly_ms: ms(t.fly),
            // Host-side work around the compute; time spent queued behind another flight is not.
            transfer_ms: ms(t.setup + t.upload + t.readback),
            results: report.results,
            paths_gpu: paths.is_none(),
            paths,
            device: Device { kind: "gpu", name: device.name(), threads: None },
            setup_ms: 0.0,
            prepare_ms,
            paths_ms,
            gpu_bytes: gpu.0,
            dispatch_ions: gpu.1,
        }
    };
    let subset = job.subset();
    let (resident, prepare_ms, longest) = {
        let mut guard = session.gpu.lock().unwrap();
        let slot = guard.as_mut().ok_or_else(|| Reason::new("fieldReplaced"))?;
        let pool = match &slot.resident {
            None => return Err(Reason::new("fieldUploading")),
            Some(Err(e)) => return Err(Reason::new("fieldFailed").detail(e)),
            Some(Ok(p)) => p.clone(),
        };
        (pool, std::mem::take(&mut slot.prepare_ms), slot.longest)
    };
    // Sizes the next flights' samples, unless the field was replaced meanwhile.
    let set_longest = |n: usize| {
        if let Some(slot) = session.gpu.lock().unwrap().as_mut().filter(|s| s.resident.as_ref().is_some_and(|r| r.as_ref().is_ok_and(|p| Arc::ptr_eq(p, &resident)))) {
            slot.longest = n;
        }
    };
    // Concurrent requests share the resident field; each holds its lock only to submit (see
    // `Resident::fly`). On the CPU fallback the drawn subset flies meanwhile.
    std::thread::scope(|scope| {
        if !picks.is_empty() {
            // Samples sized from the last flight's longest drawn ion (a quarter more, for voltage
            // changes): every accepted step when that fits `capacity`, else every n-th. The whole
            // sample buffer is read back, so it is kept no larger than needed.
            let (every, samples) = match longest {
                0 => (1, capacity),
                n if n + n / 4 + 2 <= capacity => (1, n + n / 4 + 2),
                n => ((n + n / 4).div_ceil(capacity - 2), capacity),
            };
            let sampling = TrajectorySampling { ion_indices: picks, every, max_samples_per_ion: samples };
            let (report, _) = resident.fly(&factors, ions, terminal, Some(&sampling));
            if let Ok(report) = report {
                set_longest(picks.iter().map(|&i| report.results[i].accepted_steps).max().unwrap_or(0));
                return Ok(flown(report, resident.gpu_usage(), prepare_ms, None, 0.0));
            }
            // Most likely more samples than fit: plan for far longer trajectories next time.
            set_longest(longest.max(capacity) * 4);
        }
        let fields = resident.fields();
        let (factors, subset) = (&factors, &subset);
        let cpu = scope.spawn(move || live::fly_paths(&fields, factors, subset, sampled));
        let (report, _) = resident.fly(factors, ions, terminal, None);
        let (paths, paths_ms) = cpu.join().map_err(|_| failed("trajectory worker panicked".into()))?;
        Ok(flown(report.map_err(failed)?, resident.gpu_usage(), prepare_ms, Some(paths.map_err(failed)?), paths_ms))
    })
}

/// `fly_many` through one field (the borrowed array, or the array with exact surfaces).
type FlyAll<'a> = &'a dyn Fn(&[Particle], &FlightOptions) -> Result<Vec<FlightResult>, simrep_fly::FlyError>;

/// Precise mode (and live fallback): every ion on the CPU through the fast-adjusted array, with
/// its exact surfaces when the job has them.
fn fly_cpu(job: &Job) -> Result<Flown, PaError> {
    let Job { session, request, bounds, ions, picks, terminal, sampled, exact, .. } = *job;
    let all_drawn = picks.len() == ions.len();
    let run = |field: FlyAll, setup_ms: f64| -> Result<Flown, PaError> {
        let started = Instant::now();
        let results = field(ions, if all_drawn { sampled } else { terminal }).map_err(fly_err)?;
        let fly_ms = ms(started);
        let started = Instant::now();
        let paths = if all_drawn { None } else { Some(field(&job.subset(), sampled).map_err(fly_err)?) };
        let paths_ms = if all_drawn { 0.0 } else { ms(started) };
        Ok(Flown { results, paths, paths_gpu: false, device: cpu_device(), setup_ms, prepare_ms: 0.0, fly_ms, transfer_ms: 0.0, paths_ms, gpu_bytes: 0, dispatch_ions: 0 })
    };
    if let Some(exact) = exact {
        return run(&|ions, options| fly_many(&*exact.field, ions, options), 0.0);
    }
    let started = Instant::now();
    let grid = session.field(&request.voltages, request.collision).map_err(fly_err)?;
    let setup_ms = ms(started);
    let bench = Bench { grid: &grid, bounds, place: request.placement.unwrap_or(Placement::IDENTITY) };
    run(&|ions, options| fly_many(&bench, ions, options), setup_ms)
}

/// Checks the request and returns its workbench box.
fn validate(session: &Session, request: &FlyRequest) -> Result<Option<[Vector; 2]>, PaError> {
    if session.is_pa_hash() {
        return Err(fly_reason(Reason::new("unrefined")));
    }
    if !(request.duration_us.is_finite() && request.duration_us > 0.0) {
        return Err(fly_reason(Reason::new("timeLimit")));
    }
    let bounds = request.bounds.map(|b| [[b[0], b[1], b[2]], [b[3], b[4], b[5]]]);
    if bounds.is_some_and(|b| (0..3).any(|a| !(b[0][a].is_finite() && b[1][a].is_finite() && b[0][a] < b[1][a]))) {
        return Err(fly_reason(Reason::new("benchBounds")));
    }
    if let Some(p) = request.placement {
        let orthonormal = (0..3).all(|i| (0..3).all(|j| ((0..3).map(|k| p.axes[i][k] * p.axes[j][k]).sum::<f64>() - f64::from(i == j)).abs() < 1e-9));
        if !(p.scale.is_finite() && p.scale > 0.0 && orthonormal && p.origin.iter().all(|v| v.is_finite())) {
            return Err(fly_reason(Reason::new("placement")));
        }
    }
    Ok(bounds)
}

/// The array with its sidecar's exact surfaces for `request`, unless off or there is no sidecar.
fn exact_field(session: &Session, request: &FlyRequest, bounds: Option<[Vector; 2]>, terminal: &FlightOptions) -> Result<Option<Exact>, PaError> {
    if !request.surfaces {
        return Ok(None);
    }
    session.exact(&request.voltages, bounds, &request.placement.unwrap_or(Placement::IDENTITY), terminal.position_tolerance_mm)
}

/// End-state integration settings shared by every flight (and the single-ion trace).
fn terminal_options(request: &FlyRequest) -> FlightOptions {
    FlightOptions {
        relative_tolerance: 1e-8,
        position_tolerance_mm: 1e-8,
        velocity_tolerance_mm_per_us: 1e-8,
        initial_step_us: 0.001,
        max_step_us: 0.1,
        duration_us: request.duration_us,
        relativistic: request.relativistic,
        sample_every: None,
        ..FlightOptions::default()
    }
}

/// Where an ion ended.
fn classify(session: &Session, bounds: Option<[Vector; 2]>, place: &Placement, r: &FlightResult) -> Surface {
    match r.splat_reason {
        SplatReason::Electrode { index } => match &session.fast {
            Some(fast) => Surface::Electrode { number: fast.electrode_number(index), voltage: None },
            None => Surface::Electrode { number: None, voltage: session.file.array.potentials.get(index).copied() },
        },
        // Continuous surface models (cuts sidecars) report the physical electrode number.
        SplatReason::SurfaceElectrode { number } => Surface::Electrode { number: Some(number), voltage: None },
        SplatReason::OutsideGrid | SplatReason::StopPlane => Surface::Wall { face: wall_face(session, bounds, place, r.final_state.position_mm) },
        SplatReason::TimeLimit => Surface::TimeLimit,
        SplatReason::StepLimit | SplatReason::StepUnderflow | SplatReason::NonFinite => Surface::Stalled,
    }
}

fn run(state: &AppState, session: &Arc<Session>, request: &FlyRequest) -> Result<Vec<u8>, PaError> {
    let start = Instant::now();
    let bounds = validate(session, request)?;
    let ions = cached_particles(session, request)?;
    let particles_ms = ms(start);
    let n = ions.len();
    let drawn = request.draw.min(MAX_DRAWN).min(n);
    let picks: Vec<usize> = (0..drawn).map(|k| k * n / drawn).collect();

    let terminal = terminal_options(request);
    let sampled = FlightOptions { sample_every: Some(1), ..terminal.clone() };
    let place = request.placement.unwrap_or(Placement::IDENTITY);
    let started = Instant::now();
    let exact = exact_field(session, request, bounds, &terminal)?;
    let exact_ms = ms(started);
    // Points kept per drawn trajectory (long ones are thinned evenly, keeping both ends).
    let cap = (POINT_BUDGET / drawn.max(1)).clamp(16, MAX_POINTS_PER_ION);

    let job = Job { session, request, bounds, ions: &ions, picks: &picks, terminal: &terminal, sampled: &sampled, capacity: cap + 2, exact: exact.as_ref() };
    let mut fallback = None;
    let mut flown = None;
    if request.precision == Precision::Live && exact.is_some() {
        // Surface collisions are CPU-only (as `simrep fly` forces --device cpu for a sidecar).
        fallback = Some(Reason::new("exactSurfaces"));
    } else if request.precision == Precision::Live {
        match state.fly_gpu.get() {
            None => fallback = Some(Reason::new("gpuStarting")),
            Some(Err(e)) => fallback = Some(e.clone()),
            Some(Ok(device)) => match fly_gpu(device, &job) {
                Ok(f) => flown = Some(f),
                Err(e) => fallback = Some(e),
            },
        }
    }
    let flown = match flown {
        Some(f) => f,
        None => fly_cpu(&job)?,
    };
    let results = &flown.results;

    // Classify where each ion ended.
    let mut surfaces: Vec<SurfaceCount> = Vec::new();
    let mut surface_of = Vec::with_capacity(results.len());
    for r in results {
        let surface = classify(session, bounds, &place, r);
        let k = match surfaces.iter().position(|s| s.surface == surface) {
            Some(k) => k,
            None => {
                surfaces.push(SurfaceCount { surface, count: 0, tof_mean: 0.0 });
                surfaces.len() - 1
            }
        };
        surfaces[k].count += 1;
        surfaces[k].tof_mean += r.tof_us;
        surface_of.push(k as u32);
    }
    for s in &mut surfaces {
        s.tof_mean /= s.count as f64;
    }
    let ended = results.iter().filter(|r| matches!(r.splat_reason, SplatReason::Electrode { .. } | SplatReason::OutsideGrid | SplatReason::StopPlane));
    let tof = stats(ended.map(|r| r.tof_us));
    let ke: Vec<f64> = results
        .iter()
        .zip(ions.iter())
        .map(|(r, ion)| kinetic_energy(ion.mass_amu, r.final_state.velocity_mm_per_us, request.relativistic))
        .collect();
    let ke_mean = ke.iter().sum::<f64>() / results.len() as f64;

    // Trajectories of the picked ions, thinned evenly (always keeping both ends) to the point budget.
    let mut offsets = Vec::with_capacity(picks.len() + 1);
    let mut points: Vec<f32> = Vec::new();
    let mut thinned = false;
    offsets.push(0u32);
    for (next, &i) in picks.iter().enumerate() {
        let path = flown.paths.as_ref().map_or(&results[i], |p| &p[next]);
        let samples = path.trajectory.as_deref().unwrap_or(&[]);
        let stride = samples.len().div_ceil(cap).max(1);
        thinned |= stride > 1;
        for (k, s) in samples.iter().enumerate() {
            if k % stride == 0 || k + 1 == samples.len() {
                points.extend(s.position_mm.map(|c| c as f32));
            }
        }
        offsets.push((points.len() / 3) as u32);
    }
    let surface_bytes = match surfaces.len() {
        0..=256 => 1,
        257..=65_536 => 2,
        _ => 4,
    };
    let token = keep_velocities(session, results);

    let header = Header {
        ions: results.len(),
        drawn,
        points: points.len() / 3,
        surfaces,
        tof,
        ke_mean,
        device: flown.device.clone(),
        precision: request.precision,
        fallback: if request.precision == Precision::Live { fallback } else { None },
        surface: exact.as_ref().map(SurfaceInfo::of),
        field_ms: particles_ms + exact_ms + flown.setup_ms,
        prepare_ms: flown.prepare_ms,
        fly_ms: flown.fly_ms,
        transfer_ms: flown.transfer_ms,
        paths_ms: flown.paths_ms,
        paths_gpu: flown.paths_gpu,
        total_ms: start.elapsed().as_secs_f64() * 1e3,
        thinned,
        gpu_bytes: flown.gpu_bytes,
        dispatch_ions: flown.dispatch_ions,
        surface_bytes,
        token,
    };
    let json = serde_json::to_vec(&header).map_err(fly_err)?;
    let padded = json.len().div_ceil(4) * 4;
    let n = results.len();
    let mut out = Vec::with_capacity(8 + padded + 4 * (picks.len() + offsets.len() + points.len() + 5 * n) + surface_bytes * n);
    out.extend_from_slice(b"STR3");
    out.extend_from_slice(&(padded as u32).to_le_bytes());
    out.extend_from_slice(&json);
    out.resize(8 + padded, b' ');
    picks.iter().for_each(|&i| out.extend_from_slice(&(i as u32).to_le_bytes()));
    offsets.iter().for_each(|v| out.extend_from_slice(&v.to_le_bytes()));
    points.iter().for_each(|v| out.extend_from_slice(&v.to_le_bytes()));
    for r in results {
        let p = r.final_state.position_mm;
        for v in [p[0], p[1], p[2], r.tof_us] {
            out.extend_from_slice(&(v as f32).to_le_bytes());
        }
    }
    ke.iter().for_each(|&v| out.extend_from_slice(&(v as f32).to_le_bytes()));
    for &s in &surface_of {
        out.extend_from_slice(&s.to_le_bytes()[..surface_bytes]);
    }
    Ok(out)
}

/// Recent flights whose final velocities a session keeps: the one on screen is among them, since
/// live re-flying keeps at most three requests in flight (src/scene/simulation.ts).
const KEPT_VELOCITIES: usize = 4;

/// Keeps the final velocities of `results` (f32 vx, vy, vz mm/us per ion) for
/// `flight_velocities` and returns their token.
fn keep_velocities(session: &Session, results: &[FlightResult]) -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    let token = NEXT.fetch_add(1, Ordering::Relaxed);
    let velocities: Vec<f32> = results.iter().flat_map(|r| r.final_state.velocity_mm_per_us.map(|v| v as f32)).collect();
    let mut kept = session.velocities.lock().unwrap();
    if kept.len() == KEPT_VELOCITIES {
        kept.pop_front();
    }
    kept.push_back((token, Arc::new(velocities)));
    token
}

/// Final velocities of a recent flight of array `id` (its header's `token`): f32 vx, vy, vz mm/us
/// per ion, for the end-state CSV export (flight packets leave them out).
#[tauri::command]
pub fn flight_velocities(state: State<'_, AppState>, id: String, token: u64) -> Result<Response, PaError> {
    let session = state.get(&id).ok_or_else(|| PaError::new("noSession", id.clone()))?;
    let kept = session.velocities.lock().unwrap().iter().find(|(t, _)| *t == token).map(|(_, v)| v.clone());
    let velocities: Arc<Vec<f32>> = kept.ok_or_else(|| PaError::coded("export", Reason::new("flightExpired")))?;
    Ok(Response::new(velocities.iter().flat_map(|v| v.to_le_bytes()).collect::<Vec<u8>>()))
}

/// Full time series of the drawn ions (CPU, f64, every integrator step) as VTK PolyData (.vtp)
/// or the numeric trajectory CSV of `simrep export trajectories` (docs/mesh-io.md). Answers the
/// number of trajectories written.
#[tauri::command]
pub async fn export_trajectories(state: State<'_, AppState>, request: FlyRequest, path: String) -> Result<usize, PaError> {
    let session = state.get(&request.id).ok_or_else(|| PaError::new("noSession", request.id.clone()))?;
    tauri::async_runtime::spawn_blocking(move || write_trajectories(&session, &request, &path))
        .await
        .map_err(|e| PaError::new("unexpected", e.to_string()))?
}

fn write_trajectories(session: &Arc<Session>, request: &FlyRequest, path: &str) -> Result<usize, PaError> {
    use simrep_mesh::{Trajectory, TrajectoryPoint, write_trajectories_csv, write_vtp};
    let bounds = validate(session, request)?;
    let ions = cached_particles(session, request)?;
    let n = ions.len();
    let drawn = (if request.draw == 0 { DEFAULT_DRAWN } else { request.draw }).min(MAX_DRAWN).min(n);
    let subset: Vec<Particle> = (0..drawn).map(|k| ions[k * n / drawn].clone()).collect();
    let options = FlightOptions { sample_every: Some(1), ..terminal_options(request) };
    let results = match exact_field(session, request, bounds, &options)? {
        Some(exact) => fly_many(&*exact.field, &subset, &options),
        None => {
            let grid = session.field(&request.voltages, request.collision).map_err(fly_err)?;
            let bench = Bench { grid: &grid, bounds, place: request.placement.unwrap_or(Placement::IDENTITY) };
            fly_many(&bench, &subset, &options)
        }
    }
    .map_err(fly_err)?;
    let trajectories: Vec<Trajectory> = results
        .iter()
        .zip(0..drawn)
        .filter_map(|(r, k)| {
            let points: Vec<TrajectoryPoint> = r
                .trajectory
                .as_deref()?
                .iter()
                .map(|s| TrajectoryPoint { time_us: s.time_us, position_mm: s.position_mm, velocity_mm_per_us: s.velocity_mm_per_us })
                .collect();
            (points.len() >= 2).then(|| Trajectory { id: (k * n / drawn + 1) as u64, points })
        })
        .collect();
    if trajectories.is_empty() {
        return Err(PaError::coded("export", Reason::new("noTrajectories")));
    }
    let csv = std::path::Path::new(path).extension().is_some_and(|e| e.eq_ignore_ascii_case("csv"));
    if csv { write_trajectories_csv(path, &trajectories) } else { write_vtp(path, &trajectories) }.map_err(|e| PaError::new("export", e.to_string()))?;
    Ok(trajectories.len())
}

/// Most samples returned by `ion_trace` (thinned evenly, keeping both ends).
const TRACE_POINTS: usize = 5_000;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TraceHeader {
    ion: usize,
    mass_amu: f64,
    charge_e: f64,
    samples: usize,
    /// Integrator steps before thinning.
    steps: usize,
    thinned: bool,
    surface: Surface,
    tof_us: f64,
    /// "resident": the live GPU field's bases summed on the CPU; "array": the fast-adjusted array;
    /// "exact": that array with the sidecar's exact electrode surfaces.
    field: &'static str,
    ms: f64,
}

/// Full time series of ion `ion` (index into the request's ions), flown alone on the CPU in f64
/// through the field the request's flight used.
#[tauri::command]
pub async fn ion_trace(state: State<'_, AppState>, request: FlyRequest, ion: usize) -> Result<Response, PaError> {
    let session = state.get(&request.id).ok_or_else(|| PaError::new("noSession", request.id.clone()))?;
    tauri::async_runtime::spawn_blocking(move || trace(&session, &request, ion))
        .await
        .map_err(|e| PaError::new("unexpected", e.to_string()))?
        .map(Response::new)
}

fn trace(session: &Arc<Session>, request: &FlyRequest, ion: usize) -> Result<Vec<u8>, PaError> {
    let start = Instant::now();
    let bounds = validate(session, request)?;
    let ions = cached_particles(session, request)?;
    let particle = ions.get(ion).ok_or_else(|| fly_err(format!("ion {ion} does not exist ({} ions)", ions.len())))?;
    let options = FlightOptions { sample_every: Some(1), ..terminal_options(request) };
    // Exact surfaces fly on the CPU in both modes. Otherwise live mode with a resident field: the
    // same bases and factors as the live flight.
    let place = request.placement.unwrap_or(Placement::IDENTITY);
    let exact = exact_field(session, request, bounds, &options)?;
    let resident = match request.precision {
        Precision::Live if exact.is_none() => session.live_fields.lock().unwrap().as_ref().filter(|(k, _)| *k == (request.bounds, request.placement, request.collision)).map(|(_, f)| f.clone()),
        _ => None,
    };
    let (result, potentials, field) = match (resident, exact) {
        (Some(fields), _) => {
            let (r, v) = live::trace(&fields, &factors(session, request)?, particle, &options).map_err(fly_err)?;
            (r, v, "resident")
        }
        (None, Some(exact)) => {
            let field = &*exact.field;
            let r = fly_one(field, particle, &options).map_err(fly_err)?;
            let v = r.trajectory.as_deref().unwrap_or(&[]).iter().map(|s| field.sample(s.position_mm, s.time_us).potential_v).collect();
            (r, v, "exact")
        }
        (None, None) => {
            let grid = session.field(&request.voltages, request.collision).map_err(fly_err)?;
            let bench = Bench { grid: &grid, bounds, place };
            let r = fly_one(&bench, particle, &options).map_err(fly_err)?;
            let v = r.trajectory.as_deref().unwrap_or(&[]).iter().map(|s| bench.sample(s.position_mm, s.time_us).potential_v).collect();
            (r, v, "array")
        }
    };
    let samples = result.trajectory.as_deref().unwrap_or(&[]);
    let stride = samples.len().div_ceil(TRACE_POINTS).max(1);
    let keep: Vec<usize> = (0..samples.len()).filter(|&k| k % stride == 0 || k + 1 == samples.len()).collect();
    let header = TraceHeader {
        ion,
        mass_amu: particle.mass_amu,
        charge_e: particle.charge_e,
        samples: keep.len(),
        steps: samples.len(),
        thinned: stride > 1,
        surface: classify(session, bounds, &place, &result),
        tof_us: result.tof_us,
        field,
        ms: ms(start),
    };
    let json = serde_json::to_vec(&header).map_err(fly_err)?;
    let padded = json.len().div_ceil(8) * 8;
    let mut out = Vec::with_capacity(8 + padded + 72 * keep.len());
    out.extend_from_slice(b"SIT1");
    out.extend_from_slice(&(padded as u32).to_le_bytes());
    out.extend_from_slice(&json);
    out.resize(8 + padded, b' ');
    for k in keep {
        let s = &samples[k];
        let ke = kinetic_energy(particle.mass_amu, s.velocity_mm_per_us, request.relativistic);
        let p = s.position_mm;
        let v = s.velocity_mm_per_us;
        for x in [s.time_us, p[0], p[1], p[2], v[0], v[1], v[2], ke, potentials[k]] {
            out.extend_from_slice(&x.to_le_bytes());
        }
    }
    Ok(out)
}

/// Fast-adjusts a .pa0 to `voltages` and returns the superposed potentials as an SPQ1 packet (for
/// the section view, `pa::potentials_packet`); the array stays cached for the next `fly` with the
/// same voltages.
#[tauri::command]
pub async fn fast_adjust(state: State<'_, AppState>, id: String, voltages: Vec<(usize, f64)>) -> Result<Response, PaError> {
    let session = state.get(&id).ok_or_else(|| PaError::new("noSession", id.clone()))?;
    tauri::async_runtime::spawn_blocking(move || {
        let array = session.array(&voltages)?;
        let size = std::fs::metadata(&session.path).map_or(0, |m| m.len());
        let fast = session.fast.as_ref().map(|f| f.status_at(&voltages));
        crate::pa::potentials_packet(&session.file, &array, crate::pa::Origin { path: &session.path, file_size: size, fast })
    })
    .await
    .map_err(|e| PaError::new("unexpected", e.to_string()))?
    .map(Response::new)
}
