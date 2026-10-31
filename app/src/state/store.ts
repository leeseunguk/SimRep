import { create } from "zustand";
import type { Message } from "../i18n";
import type { MessageKey } from "../i18n/en";
import type { ElectrodeSet } from "../scene/electrodes";
import type { FormatId } from "../scene/formats";
import type { MeshData } from "../scene/mesh";
import type { DeviceInfo } from "../scene/backend";
import type { Flight, IonSource } from "../scene/flight";
import { clampCoord, displayMirrors, type FastStatus, type PaFormatId, type PaGrid, type PaInfo, type SliceState } from "../scene/pa";
import type { PaSurface } from "../scene/paSurface";
import type { TemplateId } from "../scene/templates";
import type { CameraState } from "../viewport/camera";

export type Lang = "ko" | "en";
export type ThemePref = "light" | "dark" | "system";
export type Units = "mm" | "cm" | "m" | "in";
export type Projection = "perspective" | "orthographic";
export type ViewPreset = "front" | "back" | "right" | "left" | "top" | "bottom" | "iso";

export const UNIT_SCALE: Record<Units, number> = { mm: 1, cm: 10, m: 1000, in: 25.4 };

interface BaseObject {
  id: string;
  visible: boolean;
  customName?: string;
}

export interface MeshObject extends BaseObject {
  kind: "mesh";
  fileName: string;
  /** Desktop: the file on disk (saved in projects). */
  path: string | null;
  fileSize: number;
  stlFormat: "binary" | "ascii";
  units: Units;
  mesh: MeshData;
  meshVersion: number;
}

export interface LoadingObject extends BaseObject {
  kind: "loading";
  fileName: string;
  fileSize: number;
}

export interface PendingObject extends BaseObject {
  kind: "pending";
  fileName: string;
  fileSize: number;
  formatId: FormatId;
}

export interface RefineState {
  job: number | null;
  status: "starting" | "running" | "done" | "cancelled" | "error";
  device: DeviceInfo | null;
  /** Electrode slot per solution (.pa#), or [null] for a .pa. */
  stages: (number | null)[];
  stage: number;
  cycle: number;
  residual: number | null;
  /** Relative residual per V-cycle of the current solution (cycle 0 = start). */
  history: number[];
  /** The finished solutions' histories, in order (a .pa# solves one per electrode). */
  past: number[][];
  /** Target relative residual (from the native core's start message). */
  tolerance: number;
  elapsedMs: number;
  /** Device fallback reason or error. */
  note: Message | null;
  done: { files: string[]; dir: string; ms: number; cycles: number; residual: number } | null;
  /** Grid shown before refining, restored if the job is cancelled or fails. */
  original: { info: PaInfo; grid: PaGrid } | null;
}

export interface FlightState {
  status: "idle" | "running" | "done" | "error";
  result: Flight | null;
  error: Message | null;
  /** The shown result was loaded from the project folder (run `runId`) instead of flown. */
  stored?: { runId: string };
}

/** A SIMION potential array (.pa / .pa# / .pa0) decoded by the native core (or the browser preview). */
export interface PaObject extends BaseObject {
  kind: "pa";
  fileName: string;
  fileSize: number;
  formatId: PaFormatId;
  info: PaInfo;
  grid: PaGrid;
  surface: PaSurface;
  showMirrors: boolean;
  slice: SliceState;
  /** Desktop: the file on disk (refine output, fast-adjust siblings). Null in the browser preview. */
  path: string | null;
  /** Live fast adjustment state of a .pa0 (null for other arrays). */
  fast: FastStatus | null;
  /** Electrode voltages set in the UI (fast adjust), by electrode number. */
  voltages: Record<number, number>;
  /** Electrode label of every grid point (see scene/electrodes.ts); null until computed. */
  electrodeSet: ElectrodeSet | null;
  /** User names of electrodes, by label (= electrode number for .pa# / fast-adjust arrays). */
  electrodeNames: Record<number, string>;
  /** Labels of electrodes hidden in the 3D view. */
  hiddenElectrodes: number[];
  /** Template the array was generated from (default electrode names). */
  template: TemplateId | null;
  source: IonSource;
  /** Re-fly automatically when a voltage changes. */
  liveFly: boolean;
  showTrajectories: boolean;
  /** Folder for refine output; null = next to the source file. */
  outputDir: string | null;
  refine: RefineState | null;
  flight: FlightState;
}

export type SceneObject = MeshObject | LoadingObject | PendingObject | PaObject;
export type RenderableObject = MeshObject;

export type LogLevel = "info" | "success" | "warning" | "error";
export interface LogEntry {
  id: number;
  time: number;
  level: LogLevel;
  message: Message;
}

export type ToastTone = "success" | "info" | "warning" | "error";
export interface Toast {
  id: number;
  tone: ToastTone;
  title: Message;
  body?: Message;
}

export type GpuStatus =
  | { state: "pending" }
  | { state: "ready"; adapter: string }
  | { state: "unavailable"; reason: string }
  | { state: "lost"; reason: string };

/** Device the native core flies ions on (created once at start-up, see src-tauri/src/live.rs). */
export type ComputeStatus =
  | { state: "pending" }
  | { state: "gpu"; name: string; initMs: number }
  | { state: "cpu"; name: string; reason: Message };

/** `web`: a host build of this UI with its own backend (host.ts). */
export type Runtime = { kind: "browser" } | { kind: "tauri"; version?: string } | { kind: "web"; version?: string };

export type ViewCommand =
  | { type: "preset"; view: ViewPreset; fit?: boolean }
  | { type: "fit"; ids?: string[] }
  | { type: "camera"; state: CameraState };

/** The open project: a saved .simrep file (path) or an untitled one. */
export interface ProjectMeta {
  /** File stem of the .simrep, or null while untitled. */
  name: string | null;
  /** Untitled projects created from a template are named after it. */
  nameKey: MessageKey | null;
  path: string | null;
  /** Folder new files of this project go to (template output); default save location. */
  dir: string | null;
}

export interface RecentProject {
  path: string;
  name: string;
  /** ms since epoch */
  savedAt: number;
}

/** Autosaved work found at start-up (state/project.ts). */
export interface RecoveryInfo {
  meta: ProjectMeta;
  savedAt: number;
}

/** Undo / redo availability for the UI (the stacks live in state/history.ts). */
export interface HistoryInfo {
  undo: MessageKey | null;
  redo: MessageKey | null;
}

interface AppState {
  lang: Lang;
  themePref: ThemePref;
  objects: SceneObject[];
  selectedId: string | null;
  /** Electrode (label) of the selected array, when one electrode is selected. */
  selectedElectrode: number | null;
  /** Start screen (template gallery, recent projects) over the workspace. */
  home: boolean;
  helpOpen: boolean;
  project: ProjectMeta;
  /** The project differs from its saved (or freshly created) state. */
  dirty: boolean;
  history: HistoryInfo;
  /** Background work that blocks the start screen (creating a template, opening a project). */
  busy: Message | null;
  recent: RecentProject[];
  recovery: RecoveryInfo | null;
  logs: LogEntry[];
  logOpen: boolean;
  toasts: Toast[];
  gpu: GpuStatus;
  compute: ComputeStatus;
  runtime: Runtime;
  projection: Projection;
  gridVisible: boolean;
  /** Imperative camera requests; seq increments so repeated identical commands still fire. */
  viewCommand: { cmd: ViewCommand; seq: number } | null;

  setLang(lang: Lang): void;
  setThemePref(pref: ThemePref): void;
  select(id: string | null, electrode?: number | null): void;
  setHome(home: boolean): void;
  setHelpOpen(open: boolean): void;
  setBusy(busy: Message | null): void;
  addObject(obj: SceneObject): void;
  replaceObject(id: string, obj: SceneObject): void;
  removeObject(id: string): void;
  toggleVisible(id: string): void;
  rename(id: string, name: string): void;
  renameElectrode(id: string, label: number, name: string): void;
  toggleElectrode(id: string, label: number): void;
  setUnits(id: string, units: Units): void;
  setSlice(id: string, patch: Partial<SliceState>): void;
  setShowMirrors(id: string, show: boolean): void;
  updatePa(id: string, fn: (o: PaObject) => Partial<PaObject>): void;
  log(level: LogLevel, message: Message): void;
  clearLogs(): void;
  setLogOpen(open: boolean): void;
  toast(tone: ToastTone, title: Message, body?: Message): void;
  dismissToast(id: number): void;
  setGpu(gpu: GpuStatus): void;
  setCompute(compute: ComputeStatus): void;
  setRuntime(runtime: Runtime): void;
  setProjection(p: Projection): void;
  toggleGrid(): void;
  requestView(cmd: ViewCommand): void;
}

let nextNumericId = 1;
export function newId(prefix: string): string {
  return `${prefix}-${nextNumericId++}`;
}

const STORAGE_LANG = "simrep.lang";
const STORAGE_THEME = "simrep.theme";

function initialLang(): Lang {
  const saved = localStorage.getItem(STORAGE_LANG);
  if (saved === "ko" || saved === "en") return saved;
  return navigator.language.toLowerCase().startsWith("ko") ? "ko" : "en";
}

function initialTheme(): ThemePref {
  const saved = localStorage.getItem(STORAGE_THEME);
  return saved === "light" || saved === "dark" || saved === "system" ? saved : "system";
}

export const UNTITLED: ProjectMeta = { name: null, nameKey: null, path: null, dir: null };

let nextLogId = 1;
let nextToastId = 1;
const MAX_LOGS = 500;
const MAX_TOASTS = 3;

function mapObject<T extends SceneObject>(objects: SceneObject[], id: string, kind: T["kind"], fn: (o: T) => T) {
  return objects.map((o) => (o.id === id && o.kind === kind ? fn(o as T) : o));
}

export const useApp = create<AppState>()((set) => ({
  lang: initialLang(),
  themePref: initialTheme(),
  objects: [],
  selectedId: null,
  selectedElectrode: null,
  home: true,
  helpOpen: false,
  project: UNTITLED,
  dirty: false,
  history: { undo: null, redo: null },
  busy: null,
  recent: [],
  recovery: null,
  logs: [],
  logOpen: false,
  toasts: [],
  gpu: { state: "pending" },
  compute: { state: "pending" },
  runtime: { kind: "browser" },
  projection: "perspective",
  gridVisible: true,
  viewCommand: null,

  setLang: (lang) => {
    localStorage.setItem(STORAGE_LANG, lang);
    set({ lang });
  },
  setThemePref: (themePref) => {
    localStorage.setItem(STORAGE_THEME, themePref);
    set({ themePref });
  },
  select: (selectedId, electrode = null) => set({ selectedId, selectedElectrode: selectedId === null ? null : electrode }),
  setHome: (home) => set({ home }),
  setHelpOpen: (helpOpen) => set({ helpOpen }),
  setBusy: (busy) => set({ busy }),
  addObject: (obj) => set((s) => ({ objects: [...s.objects, obj] })),
  replaceObject: (id, obj) => set((s) => ({ objects: s.objects.map((o) => (o.id === id ? obj : o)) })),
  removeObject: (id) =>
    set((s) => ({
      objects: s.objects.filter((o) => o.id !== id),
      selectedId: s.selectedId === id ? null : s.selectedId,
      selectedElectrode: s.selectedId === id ? null : s.selectedElectrode,
    })),
  toggleVisible: (id) =>
    set((s) => ({ objects: s.objects.map((o) => (o.id === id ? { ...o, visible: !o.visible } : o)) })),
  rename: (id, name) =>
    set((s) => ({
      objects: s.objects.map((o) => (o.id === id ? { ...o, customName: name.trim() || undefined } : o)),
    })),
  renameElectrode: (id, label, name) =>
    set((s) => ({
      objects: mapObject<PaObject>(s.objects, id, "pa", (o) => {
        const electrodeNames = { ...o.electrodeNames };
        if (name.trim()) electrodeNames[label] = name.trim();
        else delete electrodeNames[label];
        return { ...o, electrodeNames };
      }),
    })),
  toggleElectrode: (id, label) =>
    set((s) => ({
      objects: mapObject<PaObject>(s.objects, id, "pa", (o) => ({
        ...o,
        hiddenElectrodes: o.hiddenElectrodes.includes(label)
          ? o.hiddenElectrodes.filter((l) => l !== label)
          : [...o.hiddenElectrodes, label].sort((a, b) => a - b),
      })),
    })),
  setUnits: (id, units) =>
    set((s) => ({ objects: mapObject<MeshObject>(s.objects, id, "mesh", (o) => ({ ...o, units })) })),
  setSlice: (id, patch) =>
    set((s) => ({
      objects: mapObject<PaObject>(s.objects, id, "pa", (o) => {
        const slice = { ...o.slice, ...patch };
        return { ...o, slice: { ...slice, coord: clampCoord(o.info, slice.axis, slice.coord, o.showMirrors) } };
      }),
    })),
  updatePa: (id, fn) =>
    set((s) => ({ objects: mapObject<PaObject>(s.objects, id, "pa", (o) => ({ ...o, ...fn(o) })) })),
  setShowMirrors: (id, showMirrors) =>
    set((s) => ({
      objects: mapObject<PaObject>(s.objects, id, "pa", (o) => ({
        ...o,
        showMirrors,
        slice: { ...o.slice, coord: clampCoord(o.info, o.slice.axis, o.slice.coord, showMirrors) },
      })),
    })),
  log: (level, message) =>
    set((s) => ({
      logs: [...s.logs, { id: nextLogId++, time: Date.now(), level, message }].slice(-MAX_LOGS),
    })),
  clearLogs: () => set({ logs: [] }),
  setLogOpen: (logOpen) => set({ logOpen }),
  toast: (tone, title, body) =>
    set((s) => ({ toasts: [...s.toasts, { id: nextToastId++, tone, title, body }].slice(-MAX_TOASTS) })),
  dismissToast: (id) => set((s) => ({ toasts: s.toasts.filter((t) => t.id !== id) })),
  setGpu: (gpu) => set({ gpu }),
  setCompute: (compute) => set({ compute }),
  setRuntime: (runtime) => set({ runtime }),
  setProjection: (projection) => set({ projection }),
  toggleGrid: () => set((s) => ({ gridVisible: !s.gridVisible })),
  requestView: (cmd) => set((s) => ({ viewCommand: { cmd, seq: (s.viewCommand?.seq ?? 0) + 1 } })),
}));

export function isRenderable(o: SceneObject): o is RenderableObject {
  return o.kind === "mesh";
}

/** Triangles the viewport draws: meshes plus potential-array surfaces (2 per face, per mirrored copy). */
export function sceneTriangles(objects: SceneObject[], visibleOnly: boolean): number {
  let n = 0;
  for (const o of objects) {
    if (visibleOnly && !o.visible) continue;
    if (isRenderable(o)) n += o.mesh.triangleCount;
    else if (o.kind === "pa") {
      const m = displayMirrors(o.info, o.showMirrors);
      n += o.surface.count * 2 * (m.x ? 2 : 1) * (m.y ? 2 : 1) * (m.z ? 2 : 1);
    }
  }
  return n;
}

export function objectScale(o: RenderableObject): number {
  return UNIT_SCALE[o.units];
}
