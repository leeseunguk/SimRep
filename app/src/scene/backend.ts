import { Channel, invoke } from "@tauri-apps/api/core";
import type { Message, MessageParams } from "../i18n";
import { en, type MessageKey } from "../i18n/en";
import { PaReadError, type PaErrorCode } from "./pa";
import type { Placement } from "./placement";

/**
 * Desktop-only commands of the native core (src-tauri/src/{pa,refine,fly}.rs). Files are opened by
 * path, so arrays are read and kept in Rust; only decoded grids and results cross IPC, as binary
 * packets. Every array the UI opens is a native "session" keyed by the scene object id.
 */

export interface PathInfo {
  path: string;
  name: string;
  size: number;
}

/**
 * A condition the native core reports as a code (`Reason` in pa.rs): the message is
 * `reason.<code>` in src/i18n, `params` fill it. A `detail` param is technical text (English,
 * from the OS or a library) that the message shows as such.
 */
export interface Reason {
  code: string;
  params?: Record<string, string | number>;
}

/** Error shape of every command (`PaError` in pa.rs). */
export interface BackendError {
  code: PaErrorCode | "refine" | "fly" | "noSession" | "iob" | "geometry" | "export" | "cadUnavailable";
  /** Technical text (English) for the activity log. */
  detail: string;
  /** What the UI says, in its language (absent for unexpected / technical errors). */
  reason?: Reason;
}

export function backendError(err: unknown): BackendError {
  const e = err as Partial<BackendError> | null;
  if (e && typeof e === "object" && typeof e.code === "string") return { code: e.code, detail: e.detail ?? "", reason: e.reason };
  return { code: "unexpected", detail: err instanceof Error ? err.message : String(err) };
}

/** The localized message of a native-core reason. */
export function reasonMessage(r: Reason): Message {
  const key = `reason.${r.code}`;
  const params: MessageParams = { detail: "", ...r.params };
  if (key in en) return { key: key as MessageKey, params };
  return { key: "reason.unknown", params: { code: r.code, detail: String(r.params?.detail ?? "") } };
}

/** What went wrong, in the user's language: the coded reason, else the core's technical text. */
export function errorMessage(e: BackendError): Message {
  return e.reason ? reasonMessage(e.reason) : { key: "error.technical", params: { detail: e.detail } };
}

export function pathInfo(paths: string[]): Promise<PathInfo[]> {
  return invoke<PathInfo[]>("path_info", { paths });
}

/** Reads a PA by path into session `id`; resolves to an SPA1 packet. */
export async function openPa(id: string, path: string): Promise<ArrayBuffer> {
  try {
    return await invoke<ArrayBuffer>("open_pa", { id, path });
  } catch (err) {
    const e = backendError(err);
    throw new PaReadError(e.code as PaErrorCode, e.detail);
  }
}

export function readFile(path: string): Promise<ArrayBuffer> {
  return invoke<ArrayBuffer>("read_file", { path });
}

export function closeSession(id: string): void {
  void invoke("close_session", { id }).catch(() => undefined);
}

/** u16 LE electrode label per grid point of a fast-adjust .pa0 (empty for other arrays). */
export function electrodeLabels(id: string): Promise<ArrayBuffer> {
  return invoke<ArrayBuffer>("electrode_labels", { id });
}

/** Writes a template's .pa# into a new folder under Documents/SimRep (src-tauri/src/templates.rs). */
export function createTemplate(kind: string): Promise<PathInfo & { dir: string }> {
  return invoke("create_template", { kind });
}

export function readText(path: string): Promise<string> {
  return invoke<string>("read_text", { path });
}

export function writeText(path: string, contents: string): Promise<void> {
  return invoke("write_text", { path, contents });
}

export function removeFile(path: string): Promise<void> {
  return invoke("remove_file", { path });
}

export function recoveryPath(): Promise<string> {
  return invoke<string>("recovery_path");
}

export interface RefineTargets {
  dir: string;
  files: { name: string; exists: boolean }[];
  /** Solved with the cut-cell operator (a .pa# built by the import wizard). */
  cutcell: boolean;
}

export function refineTargets(id: string, outputDir: string | null): Promise<RefineTargets> {
  return invoke<RefineTargets>("refine_targets", { id, outputDir });
}

export type RefineDevice = "auto" | "gpu" | "cpu";
export interface DeviceInfo {
  kind: "gpu" | "cpu";
  name: string;
  /** CPU devices: thread count (named in the UI's language, see `deviceLabel`). */
  threads?: number;
}

/** A device for display: GPUs by their adapter name, CPUs as a localized "CPU · N threads". */
export function deviceLabel(d: DeviceInfo): Message | string {
  return d.kind === "cpu" && d.threads ? { key: "device.cpu", params: { threads: d.threads } } : d.name;
}

/** JSON messages of a refine job (`Message` in refine.rs). Binary messages are previews / the result. */
export type RefineMessage =
  | { type: "start"; device: DeviceInfo; stages: (number | null)[]; points: number }
  | { type: "progress"; stage: number; cycle: number; residual: number; absolute: number; elapsedMs: number }
  | { type: "stageDone"; stage: number; cycles: number; residual: number }
  | { type: "device"; device: DeviceInfo; reason: Reason }
  | { type: "done"; files: string[]; dir: string; ms: number; cycles: number; residual: number; device: DeviceInfo }
  | { type: "cancelled" }
  | { type: "error"; message: string; reason: Reason | null };

/** `voltages` ([electrode number, V]) go into the .pa0 written for a .pa# (default 0 V). */
export async function refineStart(
  id: string,
  outputDir: string | null,
  device: RefineDevice,
  onMessage: (m: RefineMessage | ArrayBuffer) => void,
  voltages: [number, number][] = [],
): Promise<number> {
  const channel = new Channel<RefineMessage | ArrayBuffer>(onMessage);
  return invoke<number>("refine_start", { request: { id, outputDir, device, voltages }, channel });
}

export function refineCancel(job: number): Promise<void> {
  return invoke("refine_cancel", { job });
}

/** Superposes a .pa0 at `voltages` ([electrode number, V]); resolves to an SPQ1 packet (potentials only, `parsePotentialsPacket`). */
export function fastAdjust(id: string, voltages: [number, number][]): Promise<ArrayBuffer> {
  return invoke<ArrayBuffer>("fast_adjust", { id, voltages });
}

export type BeamShape = "point" | "line" | "disk";

export type FlySource =
  | {
      kind: "beam";
      count: number;
      massAmu: number;
      chargeE: number;
      energyEv: number;
      positionMm: [number, number, number];
      direction: [number, number, number];
      coneDeg: number;
      shape: BeamShape;
      widthMm: number;
    }
  | { kind: "file"; path: string };

/** live: every ion on the GPU (f32) when one is available; precise: every ion on the CPU (f64). */
export type FlyPrecision = "live" | "precise";

/** Electrode collision rule (src-tauri/src/fly.rs `Collision`): SIMION's all-corner cells, or a half-cell box per point. */
export type FlyCollision = "allCorners" | "nearestNode";

export interface FlyRequest {
  id: string;
  voltages: [number, number][];
  source: FlySource;
  bounds: [number, number, number, number, number, number] | null;
  /** Where the array sits on the workbench (null = grid point 0 at the origin). */
  placement: Placement | null;
  durationUs: number;
  relativistic: boolean;
  precision: FlyPrecision;
  collision: FlyCollision;
  /** Ions drawn as trajectories (evenly spread); results always count every ion. */
  draw: number;
  /** Splat on the sibling .cuts.json surfaces when the array has one (CPU only). */
  surfaces: boolean;
}

/** The app's ion-flight device (src-tauri/src/live.rs `ComputeDevice`). */
export interface ComputeDevice {
  kind: "gpu" | "cpu";
  name: string;
  initMs: number;
  reason: Reason | null;
  threads: number | null;
}

/** Resolves once the start-up GPU device creation has finished (or failed). */
export function computeDevice(): Promise<ComputeDevice> {
  return invoke<ComputeDevice>("compute_device");
}

/** Resolves to an STR3 packet (see `parseFlight` in flight.ts). */
export function fly(request: FlyRequest): Promise<ArrayBuffer> {
  return invoke<ArrayBuffer>("fly", { request });
}

/**
 * Final vx, vy, vz (mm/us) per ion of a recent flight of array `id` (its header's `token`), which
 * flight packets leave out. The native core keeps the last few flights per array.
 */
export async function flightVelocities(id: string, token: number): Promise<Float32Array<ArrayBuffer>> {
  return new Float32Array(await invoke<ArrayBuffer>("flight_velocities", { id, token }));
}

/** Re-flies ion `ion` of `request` alone (CPU, f64); resolves to an SIT1 packet (results/trace.ts). */
export function ionTrace(request: FlyRequest, ion: number): Promise<ArrayBuffer> {
  return invoke<ArrayBuffer>("ion_trace", { request, ion });
}

/** One PA instance of a SIMION workbench (src-tauri/src/iob.rs `IobInstance`). */
export interface IobInstance {
  fileName: string;
  /** The PA next to the .iob, when it exists. */
  path: string | null;
  origin: [number, number, number];
  axes: [[number, number, number], [number, number, number], [number, number, number]];
  /** Workbench mm per array grid unit along each array axis. */
  gridUnitMm: [number, number, number];
}

export interface IobInfo {
  path: string;
  fileName: string;
  bounds: [number, number, number, number, number, number];
  instances: IobInstance[];
}

export function openIob(path: string): Promise<IobInfo> {
  return invoke<IobInfo>("open_iob", { path });
}

/** Resolves to an SGP1 packet (scene/geometry.ts). */
export function inspectGeometry(path: string, units: string | null, repair: boolean): Promise<ArrayBuffer> {
  return invoke<ArrayBuffer>("inspect_geometry", { request: { path, units, repair } });
}

export type BuildMethod = "aligned" | "cutcell";
/** Surface CAD bodies are sampled from (src-tauri/src/geometry.rs `CadSurface`). */
export type CadSurface = "tessellated" | "exact";

export interface BuildRequest {
  path: string;
  units: string | null;
  repair: boolean;
  method: BuildMethod;
  parts: { index: number; number: number; voltage: number }[];
  dims: [number, number, number];
  spacing: [number, number, number];
  origin: [number, number, number];
  outputDir: string;
  stem: string;
  sidecar: boolean;
  cadSurface: CadSurface;
}

export interface BuildResult {
  paHash: string;
  files: string[];
  points: number;
  electrodePoints: [number, number][];
  cutNodes: number | null;
  ms: number;
}

export function buildGeometry(request: BuildRequest): Promise<BuildResult> {
  return invoke<BuildResult>("build_geometry", { request });
}

export interface SurfaceOptions {
  angularSegments: number;
  sweepDegrees: number;
  extrusionDepthMm: number;
}

export type ExportWhat =
  | { kind: "geometry"; surface: SurfaceOptions; names: [number, string][] }
  | { kind: "grid"; field: boolean; angularSegments: number; sweepDegrees: number }
  | { kind: "isosurface"; potential: number; surface: SurfaceOptions }
  | { kind: "pa"; format: "pa" | "paHash" | "pa0" };

/** Writes an export of array `id`; resolves to the file names written. */
export function exportArray(request: { id: string; path: string; voltages: [number, number][]; placement: Placement | null; what: ExportWhat }): Promise<string[]> {
  return invoke<string[]>("export_array", { request });
}

/** Full time series of the drawn ions to .vtp / .csv; resolves to the number of trajectories. */
export function exportTrajectories(request: FlyRequest, path: string): Promise<number> {
  return invoke<number>("export_trajectories", { request, path });
}

/** Writes bytes to `path` (exports); the body crosses IPC raw, not as JSON. */
export function writeBytes(path: string, bytes: Uint8Array<ArrayBuffer>): Promise<void> {
  return invoke("write_bytes", bytes, { headers: { path: encodeURIComponent(path) } });
}
