//! Refine (solve Laplace) for an open .pa# / .pa in a background thread, with live progress.
//!
//! Same physics and file conventions as `simrep refine` (crates/simrep-cli/src/refine_cmd.rs):
//! a .pa# yields one solution per adjustable electrode at SIMION's 10 000 V reference (.pa1..N,
//! .pa_ for unnumbered electrodes) plus the .pa0 base; a .pa is solved in place.
//!
//! The multigrid solvers report only when they finish, so the solve runs in warm-started chunks of
//! V-cycles: 1 cycle (a rough field almost at once), 2 more, then chunks sized from the measured
//! per-call overhead and per-cycle cost so updates keep coming about every `PREVIEW_PERIOD` while
//! the overhead stays a small fraction of the work. After each chunk the UI gets the residual
//! history and the current potential, so the section sharpens while it converges. Messages go over a Tauri channel:
//! JSON progress messages, and binary previews `"SPV1" | u32 stage | f32 potentials` (PA layout).
//! Files are written only after every solution converged; cancel leaves the disk untouched.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use simrep_core::{PotentialArray, Symmetry};
use simrep_io::{Pa0Metadata, PaFile, write_pa};
use simrep_solve::cutcell::CutCells;
use simrep_solve::gpu::{Backends, Gpu};
use simrep_solve::mg::{self, Geometry, Grid, Options, Residual};
use tauri::State;
use tauri::ipc::{Channel, InvokeResponseBody};

use crate::pa::{self, Origin, PaError, Reason};
use crate::session::{AppState, FastAdjust, Session};

// SIMION's PA1..PAn convention is 10000 V, not 1 V.
const REFERENCE_VOLTAGE: f64 = 10_000.0;
/// Relative infinity norm of the f64 Laplace residual (the CLI default).
const TOLERANCE: f64 = 1e-10;
const MAX_CYCLES: usize = 200;
/// Target spacing of progress updates after the first two chunks.
const PREVIEW_PERIOD: Duration = Duration::from_millis(1500);
/// Later chunks do at least this many times the per-call overhead (setup, upload) in cycles.
const OVERHEAD_FACTOR: f64 = 4.0;
/// Minimum spacing of potential previews (the first chunk of each solution is always sent).
const PREVIEW_INTERVAL: Duration = Duration::from_millis(120);
/// "Auto" uses the GPU from this many points: below it, GPU setup outweighs the solve.
const AUTO_GPU_POINTS: usize = 1 << 18;

#[derive(Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum DeviceChoice {
    Auto,
    Gpu,
    Cpu,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RefineRequest {
    id: String,
    output_dir: Option<String>,
    device: DeviceChoice,
    /// (electrode number, V) written into the .pa0 of a .pa# (default: every electrode at 0 V).
    #[serde(default)]
    voltages: Vec<(usize, f64)>,
}

#[derive(Serialize, Clone)]
pub struct Device {
    pub kind: &'static str,
    pub name: String,
    /// CPU threads (CPU devices only): the UI names a CPU device in its own language.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub threads: Option<usize>,
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
enum Message {
    #[serde(rename_all = "camelCase")]
    Start { device: Device, stages: Vec<Option<usize>>, points: usize },
    #[serde(rename_all = "camelCase")]
    Progress { stage: usize, cycle: usize, residual: f64, absolute: f64, elapsed_ms: f64 },
    #[serde(rename_all = "camelCase")]
    StageDone { stage: usize, cycles: usize, residual: f64 },
    #[serde(rename_all = "camelCase")]
    Device { device: Device, reason: Reason },
    #[serde(rename_all = "camelCase")]
    Done { files: Vec<String>, dir: String, ms: f64, cycles: usize, residual: f64, device: Device },
    Cancelled,
    /// `reason`: a condition the UI states in its language; otherwise `message` is technical text.
    Error { message: String, reason: Option<Reason> },
}

enum Fail {
    Cancelled,
    Error(String),
    Reason(Reason),
}

impl<E: std::fmt::Display> From<E> for Fail {
    fn from(e: E) -> Self {
        Fail::Error(e.to_string())
    }
}

fn send(channel: &Channel<InvokeResponseBody>, message: &Message) {
    if let Ok(json) = serde_json::to_string(message) {
        // A closed window drops the channel; the job then just runs to its end.
        let _ = channel.send(InvokeResponseBody::Json(json));
    }
}

// ------------------------------------------------------------------------- SIMION conventions

/// SIMION stores cylindrical arrays axial-x / radial-y; the solver wants radial-x / axial-y.
struct Layout {
    grid: Grid,
    fixed: Vec<bool>,
    cylindrical: bool,
}

fn transpose<T: Copy>(values: &[T], nx: usize, ny: usize) -> Vec<T> {
    (0..values.len()).map(|i| values[i / ny + nx * (i % ny)]).collect()
}

fn antimirror_plane(a: &PotentialArray, i: usize) -> bool {
    (a.anti_mirrors.x && i.is_multiple_of(a.nx))
        || (a.anti_mirrors.y && (i / a.nx).is_multiple_of(a.ny))
        || (a.anti_mirrors.z && i / (a.nx * a.ny) == 0)
}

impl Layout {
    fn new(a: &PotentialArray) -> Self {
        let cylindrical = a.symmetry == Symmetry::Cylindrical;
        let mut fixed = a.electrodes.clone();
        for (i, value) in fixed.iter_mut().enumerate() {
            if antimirror_plane(a, i) {
                *value = true;
            }
        }
        if cylindrical {
            fixed = transpose(&fixed, a.nx, a.ny);
        }
        let grid = if cylindrical {
            Grid { dims: [a.ny, a.nx, 1], spacing: [a.dy_mm, a.dx_mm, a.dz_mm], geometry: Geometry::Cylindrical { radial_origin: 0.0 } }
        } else {
            Grid { dims: [a.nx, a.ny, a.nz], spacing: [a.dx_mm, a.dy_mm, a.dz_mm], geometry: Geometry::Planar }
        };
        Self { grid, fixed, cylindrical }
    }

    /// Solver-layout values for `a` (anti-mirror planes forced to 0 V).
    fn values(&self, a: &PotentialArray) -> Result<Vec<f64>, Fail> {
        let mut p = a.potentials.clone();
        for (i, v) in p.iter_mut().enumerate() {
            if antimirror_plane(a, i) {
                if a.electrodes[i] && *v != 0.0 {
                    return Err(Fail::Reason(Reason::new("antiMirrorElectrode")));
                }
                *v = 0.0;
            }
        }
        Ok(if self.cylindrical { transpose(&p, a.nx, a.ny) } else { p })
    }

    /// Back to PA layout.
    fn to_pa(&self, values: &[f64], a: &PotentialArray) -> Vec<f64> {
        if self.cylindrical { transpose(values, a.ny, a.nx) } else { values.to_vec() }
    }
}

fn electrode_number(value: f64) -> Option<usize> {
    ((1.0..=1000.0).contains(&value) && value.fract() == 0.0).then_some(value as usize)
}

/// PA0 table: first grid index of each numbered electrode (slot 0 = unnumbered, largest |V|).
fn electrode_table(a: &PotentialArray) -> Vec<i64> {
    let mut indices = vec![-1; 1001];
    for (i, (&value, &fixed)) in a.potentials.iter().zip(&a.electrodes).enumerate() {
        if fixed {
            let slot = electrode_number(value).unwrap_or(0);
            if indices[slot] == -1 || (slot == 0 && value.abs() > a.potentials[indices[slot] as usize].abs()) {
                indices[slot] = i as i64;
            }
        }
    }
    let entries = indices.iter().rposition(|&i| i != -1).map_or(32, |i| (i + 1).max(32));
    indices.truncate(entries);
    indices
}

fn basis_values(source: &PaFile, slot: usize, scalable_voltage: f64) -> Vec<f64> {
    source
        .array
        .potentials
        .iter()
        .zip(&source.array.electrodes)
        .map(|(&input, &fixed)| {
            if !fixed {
                0.0
            } else if slot == 0 && electrode_number(input).is_none() {
                if scalable_voltage == 0.0 { 0.0 } else { (input / scalable_voltage) * REFERENCE_VOLTAGE }
            } else if electrode_number(input) == Some(slot) {
                REFERENCE_VOLTAGE
            } else {
                0.0
            }
        })
        .collect()
}

fn is_pa_hash(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("pa#"))
}

/// Solutions a refine produces: electrode slots for a .pa#, `[None]` for a .pa.
fn stages(session: &Session) -> Vec<Option<usize>> {
    if is_pa_hash(&session.path) {
        let table = electrode_table(&session.file.array);
        (0..table.len()).filter(|&s| table[s] != -1).map(Some).collect()
    } else {
        vec![None]
    }
}

/// Output file for a stage (`None` = the refined .pa itself), plus the .pa0 for a .pa#.
fn output_path(session: &Session, dir: &Path, stage: Option<usize>) -> PathBuf {
    let name = PathBuf::from(pa::file_name(&session.path));
    match stage {
        None if is_pa_hash(&session.path) => dir.join(name.with_extension("pa0")),
        None => dir.join(name),
        Some(0) => dir.join(name.with_extension("pa_")),
        Some(slot) => dir.join(name.with_extension(format!("pa{slot}"))),
    }
}

fn outputs(session: &Session, dir: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = stages(session).into_iter().map(|s| output_path(session, dir, s)).collect();
    if is_pa_hash(&session.path) {
        out.push(output_path(session, dir, None));
    }
    out.extend(moved_sidecar(session, dir).map(|(_, to)| to));
    out
}

/// A .pa#'s cut-cell sidecar (import wizard) and its copy next to the .pa0 when the refine writes
/// to another folder, so the refined array still flies against its exact surfaces (session.rs).
fn moved_sidecar(session: &Session, dir: &Path) -> Option<(PathBuf, PathBuf)> {
    if !is_pa_hash(&session.path) {
        return None;
    }
    let from = session.path.with_extension("cuts.json");
    let to = output_path(session, dir, None).with_extension("cuts.json");
    let same_dir = match (from.parent().map(std::fs::canonicalize), std::fs::canonicalize(dir)) {
        (Some(Ok(a)), Ok(b)) => a == b,
        _ => true,
    };
    (from.is_file() && !same_dir).then_some((from, to))
}

fn resolve_dir(session: &Session, requested: Option<String>) -> PathBuf {
    requested.map(PathBuf::from).unwrap_or_else(|| session.path.parent().map(Path::to_path_buf).unwrap_or_default())
}

fn check(session: &Session) -> Result<(), PaError> {
    let ext = session.path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    if ext != "pa#" && ext != "pa" {
        return Err(PaError::coded("refine", Reason::new("refineFormat")));
    }
    if !session.file.array.refinable {
        return Err(PaError::coded("refine", Reason::new("refineFixedMap")));
    }
    if !session.file.array.electrodes.iter().any(|&e| e) {
        return Err(PaError::coded("refine", Reason::new("refineNoElectrodes")));
    }
    Ok(())
}

// ------------------------------------------------------------------------------- commands

#[derive(Serialize)]
pub struct Target {
    name: String,
    exists: bool,
}

#[derive(Serialize)]
pub struct Targets {
    dir: String,
    files: Vec<Target>,
    /// Solved with the cut-cell operator (a .pa# built by the import wizard, geometry.rs).
    cutcell: bool,
}

/// Files a refine would write into `output_dir` (default: next to the source), and which exist.
#[tauri::command]
pub fn refine_targets(state: State<'_, AppState>, id: String, output_dir: Option<String>) -> Result<Targets, PaError> {
    let session = state.get(&id).ok_or_else(|| PaError::new("noSession", id))?;
    check(&session)?;
    let dir = resolve_dir(&session, output_dir);
    let files = outputs(&session, &dir).iter().map(|p| Target { name: pa::file_name(p), exists: p.exists() }).collect();
    let cutcell = state.geometries.lock().unwrap().contains_key(&session.path);
    Ok(Targets { dir: dir.display().to_string(), files, cutcell })
}

#[tauri::command]
pub fn refine_start(state: State<'_, AppState>, request: RefineRequest, channel: Channel<InvokeResponseBody>) -> Result<u32, PaError> {
    let session = state.get(&request.id).ok_or_else(|| PaError::new("noSession", request.id.clone()))?;
    check(&session)?;
    let dir = resolve_dir(&session, request.output_dir);
    if !dir.is_dir() {
        return Err(PaError::coded("refine", Reason::new("refineNoFolder").with("dir", dir.display().to_string())));
    }
    let state = state.inner().clone();
    let job = state.next_job.fetch_add(1, Ordering::Relaxed) + 1;
    let cancel = Arc::new(AtomicBool::new(false));
    state.jobs.lock().unwrap().insert(job, cancel.clone());
    std::thread::Builder::new()
        .name(format!("refine-{job}"))
        .spawn(move || {
            match run(&state, &request.id, &session, &dir, request.device, &request.voltages, &cancel, &channel) {
                Ok(()) => {}
                Err(Fail::Cancelled) => send(&channel, &Message::Cancelled),
                Err(Fail::Error(message)) => send(&channel, &Message::Error { message, reason: None }),
                Err(Fail::Reason(reason)) => send(&channel, &Message::Error { message: reason.code.to_owned(), reason: Some(reason) }),
            }
            state.jobs.lock().unwrap().remove(&job);
        })
        .map_err(|e| PaError::new("refine", e.to_string()))?;
    Ok(job)
}

#[tauri::command]
pub fn refine_cancel(state: State<'_, AppState>, job: u32) {
    if let Some(flag) = state.jobs.lock().unwrap().get(&job) {
        flag.store(true, Ordering::Relaxed);
    }
}

// ------------------------------------------------------------------------------------ run

pub fn cpu_device() -> Device {
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get());
    Device { kind: "cpu", name: format!("CPU · {threads} threads"), threads: Some(threads) }
}

fn gpu_device(gpu: &Gpu) -> Device {
    Device { kind: "gpu", name: format!("{} · Vulkan", gpu.info.name), threads: None }
}

pub fn probe_gpu() -> Result<Arc<Gpu>, Reason> {
    match pollster::block_on(Gpu::new(Backends::VULKAN)) {
        Ok(Some(gpu)) => Ok(Arc::new(gpu)),
        Ok(None) => Err(Reason::new("noVulkan")),
        Err(e) => Err(Reason::new("gpuInit").detail(e)),
    }
}

/// The operator of one solution: staircase electrodes (the PA flags) or cut cells.
enum Operator<'a> {
    Staircase,
    Cut(&'a CutCells),
}

struct Solver<'a> {
    gpu: Option<Arc<Gpu>>,
    channel: &'a Channel<InvokeResponseBody>,
    cancel: &'a AtomicBool,
    start: Instant,
}

impl Solver<'_> {
    fn device(&self) -> Device {
        self.gpu.as_deref().map_or_else(cpu_device, gpu_device)
    }

    /// One solution, progressively. `preview` gets the solver-layout values after each chunk.
    fn solve(&mut self, stage: usize, values: &mut [f64], layout: &Layout, operator: &Operator, mut preview: impl FnMut(&[f64])) -> Result<(usize, Residual), Fail> {
        let mut done = 0;
        let mut chunk = 1;
        let mut calls: Vec<(usize, f64)> = Vec::new(); // (cycles, seconds) of each call
        let mut last_preview: Option<Instant> = None;
        loop {
            let options = Options { relative_tolerance: TOLERANCE, max_cycles: chunk, ..Options::default() };
            let started = Instant::now();
            let (converged, iterations, history) = match self.gpu.clone() {
                Some(gpu) => match match operator {
                    Operator::Staircase => gpu.solve(values, &layout.fixed, layout.grid, options),
                    Operator::Cut(cuts) => gpu.solve_sdf(values, cuts, options),
                } {
                    Ok(r) => (r.converged, r.iterations, r.residual_history),
                    Err(e) => {
                        // e.g. the array exceeds a GPU buffer limit: finish on the CPU.
                        self.gpu = None;
                        send(self.channel, &Message::Device { device: self.device(), reason: Reason::new("gpuSolveFailed").detail(e) });
                        continue;
                    }
                },
                None => {
                    let r = match operator {
                        Operator::Staircase => mg::solve(values, &layout.fixed, layout.grid, options)?,
                        Operator::Cut(cuts) => cuts.solve(values, options)?,
                    };
                    (r.converged, r.iterations, r.residual_history)
                }
            };
            let first = usize::from(done > 0); // later chunks repeat the previous final residual
            for (k, r) in history.iter().enumerate().skip(first) {
                let elapsed_ms = self.start.elapsed().as_secs_f64() * 1e3;
                send(self.channel, &Message::Progress { stage, cycle: done + k, residual: r.relative, absolute: r.absolute, elapsed_ms });
            }
            done += iterations;
            let last = *history.last().ok_or("solver returned no residual")?;
            if converged {
                return Ok((done, last));
            }
            if self.cancel.load(Ordering::Relaxed) {
                return Err(Fail::Cancelled);
            }
            if last_preview.is_none_or(|t| t.elapsed() >= PREVIEW_INTERVAL) {
                preview(values);
                last_preview = Some(Instant::now());
            }
            if done >= MAX_CYCLES {
                return Err(Fail::Reason(Reason::new("noConverge").with("cycles", done).with("residual", format!("{:.2e}", last.relative))));
            }
            calls.push((iterations, started.elapsed().as_secs_f64()));
            chunk = match calls.as_slice() {
                [_] => 2,
                [(c1, t1), .., (c2, t2)] => {
                    // Per-cycle cost and per-call overhead from the first and the latest call.
                    let per_cycle = if c2 > c1 { ((t2 - t1) / (c2 - c1) as f64).max(1e-6) } else { (t2 / (*c2).max(1) as f64).max(1e-6) };
                    let overhead = (t1 - per_cycle * *c1 as f64).max(0.0);
                    let wanted = (OVERHEAD_FACTOR * overhead).max(PREVIEW_PERIOD.as_secs_f64()) / per_cycle;
                    wanted.ceil().clamp(1.0, MAX_CYCLES as f64) as usize
                }
                [] => 1,
            };
            chunk = chunk.min(MAX_CYCLES - done);
        }
    }
}

fn preview_packet(stage: usize, potentials: &[f64]) -> Vec<u8> {
    let mut out = Vec::with_capacity(8 + potentials.len() * 4);
    out.extend_from_slice(b"SPV1");
    out.extend_from_slice(&(stage as u32).to_le_bytes());
    for &p in potentials {
        out.extend_from_slice(&(p as f32).to_le_bytes());
    }
    out
}

#[allow(clippy::too_many_arguments)]
fn run(state: &AppState, id: &str, session: &Session, dir: &Path, choice: DeviceChoice, voltages: &[(usize, f64)], cancel: &AtomicBool, channel: &Channel<InvokeResponseBody>) -> Result<(), Fail> {
    let start = Instant::now();
    let small = session.file.array.potentials.len() < AUTO_GPU_POINTS;
    let gpu = match choice {
        DeviceChoice::Cpu => None,
        DeviceChoice::Auto if small => None,
        DeviceChoice::Auto | DeviceChoice::Gpu => match state.gpu.get_or_init(probe_gpu) {
            Ok(gpu) => Some(gpu.clone()),
            Err(reason) => {
                send(channel, &Message::Device { device: cpu_device(), reason: reason.clone() });
                None
            }
        },
    };
    let mut solver = Solver { gpu, channel, cancel, start };
    let source = &session.file;
    let a = &source.array;
    let layout = Layout::new(a);
    let stage_list = stages(session);
    let prepared = if is_pa_hash(&session.path) { state.geometries.lock().unwrap().get(&session.path).cloned() } else { None };
    send(channel, &Message::Start { device: solver.device(), stages: stage_list.clone(), points: a.potentials.len() });

    let mut total_cycles = 0;
    let mut worst = 0.0_f64;
    let mut solutions: Vec<(Option<usize>, Vec<f64>)> = Vec::new();
    let scalable_voltage = {
        let table = electrode_table(a);
        if table[0] == -1 { 0.0 } else { a.potentials[table[0] as usize] }
    };
    for (stage, &slot) in stage_list.iter().enumerate() {
        let initial = match slot {
            Some(slot) => PotentialArray { potentials: basis_values(source, slot, scalable_voltage), ..a.clone() },
            None => a.clone(),
        };
        // Cut-cell bases start from the operator's own boundary values (solver layout).
        let cuts = match (&prepared, slot) {
            (Some(p), Some(n)) if n > 0 => Some(p.cut_cells(Some(n))?),
            (Some(_), _) => return Err(Fail::Error("cut-cell geometry has an unnumbered electrode".into())),
            (None, _) => None,
        };
        let mut values = match &cuts {
            Some(c) => c.values.clone(),
            None => layout.values(&initial)?,
        };
        let operator = cuts.as_ref().map_or(Operator::Staircase, Operator::Cut);
        let (cycles, residual) = solver.solve(stage, &mut values, &layout, &operator, |v| {
            let _ = channel.send(InvokeResponseBody::Raw(preview_packet(stage, &layout.to_pa(v, a))));
        })?;
        let solved = layout.to_pa(&values, a);
        let _ = channel.send(InvokeResponseBody::Raw(preview_packet(stage, &solved)));
        send(channel, &Message::StageDone { stage, cycles, residual: residual.relative });
        total_cycles += cycles;
        worst = worst.max(residual.relative);
        solutions.push((slot, solved));
    }

    // Everything converged: write the files.
    let mut written = Vec::new();
    let (path, file, fast) = if is_pa_hash(&session.path) {
        let mut base = source.clone();
        base.pa0 = None;
        let mut basis = source.clone();
        basis.pa0 = None;
        basis.max_voltage = source.max_voltage.max(100_000.0);
        base.array.potentials = vec![0.0; a.potentials.len()];
        for (slot, potentials) in &solutions {
            let slot = slot.expect("pa# stages are electrode slots");
            basis.array.potentials.clone_from(potentials);
            let out = output_path(session, dir, Some(slot));
            write_pa(&out, &basis).map_err(|e| format!("{}: {e}", out.display()))?;
            written.push(pa::file_name(&out));
            if slot == 0 {
                for (i, b) in base.array.potentials.iter_mut().enumerate() {
                    *b = if a.electrodes[i] {
                        let input = a.potentials[i];
                        if electrode_number(input).is_none() { input } else { 0.0 }
                    } else {
                        potentials[i] * (scalable_voltage / REFERENCE_VOLTAGE)
                    };
                }
            }
        }
        // Requested electrode voltages go into the .pa0 (fast adjust then starts from them).
        for (slot, potentials) in &solutions {
            if let Some(&(_, v)) = voltages.iter().find(|(n, _)| Some(*n) == *slot && *n > 0) {
                base.array.potentials.iter_mut().zip(potentials).for_each(|(b, p)| *b += p * (v / REFERENCE_VOLTAGE));
            }
        }
        base.pa0 = Some(Pa0Metadata { reference_voltage: REFERENCE_VOLTAGE, electrode_indices: electrode_table(a), scalable: None });
        let out = output_path(session, dir, None);
        write_pa(&out, &base).map_err(|e| format!("{}: {e}", out.display()))?;
        written.push(pa::file_name(&out));
        if let Some((from, to)) = moved_sidecar(session, dir) {
            std::fs::copy(&from, &to).map_err(|e| format!("{}: {e}", to.display()))?;
            written.push(pa::file_name(&to));
        }
        let fast = FastAdjust::new(&base, solutions.into_iter().map(|(s, p)| (s.unwrap_or(0), p)).collect())?;
        (out, base, Some(fast))
    } else {
        let mut refined = source.clone();
        refined.array.potentials = solutions.pop().map(|(_, p)| p).unwrap_or_default();
        let out = output_path(session, dir, None);
        write_pa(&out, &refined).map_err(|e| format!("{}: {e}", out.display()))?;
        written.push(pa::file_name(&out));
        (out, refined, None)
    };
    let file_size = std::fs::metadata(&path).map_or(0, |m| m.len());
    let status = fast.as_ref().map(FastAdjust::status);
    let packet = pa::packet(&file, &file.array, Origin { path: &path, file_size, fast: status }).map_err(|e| format!("{e:?}"))?;
    state.insert(id.to_owned(), Arc::new(Session::new(path, file, fast)));
    let _ = channel.send(InvokeResponseBody::Raw(packet));
    send(
        channel,
        &Message::Done {
            files: written,
            dir: dir.display().to_string(),
            ms: start.elapsed().as_secs_f64() * 1e3,
            cycles: total_cycles,
            residual: worst,
            device: solver.device(),
        },
    );
    Ok(())
}
