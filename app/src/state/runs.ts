import { create } from "zustand";
import { isTauri } from "@tauri-apps/api/core";
import { autoBins, extent, extentAt, heat, histogram, indicesOf, landed, mapWindow, padRange, planeAxes, runOf, select, surfaceKey, type MapAxis, type RunData } from "../results/analysis";
import { flightVelocities, type FlyRequest } from "../scene/backend";
import type { Flight } from "../scene/flight";
import { flyRequest, setFlightListener } from "../scene/simulation";
import { deleteProjectFile, fileSha256, listProjectFiles, readProjectFile, readProjectText, writeProjectFile } from "./projectDir";
import { fieldFingerprint } from "./fieldFingerprint";
import {
  DEFAULT_KEEP_RUNS,
  DEFAULT_RUN_CAP_MB,
  FLIGHT_TOLERANCES,
  RUN_FORMAT,
  RUN_ID,
  RUN_VERSION,
  decodeResults,
  decodeTrajectories,
  encodeResults,
  encodeTrajectories,
  flightFromStored,
  newRunId,
  planStorage,
  pruneRuns,
  runMatches,
  settingsHash,
  type RunInputs,
  type RunMetrics,
  type RunRecord,
  type RunSettings,
} from "./runFormat";
import { useApp, type PaObject } from "./store";
import { useResults } from "../results/store";

/**
 * Run history of the open project. Every completed flight becomes a run: its record (`run.json`:
 * settings, metrics, small plot summaries) and, under the per-run size cap, its per-ion results
 * and drawn trajectories, in `X.simrep.d/runs/<id>/` (state/runFormat.ts has the layout).
 *
 * Untitled projects keep their runs in memory; saving writes them into the new project folder.
 * Live re-flies (a voltage slider being dragged) are recorded once they settle (LIVE_SETTLE_MS
 * without a newer flight), so a drag adds one run, not dozens. At most `keep` runs stay (oldest
 * unpinned ones go first; pinned runs and the runs the project shows are never pruned).
 *
 * Contract for other features (report export, AI): `listRuns()` and `loadRun(id)`.
 */

const LIVE_SETTLE_MS = 1500;
/** Unsaved runs whose flight data stays in memory (older ones keep only their record). */
const MEMORY_FLIGHTS = 12;

export interface RunsConfig {
  /** Runs kept (pinned ones on top of this). */
  keep: number;
  /** Size cap per run, MB: trajectories are dropped first, then the per-ion results. */
  capMb: number;
}

export const DEFAULT_RUNS_CONFIG: RunsConfig = { keep: DEFAULT_KEEP_RUNS, capMb: DEFAULT_RUN_CAP_MB };

/** Small plot data kept in run.json, so a run can be shown (report, history) without its files. */
export interface RunPlots {
  /** TOF of the ions that landed (electrodes, array edges). */
  histogram: { quantity: "tof"; unit: "µs"; lo: number; hi: number; counts: number[] } | null;
  /** 2D density of the end points on the surface most ions landed on (row 0 = low y; square cells, MAP_BINS across the larger side). */
  detectorMap: { surface: string; axes: [MapAxis, MapAxis]; x: [number, number]; y: [number, number]; nx: number; ny: number; counts: number[] } | null;
}

export type StoredRun = RunRecord & { plots: RunPlots };

interface Held {
  flight: Flight;
  velocities: Float64Array | null;
}

interface RunsState {
  /** Project file the runs belong to (null: untitled, in memory). */
  project: string | null;
  config: RunsConfig;
  /** Newest first. */
  runs: StoredRun[];
  /** Runs written to the project folder. */
  onDisk: Record<string, true>;
  /** Run shown by each array (scene object id -> run id), saved in the project file. */
  shown: Record<string, string>;
  /** Selected in the history list (compare takes two). */
  selected: string[];
  /** Runs found stale on open (the array or settings changed since): flown again. */
  stale: Record<string, true>;
}

export const useRuns = create<RunsState>()(() => ({ project: null, config: DEFAULT_RUNS_CONFIG, runs: [], onDisk: {}, shown: {}, selected: [], stale: {} }));

/** Flight data of runs still in memory (unsaved, or flown in this session). */
const held = new Map<string, Held>();
/** Which run a shown flight object is (so a save can point at it). */
const runOfFlight = new WeakMap<Flight, string>();

const runDir = (id: string) => `runs/${id}`;

// ------------------------------------------------------------------------------- contract

/** The project's runs, newest first. */
export async function listRuns(): Promise<StoredRun[]> {
  return useRuns.getState().runs;
}

export interface LoadedRun {
  record: StoredRun;
  settings: RunSettings;
  metrics: RunMetrics;
  histogramData: RunPlots["histogram"];
  detectorMap: RunPlots["detectorMap"];
  trajectoriesAvailable: boolean;
  /** Per-ion end states (null when they were not stored: over the size cap). */
  run: RunData | null;
}

/** One run with its plot data and, when stored, its per-ion results; null when unknown. */
export async function loadRun(id: string): Promise<LoadedRun | null> {
  const record = useRuns.getState().runs.find((r) => r.id === id);
  if (!record) return null;
  const flight = await runFlight(record, false);
  return {
    record,
    settings: record.settings,
    metrics: record.metrics,
    histogramData: record.plots.histogram,
    detectorMap: record.plots.detectorMap,
    trajectoriesAvailable: record.stored.trajectories || Boolean(held.get(id)?.flight.header.points),
    run: flight && runOf(flight),
  };
}

// ------------------------------------------------------------------------- building runs

/** The inputs of a fly request (the complete field fingerprint computed by the caller). */
export async function inputsOf(request: FlyRequest, pa: { path: string; sha256: string }): Promise<RunInputs> {
  const source =
    request.source.kind === "file" ? { ...request.source, sha256: await fileSha256(request.source.path).catch(() => null) } : request.source;
  return {
    pa,
    voltages: Object.fromEntries(request.voltages.map(([n, v]) => [String(n), v])),
    source,
    bounds: request.bounds,
    placement: request.placement,
    durationUs: request.durationUs,
    relativistic: request.relativistic,
    precision: request.precision,
    collision: request.collision,
    draw: request.draw,
    surfaces: request.surfaces,
    tolerances: { ...FLIGHT_TOLERANCES },
  };
}

export function metricsOf(f: Flight): RunMetrics {
  const h = f.header;
  return {
    ions: h.ions,
    arrivals: h.tof.n,
    tof: h.tof,
    keMean: h.keMean,
    splats: Object.fromEntries(h.surfaces.map((s) => [surfaceKey(s.surface), s.count])),
    computeMs: h.flyMs,
  };
}

const MAP_BINS = 48;

export function plotsOf(run: RunData): RunPlots {
  const tof = select(run, "tof", "landed");
  const e = extent(tof);
  let histogramData: RunPlots["histogram"] = null;
  if (e) {
    const [lo, hi] = padRange(e);
    const h = histogram(tof, lo, hi, autoBins(tof.length));
    histogramData = { quantity: "tof", unit: "µs", lo, hi, counts: Array.from(h.counts) };
  }
  let detectorMap: RunPlots["detectorMap"] = null;
  const best = [...run.surfaces].filter((s) => landed(s.surface)).sort((a, b) => b.count - a.count)[0];
  if (best) {
    const key = surfaceKey(best.surface);
    const idx = indicesOf(run, key);
    const axes = planeAxes("auto", best.surface);
    const x = extentAt(run.cols[axes[0]], idx);
    const y = extentAt(run.cols[axes[1]], idx);
    const w = x && y ? mapWindow([{ x, y }], MAP_BINS) : null;
    if (w) {
      const counts = heat(run.cols[axes[0]], run.cols[axes[1]], idx, w.x, w.y, w.nx, w.ny);
      detectorMap = { surface: key, axes, x: w.x, y: w.y, nx: w.nx, ny: w.ny, counts: Array.from(counts) };
    }
  }
  return { histogram: histogramData, detectorMap };
}

async function buildRecord(o: PaObject, request: FlyRequest, flight: Flight, velocities: Float64Array | null): Promise<StoredRun | null> {
  if (!o.path) return null;
  const sha256 = await fieldFingerprint(o.path, o.info, o.fast, request.surfaces);
  const inputs = await inputsOf(request, { path: o.path, sha256 });
  const h = flight.header;
  const app = useApp.getState();
  const settings: RunSettings = {
    inputs,
    executed: { ...h.executed, deviceName: h.device.name, fallback: h.fallback?.code ?? null },
    simrepVersion: app.runtime.kind === "tauri" ? (app.runtime.version ?? null) : null,
    computeMs: h.flyMs,
  };
  const { capMb } = useRuns.getState().config;
  const plan = planStorage(h.ions, flight.points.length / 3, velocities !== null, capMb * 1024 * 1024);
  const seq = Math.max(0, ...useRuns.getState().runs.map((r) => r.seq)) + 1;
  return {
    format: RUN_FORMAT,
    version: RUN_VERSION,
    id: newRunId(),
    seq,
    label: "",
    note: "",
    pinned: false,
    createdAt: new Date().toISOString(),
    array: { name: o.customName ?? o.fileName, path: o.path },
    settings,
    settingsHash: await settingsHash(inputs),
    metrics: metricsOf(flight),
    header: { ...h },
    stored: { results: plan.results, trajectories: plan.trajectories, velocities: plan.results && velocities !== null, bytes: plan.bytes, dropped: plan.dropped },
    plots: plotsOf(runOf(flight)),
  };
}

const recordJson = (r: StoredRun) => JSON.stringify(r, null, 1);

/** Writes a run's files into `project`'s folder (from memory, or copied from `from`'s folder). */
async function writeRun(r: StoredRun, project: string, from: string | null = null): Promise<void> {
  const h = held.get(r.id);
  if (r.stored.results) {
    const results = h ? encodeResults(h.flight, h.velocities) : from ? await readProjectFile(`${runDir(r.id)}/results.bin`, from) : null;
    if (results) await writeProjectFile(`${runDir(r.id)}/results.bin`, results, project);
  }
  if (r.stored.trajectories) {
    const paths = h ? encodeTrajectories(h.flight) : from ? await readProjectFile(`${runDir(r.id)}/trajectories.bin`, from) : null;
    if (paths) await writeProjectFile(`${runDir(r.id)}/trajectories.bin`, paths, project);
  }
  // The record last: a run.json always has its data files next to it.
  await writeProjectFile(`${runDir(r.id)}/run.json`, recordJson(r), project);
}

function hold(id: string, data: Held) {
  held.set(id, data);
  runOfFlight.set(data.flight, id);
  // Bound memory: flights of saved runs can be read back; unsaved ones beyond the bound lose their data.
  const ids = [...held.keys()];
  for (const old of ids.slice(0, Math.max(0, ids.length - MEMORY_FLIGHTS))) held.delete(old);
}

function warn(detail: unknown) {
  useApp.getState().log("warning", { key: "log.runsFailed", params: { detail: detail instanceof Error ? detail.message : String((detail as { detail?: string })?.detail ?? detail) } });
}

/** Adds a finished flight to the history (and the project folder, when the project is saved). */
export async function recordFlight(objectId: string, request: FlyRequest, flight: Flight): Promise<StoredRun | null> {
  const o = useApp.getState().objects.find((x) => x.id === objectId);
  if (o?.kind !== "pa") return null;
  let velocities: Float64Array | null = flight.velocities ?? null;
  if (!velocities) velocities = await flightVelocities(objectId, flight.header.token).catch(() => null);
  const project = useRuns.getState().project;
  const record = await buildRecord(o, request, flight, velocities);
  if (!record) return null;
  hold(record.id, { flight, velocities });
  useRuns.setState((s) => ({ runs: [record, ...s.runs], shown: { ...s.shown, [objectId]: record.id } }));
  if (record.stored.dropped) useApp.getState().log("warning", { key: record.stored.dropped === "trajectories" ? "log.runTrajectoriesDropped" : "log.runResultsDropped", params: { cap: useRuns.getState().config.capMb } });
  if (project && project === useRuns.getState().project) {
    try {
      await writeRun(record, project);
      useRuns.setState((s) => ({ onDisk: { ...s.onDisk, [record.id]: true } }));
    } catch (err) {
      warn(err);
    }
  }
  await prune();
  return record;
}

/** Deletes runs beyond `keep` (oldest unpinned first; shown runs stay). */
export async function prune(): Promise<void> {
  const s = useRuns.getState();
  const gone = pruneRuns(s.runs, s.config.keep, new Set(Object.values(s.shown)));
  if (gone.length) await removeRuns(gone);
}

async function removeRuns(ids: string[]) {
  const s = useRuns.getState();
  const drop = new Set(ids);
  const onDisk = { ...s.onDisk };
  for (const id of ids) {
    held.delete(id);
    if (s.onDisk[id] && s.project) {
      try {
        await deleteProjectFile(runDir(id), s.project);
      } catch (err) {
        warn(err);
      }
    }
    delete onDisk[id];
  }
  useRuns.setState((now) => ({
    runs: now.runs.filter((r) => !drop.has(r.id)),
    onDisk,
    shown: Object.fromEntries(Object.entries(now.shown).filter(([, r]) => !drop.has(r))),
    selected: now.selected.filter((r) => !drop.has(r)),
  }));
}

// ---------------------------------------------------------------------------- history edits

async function patchRun(id: string, patch: Partial<Pick<StoredRun, "label" | "note" | "pinned">>) {
  useRuns.setState((s) => ({ runs: s.runs.map((r) => (r.id === id ? { ...r, ...patch } : r)) }));
  const s = useRuns.getState();
  const r = s.runs.find((x) => x.id === id);
  if (r && s.onDisk[id] && s.project) {
    try {
      await writeProjectFile(`${runDir(id)}/run.json`, recordJson(r), s.project);
    } catch (err) {
      warn(err);
    }
  }
}

export const renameRun = (id: string, label: string) => patchRun(id, { label: label.trim() });
export const setRunNote = (id: string, note: string) => patchRun(id, { note: note.trim() });
export const pinRun = (id: string, pinned: boolean) => patchRun(id, { pinned }).then(prune);
export const deleteRun = (id: string) => removeRuns([id]);

export function toggleSelected(id: string) {
  useRuns.setState((s) => ({ selected: s.selected.includes(id) ? s.selected.filter((x) => x !== id) : [...s.selected, id].slice(-2) }));
}

/** The two selected runs, older first (run A, run B of the comparison). */
export function selectedPair(): [string, string] | null {
  const s = useRuns.getState();
  if (s.selected.length !== 2) return null;
  const at = (id: string) => s.runs.find((r) => r.id === id)?.createdAt ?? "";
  const [a, b] = [...s.selected].sort((x, y) => at(x).localeCompare(at(y)));
  return [a!, b!];
}

export function setRunsConfig(patch: Partial<RunsConfig>) {
  const config = { ...useRuns.getState().config, ...patch };
  config.keep = Math.max(1, Math.round(config.keep));
  config.capMb = Math.max(1, config.capMb);
  useRuns.setState({ config });
  void prune();
}

// ------------------------------------------------------------------------------- reading back

/** A run's flight: from memory, else from its stored files (trajectories only when `paths`). */
async function runFlight(r: StoredRun, paths: boolean): Promise<Flight | null> {
  const h = held.get(r.id);
  if (h) return h.flight;
  const project = useRuns.getState().project;
  if (!r.stored.results || !project) return null;
  const results = await readProjectFile(`${runDir(r.id)}/results.bin`, project);
  if (!results) return null;
  const traj = paths && r.stored.trajectories ? await readProjectFile(`${runDir(r.id)}/trajectories.bin`, project) : null;
  return flightFromStored(r.header, decodeResults(results), traj && decodeTrajectories(traj));
}

/** End states of a run for the compare tab (null when not stored). */
export async function runData(id: string): Promise<RunData | null> {
  return (await loadRun(id))?.run ?? null;
}

/** Reads every run.json of `project` (newest first); unreadable records are skipped with a warning. */
export async function readRuns(project: string): Promise<StoredRun[]> {
  const files = (await listProjectFiles("runs", project)).filter((f) => /^runs\/[^/]+\/run\.json$/.test(f.path));
  const out: StoredRun[] = [];
  for (const f of files) {
    const id = f.path.split("/")[1]!;
    if (!RUN_ID.test(id)) continue;
    try {
      const r = JSON.parse((await readProjectText(f.path, project)) ?? "null") as StoredRun | null;
      if (r?.format === RUN_FORMAT && r.id === id && r.version <= RUN_VERSION) out.push({ ...r, plots: r.plots ?? { histogram: null, detectorMap: null } });
    } catch (err) {
      warn(err);
    }
  }
  return out.sort((a, b) => b.createdAt.localeCompare(a.createdAt));
}

// ------------------------------------------------------------------------- project lifecycle

/** A new or closed project: forget the history (untitled: nothing on disk). */
export function resetRuns(project: string | null, config: RunsConfig = DEFAULT_RUNS_CONFIG) {
  held.clear();
  clearTimers();
  useResults.getState().comparePair(null);
  useRuns.setState({ project, config, runs: [], onDisk: {}, shown: {}, selected: [], stale: {} });
}

/** Opening a project: its stored runs. */
export async function openRuns(project: string, config: RunsConfig): Promise<void> {
  resetRuns(project, config);
  try {
    const runs = await readRuns(project);
    useRuns.setState({ runs, onDisk: Object.fromEntries(runs.map((r) => [r.id, true as const])) });
  } catch (err) {
    warn(err);
  }
}

/**
 * Saving to `project`: runs not yet in its folder are written there (from memory, or copied from
 * the previous project's folder on "save as").
 */
export async function saveRuns(project: string): Promise<void> {
  const s = useRuns.getState();
  const from = s.project && s.project !== project ? s.project : null;
  const written: Record<string, true> = from ? {} : { ...s.onDisk };
  for (const r of [...s.runs].reverse()) {
    if (written[r.id]) continue;
    try {
      await writeRun(r, project, from && s.onDisk[r.id] ? from : null);
      written[r.id] = true;
    } catch (err) {
      warn(err);
    }
  }
  useRuns.setState({ project, onDisk: written });
}

/** The run a shown flight is (project file reference), or null. */
export function shownRun(o: PaObject): string | null {
  const f = o.flight.result;
  if (!f) return null;
  const id = o.flight.stored?.runId ?? runOfFlight.get(f) ?? null;
  return id && useRuns.getState().runs.some((r) => r.id === id) ? id : null;
}

export type RestoreOutcome = "loaded" | "stale" | "missing";

/**
 * Opening a project: shows run `runId` for array `o` if its inputs still match (same field contents,
 * same settings hash); "stale" / "missing" mean the caller flies again.
 */
export async function restoreRun(o: PaObject, runId: string): Promise<RestoreOutcome> {
  const r = useRuns.getState().runs.find((x) => x.id === runId);
  if (!r || !o.path) return "missing";
  const request = flyRequest(o);
  let sha: string;
  try {
    sha = await fieldFingerprint(o.path, o.info, o.fast, request.surfaces);
  } catch {
    useRuns.setState((s) => ({ stale: { ...s.stale, [r.id]: true } }));
    return "stale";
  }
  const hash = await settingsHash(await inputsOf(request, { path: o.path, sha256: sha }));
  const verdict = runMatches(r, sha, hash);
  if (verdict === "stale") useRuns.setState((s) => ({ stale: { ...s.stale, [r.id]: true } }));
  if (verdict !== "match") return verdict === "stale" ? "stale" : "missing";
  const flight = await runFlight(r, true).catch(() => null);
  if (!flight) return "missing";
  runOfFlight.set(flight, r.id);
  useApp.getState().updatePa(o.id, () => ({ flight: { status: "done", result: flight, error: null, stored: { runId: r.id } } }));
  useRuns.setState((s) => ({ shown: { ...s.shown, [o.id]: r.id } }));
  return "loaded";
}

// --------------------------------------------------------------------------- flight listener

const timers = new Map<string, ReturnType<typeof setTimeout>>();
function clearTimers() {
  for (const t of timers.values()) clearTimeout(t);
  timers.clear();
}

if (isTauri()) {
  setFlightListener((id, request, flight, live) => {
    const pending = timers.get(id);
    if (pending) clearTimeout(pending);
    timers.delete(id);
    const run = () => void recordFlight(id, request, flight).catch(warn);
    if (!live) return run();
    timers.set(id, setTimeout(() => {
      timers.delete(id);
      // Only the flight still shown is recorded (a newer one replaced it otherwise).
      const o = useApp.getState().objects.find((x) => x.id === id);
      if (o?.kind === "pa" && o.flight.result === flight) run();
    }, LIVE_SETTLE_MS));
  });
}
