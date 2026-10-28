import type { FlySource } from "../scene/backend";
import type { Flight, FlightHeader } from "../scene/flight";

/**
 * Pure parts of stored flight runs (state/runs.ts): the run.json record, the compact binary files
 * next to it, storage planning under a size cap, history pruning and the settings comparison.
 * Nothing here touches the store, the native core or the disk, so it is unit tested directly.
 *
 * `runs/<id>/results.bin` (SRR1, little endian):
 *   0  "SRR1"   4  u32 version (1)   8  u32 ions n   12  u32 flags (bit 0: velocities present)
 *   16 f64[4n]  x, y, z mm, TOF us per ion
 *      f64[n]   final kinetic energy, eV
 *      f64[3n]  final vx, vy, vz mm/us (flag bit 0 only)
 *      u32[n]   index into header.surfaces per ion
 * `runs/<id>/trajectories.bin` (SRT1): the drawn (already decimated) paths as the viewer holds them:
 *   0  "SRT1"   4  u32 version (1)   8  u32 ions n   12  u32 points P
 *   16 u32[n+1] offsets into the points (in points)
 *      f32[3P]  x, y, z mm
 */

export const RUN_FORMAT = "simrep-run";
export const RUN_VERSION = 1;
export const DEFAULT_KEEP_RUNS = 50;
export const DEFAULT_RUN_CAP_MB = 200;

/** End-state integration settings of every flight (src-tauri/src/fly.rs `terminal_options`). */
export const FLIGHT_TOLERANCES = { relativeTolerance: 1e-8, positionToleranceMm: 1e-8, velocityToleranceMmPerUs: 1e-8, initialStepUs: 0.001, maxStepUs: 0.1 } as const;

/** What a flight was flown from; the settings hash covers exactly this. */
export interface RunInputs {
  pa: { path: string; sha256: string };
  voltages: Record<string, number>;
  /** The flight request's source (beam parameters, or the particle file with its sha256). */
  source: FlySource & { sha256?: string | null };
  bounds: [number, number, number, number, number, number] | null;
  placement: unknown;
  durationUs: number;
  relativistic: boolean;
  /** Requested mode (live / precise). */
  precision: string;
  collision: string;
  /** Trajectories drawn. */
  draw: number;
  /** Exact surfaces requested. */
  surfaces: boolean;
  tolerances: typeof FLIGHT_TOLERANCES;
}

export interface RunSettings {
  inputs: RunInputs;
  /** How it actually ran (a live request can run on the CPU). */
  executed: FlightHeader["executed"] & { deviceName: string; fallback: string | null };
  simrepVersion: string | null;
  computeMs: number;
}

export interface RunMetrics {
  ions: number;
  /** Ions that ended on an electrode or an array edge (TOF statistics cover these). */
  arrivals: number;
  tof: FlightHeader["tof"];
  keMean: number;
  /** End surface (analysis.surfaceKey) -> ions. */
  splats: Record<string, number>;
  computeMs: number;
}

export interface RunRecord {
  format: typeof RUN_FORMAT;
  version: typeof RUN_VERSION;
  id: string;
  /** Running number in the project (default label "Run N"). */
  seq: number;
  /** User label; "" = the default. */
  label: string;
  note: string;
  pinned: boolean;
  createdAt: string;
  array: { name: string; path: string };
  settings: RunSettings;
  settingsHash: string;
  metrics: RunMetrics;
  header: FlightHeader;
  stored: { results: boolean; trajectories: boolean; velocities: boolean; bytes: number; dropped: "trajectories" | "results" | null };
}

// ------------------------------------------------------------------------------- hashing

/** JSON with object keys sorted at every level (a stable text to hash). */
export function canonicalJson(v: unknown): string {
  if (v === null || typeof v !== "object") return JSON.stringify(v ?? null);
  if (Array.isArray(v)) return `[${v.map(canonicalJson).join(",")}]`;
  const o = v as Record<string, unknown>;
  return `{${Object.keys(o)
    .filter((k) => o[k] !== undefined)
    .sort()
    .map((k) => `${JSON.stringify(k)}:${canonicalJson(o[k])}`)
    .join(",")}}`;
}

export async function sha256Hex(data: string | Uint8Array<ArrayBuffer>): Promise<string> {
  const bytes = typeof data === "string" ? new TextEncoder().encode(data) : data;
  const digest = await crypto.subtle.digest("SHA-256", bytes);
  return Array.from(new Uint8Array(digest), (b) => b.toString(16).padStart(2, "0")).join("");
}

/** Hash of everything a flight depends on except the array contents (`pa.sha256` is compared on its own). */
export function settingsHash(inputs: RunInputs): Promise<string> {
  // The path may differ between computers; the array's contents (sha256) decide.
  const { pa, ...rest } = inputs;
  return sha256Hex(canonicalJson({ ...rest, pa: { sha256: pa.sha256 } }));
}

/** Whether a stored run can stand in for flying `inputs` now. */
export function runMatches(record: Pick<RunRecord, "settings" | "settingsHash" | "stored">, paSha256: string, hash: string): "match" | "stale" | "noResults" {
  if (record.settings.inputs.pa.sha256 !== paSha256 || record.settingsHash !== hash) return "stale";
  return record.stored.results ? "match" : "noResults";
}

// ----------------------------------------------------------------------------- binary files

const HEADER = 16;
const magic = (b: Uint8Array, m: string) => String.fromCharCode(...b.subarray(0, 4)) === m;

export function resultsBytes(n: number, velocities: boolean): number {
  return HEADER + n * (32 + 8 + (velocities ? 24 : 0) + 4);
}

export function trajectoryBytes(n: number, points: number): number {
  return HEADER + 4 * (n + 1) + 12 * points;
}

export function encodeResults(f: Pick<Flight, "ends" | "ke" | "surface">, velocities: Float64Array | null): Uint8Array<ArrayBuffer> {
  const n = f.surface.length;
  const buf = new ArrayBuffer(resultsBytes(n, velocities !== null));
  const bytes = new Uint8Array(buf);
  bytes.set(new TextEncoder().encode("SRR1"));
  const dv = new DataView(buf);
  dv.setUint32(4, 1, true);
  dv.setUint32(8, n, true);
  dv.setUint32(12, velocities ? 1 : 0, true);
  let at = HEADER;
  new Float64Array(buf, at, 4 * n).set(f.ends.subarray(0, 4 * n));
  at += 32 * n;
  new Float64Array(buf, at, n).set(f.ke.subarray(0, n));
  at += 8 * n;
  if (velocities) {
    new Float64Array(buf, at, 3 * n).set(velocities.subarray(0, 3 * n));
    at += 24 * n;
  }
  new Uint32Array(buf, at, n).set(f.surface.subarray(0, n));
  return bytes;
}

export interface StoredResults {
  ends: Float64Array<ArrayBuffer>;
  ke: Float64Array<ArrayBuffer>;
  surface: Uint32Array<ArrayBuffer>;
  velocities: Float64Array<ArrayBuffer> | null;
}

export function decodeResults(bytes: Uint8Array<ArrayBuffer>): StoredResults {
  // Copy into an aligned buffer of its own (a view may start anywhere).
  const buf = bytes.slice().buffer;
  const b = new Uint8Array(buf);
  if (b.length < HEADER || !magic(b, "SRR1")) throw new Error("not a SimRep run results file");
  const dv = new DataView(buf);
  const n = dv.getUint32(8, true);
  const hasV = (dv.getUint32(12, true) & 1) === 1;
  if (b.length < resultsBytes(n, hasV)) throw new Error("run results file is truncated");
  let at = HEADER;
  const ends = new Float64Array(buf, at, 4 * n);
  at += 32 * n;
  const ke = new Float64Array(buf, at, n);
  at += 8 * n;
  let velocities: Float64Array<ArrayBuffer> | null = null;
  if (hasV) {
    velocities = new Float64Array(buf, at, 3 * n);
    at += 24 * n;
  }
  const surface = new Uint32Array(buf, at, n);
  return { ends, ke, surface, velocities };
}

export function encodeTrajectories(f: Pick<Flight, "offsets" | "points">): Uint8Array<ArrayBuffer> {
  const n = f.offsets.length - 1;
  const p = f.points.length / 3;
  const buf = new ArrayBuffer(trajectoryBytes(n, p));
  new Uint8Array(buf).set(new TextEncoder().encode("SRT1"));
  const dv = new DataView(buf);
  dv.setUint32(4, 1, true);
  dv.setUint32(8, n, true);
  dv.setUint32(12, p, true);
  new Uint32Array(buf, HEADER, n + 1).set(f.offsets);
  new Float32Array(buf, HEADER + 4 * (n + 1), 3 * p).set(f.points);
  return new Uint8Array(buf);
}

export function decodeTrajectories(bytes: Uint8Array<ArrayBuffer>): { offsets: Uint32Array<ArrayBuffer>; points: Float32Array<ArrayBuffer> } {
  const buf = bytes.slice().buffer;
  const b = new Uint8Array(buf);
  if (b.length < HEADER || !magic(b, "SRT1")) throw new Error("not a SimRep run trajectories file");
  const dv = new DataView(buf);
  const n = dv.getUint32(8, true);
  const p = dv.getUint32(12, true);
  if (b.length < trajectoryBytes(n, p)) throw new Error("run trajectories file is truncated");
  return { offsets: new Uint32Array(buf, HEADER, n + 1), points: new Float32Array(buf, HEADER + 4 * (n + 1), 3 * p) };
}

/** No trajectory points: every ion's range is empty (end points and statistics only). */
export function emptyTrajectories(n: number) {
  return { offsets: new Uint32Array(n + 1), points: new Float32Array(0) };
}

/** A flight rebuilt from its stored header and files. */
export function flightFromStored(header: FlightHeader, r: StoredResults, t: { offsets: Uint32Array<ArrayBuffer>; points: Float32Array<ArrayBuffer> } | null): Flight {
  const tr = t ?? emptyTrajectories(header.ions);
  const drawn = t ? header.drawn : 0;
  return {
    header: { ...header, drawn, points: tr.points.length / 3 },
    offsets: tr.offsets,
    points: tr.points,
    ends: r.ends,
    ke: r.ke,
    surface: r.surface,
    velocities: r.velocities ?? undefined,
    roundTripMs: 0,
  };
}

// ------------------------------------------------------------------------- storage planning

/**
 * What of a run fits under `capBytes`: everything, the results without trajectories (dropped
 * first), or only the record (metrics and settings).
 */
export function planStorage(n: number, points: number, velocities: boolean, capBytes: number): { results: boolean; trajectories: boolean; bytes: number; dropped: RunRecord["stored"]["dropped"] } {
  const results = resultsBytes(n, velocities);
  const paths = trajectoryBytes(n, points);
  if (results + paths <= capBytes) return { results: true, trajectories: true, bytes: results + paths, dropped: null };
  if (results <= capBytes) return { results: true, trajectories: false, bytes: results, dropped: "trajectories" };
  return { results: false, trajectories: false, bytes: 0, dropped: "results" };
}

/**
 * Runs to delete so at most `keep` remain: oldest unpinned first; pinned runs (and `protect`, the
 * runs the project file points to) are never deleted, even when they alone exceed `keep`.
 */
export function pruneRuns(runs: readonly Pick<RunRecord, "id" | "pinned" | "createdAt">[], keep: number, protect: ReadonlySet<string> = new Set()): string[] {
  const excess = runs.length - Math.max(0, keep);
  if (excess <= 0) return [];
  return [...runs]
    .filter((r) => !r.pinned && !protect.has(r.id))
    .sort((a, b) => a.createdAt.localeCompare(b.createdAt))
    .slice(0, excess)
    .map((r) => r.id);
}

// ------------------------------------------------------------------------------- compare

/** Leaf values of an object by dotted path ("inputs.voltages.2"). */
export function flatten(v: unknown, prefix = "", out: Record<string, string> = {}): Record<string, string> {
  if (v !== null && typeof v === "object" && !Array.isArray(v)) {
    for (const [k, x] of Object.entries(v as Record<string, unknown>)) if (x !== undefined) flatten(x, prefix ? `${prefix}.${k}` : k, out);
  } else {
    out[prefix] = Array.isArray(v) ? v.map((x) => (typeof x === "object" ? JSON.stringify(x) : String(x))).join(", ") : v === null ? "–" : String(v);
  }
  return out;
}

export interface SettingsRow {
  key: string;
  a: string;
  b: string;
  same: boolean;
}

/** Every setting of two runs side by side, differing rows first (then by key). */
export function settingsDiff(a: RunSettings, b: RunSettings): SettingsRow[] {
  const fa = flatten(a);
  const fb = flatten(b);
  const keys = [...new Set([...Object.keys(fa), ...Object.keys(fb)])];
  return keys
    .map((key) => ({ key, a: fa[key] ?? "–", b: fb[key] ?? "–", same: fa[key] === fb[key] }))
    .sort((x, y) => Number(x.same) - Number(y.same) || x.key.localeCompare(y.key));
}

/** Run ids: sortable by time, safe as a folder name. */
export function newRunId(now = new Date(), random = Math.random): string {
  const p = (v: number, n = 2) => String(v).padStart(n, "0");
  const stamp = `${now.getUTCFullYear()}${p(now.getUTCMonth() + 1)}${p(now.getUTCDate())}-${p(now.getUTCHours())}${p(now.getUTCMinutes())}${p(now.getUTCSeconds())}${p(now.getUTCMilliseconds(), 3)}`;
  return `${stamp}-${Math.floor(random() * 0x10000).toString(16).padStart(4, "0")}`;
}

export const RUN_ID = /^[0-9]{8}-[0-9]{9}-[0-9a-f]{4}$/;
