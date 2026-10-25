//! GPU live flying: one flyer per app session, one resident field per open array.
//!
//! The Vulkan flyer (`simrep_fly::gpu`) is built once, in the background at start-up, on the
//! refine solver's device (`Gpu::from_device_with_precision`, f32 preview pipelines on the same
//! `wgpu::Device` and queue, so the app opens one GPU device), and kept for the life of the app.
//! Each array session prepares its field on it once (PA0 base plus one unit-voltage basis per
//! adjustable electrode, compact `GridField`s, see `Session::gpu`); a voltage change then only
//! rewrites the small voltage uniform before the next flight. No basis is re-uploaded.
//!
//! Live re-flying keeps up to three requests in flight (src/scene/simulation.ts). A request locks
//! the prepared field only to set its voltages and submit (`PreparedField::submit`: ion packing,
//! upload, enqueue), then waits for its own `PendingFlight` unlocked: the next request packs and
//! queues its flight while the previous one is still on the GPU, being read back, or crossing the
//! IPC, so the GPU does not idle between flights. 3D fields also cache their basis cell powers on
//! the GPU (`cache_basis_powers`), which removes the per-voltage-change basis expansion (10 of 32 ms
//! per 10k-ion quadrupole flight); `plan` allows that only within `LIVE_VRAM_BUDGET`, from the
//! field's measured GPU memory.
//!
//! The drawn subset's trajectories are sampled on the GPU in the same flight (`fly_sampled`).
//! When that fails (sample capacity exceeded), the end states are flown again without samples and
//! the trajectories on the CPU through `LiveField`, which sums the same resident bases (kept
//! CPU-side in `Fields`) with the same factors.

use std::sync::{Arc, Mutex};
use std::time::Instant;

use serde::Serialize;
use self_cell::self_cell;
use simrep_core::PotentialArray;
use simrep_fly::gpu::{FlightReport, Gpu, Precision, PreparedField, TrajectorySampling};
use simrep_fly::{Field, FieldSample, FlightOptions, FlightResult, GridField, InstanceField, Particle, SplatReason, Vector, fly, fly_many};

use crate::fly::{Collision, Placement};
use crate::pa::Reason;
use crate::session::{AppState, Session};

/// What a resident field was built for: workbench box and array placement.
pub type BenchKey = (Option<[f64; 6]>, Option<Placement>, Collision);

/// The app's flyer device. Leaked once: it lives as long as the app, and prepared fields borrow it.
pub struct FlyDevice {
    pub gpu: &'static Gpu,
    pub init_ms: f64,
}

impl FlyDevice {
    pub fn name(&self) -> String {
        format!("{} · Vulkan", self.gpu.info.name)
    }
}

/// Builds the flyer's preview (f32) pipelines on the solver's device: one device for refine and
/// live flights. Without a solver device there is no flyer either (same reason).
pub fn probe(solver: &Result<Arc<simrep_solve::gpu::Gpu>, Reason>) -> Result<FlyDevice, Reason> {
    let solver = solver.as_ref().map_err(Clone::clone)?;
    let start = Instant::now();
    let gpu = pollster::block_on(Gpu::from_device_with_precision(solver.device().clone(), solver.queue().clone(), solver.info.clone(), Precision::Preview))
        .map_err(|e| Reason::new("gpuInit").detail(e))?;
    Ok(FlyDevice { gpu: Box::leak(Box::new(gpu)), init_ms: start.elapsed().as_secs_f64() * 1e3 })
}

/// The flyer device for the status bar (mirrored by `ComputeDevice` in src/scene/backend.ts).
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ComputeDevice {
    kind: &'static str,
    name: String,
    init_ms: f64,
    /// Why there is no GPU (live flights then run on the CPU).
    reason: Option<Reason>,
    /// CPU threads (CPU device only; the UI names the device in its language).
    threads: Option<usize>,
}

/// Waits for the start-up device creation (main.rs), or creates the device if it never started.
#[tauri::command]
pub async fn compute_device(state: tauri::State<'_, AppState>) -> Result<ComputeDevice, ()> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || match state.fly_gpu.get_or_init(|| probe(state.gpu.get_or_init(crate::refine::probe_gpu))) {
        Ok(d) => ComputeDevice { kind: "gpu", name: d.name(), init_ms: d.init_ms, reason: None, threads: None },
        Err(e) => {
            let cpu = crate::refine::cpu_device();
            ComputeDevice { kind: "cpu", name: cpu.name, init_ms: 0.0, reason: Some(e.clone()), threads: cpu.threads }
        }
    })
    .await
    .map_err(|_| ())
}

/// CPU side of a resident field: the placed PA0 base and the unit-voltage bases uploaded with it.
/// Shared (Arc) so trajectories can be flown on the CPU without holding the GPU field's lock.
pub struct Fields {
    instance: InstanceField,
    bases: Vec<GridField>,
    bounded: bool,
    place: Placement,
}

/// The prepared field. Its lock covers a voltage change and a flight's submission, never the GPU
/// work or the readback (see `Resident::fly`).
pub struct Live<'a>(Mutex<PreparedField<'a>>);

self_cell!(
    /// A field resident on the flyer device, with the CPU fields it was built from. Shared by the
    /// concurrent live requests of its array.
    pub struct Resident {
        owner: Arc<Fields>,
        #[not_covariant]
        dependent: Live,
    }
);

/// GPU memory the live flyer may give one array's resident field (see `plan`).
const LIVE_VRAM_BUDGET: u64 = 2 << 30;

/// Whether to cache the uploaded field's basis cell powers (`cache_basis_powers`): only when the
/// cached field surely fits `LIVE_VRAM_BUDGET`. `compact`: the uploaded field's GPU memory
/// (`gpu_memory_bytes`). A compact 3D field holds one combined set of cell powers next to its much
/// smaller nodal bases; the cache adds one set per basis (`degrees`: the PA0 base plus one per
/// adjustable electrode), so the cached field holds at most `compact * (1 + degrees)` bytes. 2D
/// fields already hold every basis's powers (caching does nothing). Measured on the quadrupole
/// template (RTX 3070, before submit/collect, ms per flight at 10k / 100k ions with two requests
/// in flight): compact 30.8 / 129, cached 21.2 / 120.
fn plan(compact: u64, degrees: usize) -> bool {
    compact.saturating_mul(1 + degrees as u64) <= LIVE_VRAM_BUDGET
}

/// A session's resident field, keyed by the workbench box and placement it was built for.
pub struct Slot {
    pub key: BenchKey,
    /// None while `prepare_in_background` runs.
    pub resident: Option<Result<Arc<Resident>, String>>,
    /// Preparation time, reported once by the first flight that uses the field.
    pub prepare_ms: f64,
    /// Accepted steps of the last flight's longest drawn ion (0: none yet), which sizes the next
    /// flight's GPU trajectory samples.
    pub longest: usize,
}

/// Marks `session`'s slot as preparing for `bounds` and builds + uploads the field on a background
/// thread, so the request that triggered it (and any during the upload) fly on the CPU meanwhile.
pub fn prepare_in_background(gpu: &'static Gpu, session: Arc<Session>, key: BenchKey) {
    *session.gpu.lock().unwrap() = Some(Slot { key, resident: None, prepare_ms: 0.0, longest: 0 });
    std::thread::spawn(move || {
        let started = Instant::now();
        let placed = key.0.map(|b| [[b[0], b[1], b[2]], [b[3], b[4], b[5]]]);
        let resident = prepare(gpu, &session, placed, key.1.unwrap_or(Placement::IDENTITY), key.2);
        let mut slot = session.gpu.lock().unwrap();
        // A newer request may have asked for another workbench box meanwhile; keep its slot.
        if let Some(s) = slot.as_mut().filter(|s| s.key == key && s.resident.is_none()) {
            s.resident = Some(resident.map(|(resident, fields)| {
                *session.live_fields.lock().unwrap() = Some((key, fields));
                resident
            }));
            s.prepare_ms = started.elapsed().as_secs_f64() * 1e3;
        }
    });
}

/// Builds the base + unit bases of `session` once and uploads them (once per array and workbench
/// box), with 3D basis cell powers cached when `plan` allows it, else expanded on a voltage change.
/// Compact fields keep the shared nodal derivatives instead of every CPU cell polynomial, so the
/// CPU build is lighter. A cache that does not fit the device (`check_size`) leaves the field
/// compact.
pub fn prepare(gpu: &'static Gpu, session: &Session, bounds: Option<[Vector; 2]>, place: Placement, collision: Collision) -> Result<(Arc<Resident>, Arc<Fields>), String> {
    let array = &session.file.array;
    let grid = collision.apply(GridField::new_compact(array.clone()).map_err(|e| e.to_string())?);
    let bases = match &session.fast {
        Some(fast) => fast
            .unit_bases()
            .map(|potentials| GridField::new_compact(PotentialArray { potentials, ..array.clone() }).map(|g| collision.apply(g)).map_err(|e| e.to_string()))
            .collect::<Result<Vec<_>, _>>()?,
        None => Vec::new(),
    };
    let instance = InstanceField::new(grid, place.origin, place.axes, place.scale, bounds, None).map_err(|e| e.to_string())?;
    let degrees = 1 + bases.len();
    let fields = Arc::new(Fields { instance, bases, bounded: bounds.is_some(), place });
    let mut resident = Resident::try_new(fields.clone(), |f| gpu.prepare(&f.instance, &f.bases).map(|p| Live(Mutex::new(p))).map_err(|e| e.to_string()))?;
    resident.with_dependent_mut(|_, live| {
        let prepared = live.0.get_mut().unwrap();
        if plan(prepared.gpu_memory_bytes(), degrees) {
            // An error here (cache larger than the device's buffer limit) leaves the field compact.
            let _ = prepared.cache_basis_powers();
        }
    });
    Ok((Arc::new(resident), fields))
}

impl Resident {
    pub fn fields(&self) -> Arc<Fields> {
        self.borrow_owner().clone()
    }

    /// All ions on the GPU, with the trajectories of `sampling`'s ions when given. `factors`: 1 for
    /// the PA0 base, then (requested - stored) volts per adjustable electrode. The field is locked
    /// only while the voltages are set and the flight is submitted; collecting it (GPU wait,
    /// readback) runs unlocked, so a concurrent request queues its flight right behind this one.
    /// Returns the report and its wall time (including any wait for the lock and the GPU queue).
    pub fn fly(&self, factors: &[f64], ions: &[Particle], terminal: &FlightOptions, sampling: Option<&TrajectorySampling>) -> (Result<FlightReport, String>, f64) {
        let start = Instant::now();
        let report = self.with_dependent(|_, live| {
            let pending = {
                let mut prepared = live.0.lock().unwrap();
                prepared.set_voltages(factors)?;
                match sampling {
                    Some(s) => prepared.submit_sampled(ions, terminal, s),
                    None => prepared.submit(ions, terminal),
                }?
            };
            pending.collect()
        });
        (report.map_err(|e| e.to_string()), start.elapsed().as_secs_f64() * 1e3)
    }

    /// GPU memory of the field, including its pooled flight scratch (`gpu_memory_bytes`), and the
    /// most ions per dispatch (the crate's default cap, clamped to the device).
    pub fn gpu_usage(&self) -> (u64, usize) {
        self.with_dependent(|_, live| {
            let prepared = live.0.lock().unwrap();
            (prepared.gpu_memory_bytes(), prepared.max_ions_per_dispatch())
        })
    }
}

/// Trajectories of `subset` through the same bases and factors, on the CPU (f64). Returns the
/// results and their wall time.
pub fn fly_paths(fields: &Fields, factors: &[f64], subset: &[Particle], sampled: &FlightOptions) -> (Result<Vec<FlightResult>, String>, f64) {
    let start = Instant::now();
    let paths = if subset.is_empty() { Ok(Vec::new()) } else { fly_many(&LiveField { fields, factors }, subset, sampled).map_err(|e| e.to_string()) };
    (paths, start.elapsed().as_secs_f64() * 1e3)
}

/// One ion through the same bases and factors with its trajectory, plus the potential at every
/// sample (the per-ion plots in the results panel).
pub fn trace(fields: &Fields, factors: &[f64], particle: &Particle, sampled: &FlightOptions) -> Result<(FlightResult, Vec<f64>), String> {
    let field = LiveField { fields, factors };
    let result = fly(&field, particle, sampled).map_err(|e| e.to_string())?;
    let potentials = result.trajectory.as_deref().unwrap_or(&[]).iter().map(|s| field.sample(s.position_mm, s.time_us).potential_v).collect();
    Ok((result, potentials))
}

/// The resident bases summed on the CPU (f64), for the trajectories of the drawn subset.
struct LiveField<'a> {
    fields: &'a Fields,
    factors: &'a [f64],
}

impl Field for LiveField<'_> {
    fn sample(&self, p: Vector, t: f64) -> FieldSample {
        let mut s = self.fields.instance.sample(p, t);
        let place = &self.fields.place;
        let q = place.local(p);
        // Field-free outside the array when a workbench box is set (as the instance itself is).
        if self.fields.bounded && self.fields.bases.first().is_some_and(|g| g.status(q) == Some(SplatReason::OutsideGrid)) {
            return s;
        }
        let mut local = [0.0; 3];
        for (grid, &factor) in self.fields.bases.iter().zip(&self.factors[1..]) {
            if factor != 0.0 {
                let b = grid.sample(q, t);
                s.potential_v += factor * b.potential_v;
                for a in 0..3 {
                    local[a] += factor * b.electric_v_per_mm[a];
                }
            }
        }
        let world = place.world_field(local);
        for a in 0..3 {
            s.electric_v_per_mm[a] += world[a];
        }
        s
    }

    fn status(&self, p: Vector) -> Option<SplatReason> {
        self.fields.instance.status(p)
    }

    fn intersection(&self, from: Vector, to: Vector) -> Option<(f64, SplatReason)> {
        self.fields.instance.intersection(from, to)
    }

    fn launch_intersection(&self, from: Vector, to: Vector) -> Option<(f64, SplatReason)> {
        self.fields.instance.launch_intersection(from, to)
    }
}
