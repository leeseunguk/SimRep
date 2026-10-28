import { isTauri } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { ask, open, save } from "@tauri-apps/plugin-dialog";
import { translate, type Message } from "../i18n";
import type { MessageKey } from "../i18n/en";
import { backendError, createTemplate, pathInfo, readText, recoveryPath, removeFile, writeText } from "../scene/backend";
import type { FlightHeader, IonSource } from "../scene/flight";
import { importPaths } from "../scene/importFiles";
import type { SliceState } from "../scene/pa";
import { flyNow, startRefine, waitForRefine } from "../scene/simulation";
import { parseProject, PROJECT_FORMAT, PROJECT_VERSION } from "./projectFormat";
import { openRuns, resetRuns, restoreRun, saveRuns, shownRun, useRuns, type RunsConfig } from "./runs";
import { templateById, type TemplateId } from "../scene/templates";
import type { CameraState } from "../viewport/camera";
import { cameraState } from "../viewport/viewState";
import { markSaved, markUnsaved, quietly, quietlyAsync, resetHistory } from "./history";
import { UNTITLED, useApp, type ProjectMeta, type Projection, type RecentProject, type Units } from "./store";

/**
 * Projects: one `.simrep` file (JSON) that references the arrays and meshes it uses by path
 * (relative to the project file, absolute as a fallback; arrays are never copied into it) and
 * stores everything the user set: electrode names / voltages / visibility, ion sources, section
 * planes, the camera, and a summary of the last flight.
 *
 * Format 2 adds the data folder `X.simrep.d/` (state/runs.ts, docs/project-format.md): every
 * completed flight is a run in `runs/<id>/`, and each array names the run it showed. On open that
 * run is shown again when its inputs still match (same PA sha256 and settings hash); otherwise the
 * ions are flown again, as format 1 projects always are.
 *
 * The open project is autosaved to a recovery file a moment after each change and the start screen
 * offers it after a crash. Saved projects are listed under "recent projects".
 */

export const PROJECT_EXTENSION = "simrep";
const FORMAT = PROJECT_FORMAT;
const VERSION = PROJECT_VERSION;
const RECENT_KEY = "simrep.recent";
const RECENT_MAX = 8;
const AUTOSAVE_MS = 1500;

interface FileRef {
  /** Relative to the project file ('/' separated) when possible, else absolute. */
  path: string;
  absolute: string;
}

interface ArrayEntry {
  file: FileRef;
  name: string | null;
  visible: boolean;
  template: TemplateId | null;
  voltages: Record<number, number>;
  electrodeNames: Record<number, string>;
  hiddenElectrodes: number[];
  source: Omit<IonSource, "file"> & { file: (FileRef & { name: string }) | null };
  liveFly: boolean;
  showTrajectories: boolean;
  showMirrors: boolean;
  slice: SliceState;
  outputDir: string | null;
  /** Summary of the last flight. */
  results: Pick<FlightHeader, "ions" | "surfaces" | "tof" | "keMean" | "flyMs"> | null;
  /** Format 2: the stored run (X.simrep.d/runs/<id>) the array showed, or null. */
  run: string | null;
}

interface MeshEntry {
  file: FileRef;
  name: string | null;
  visible: boolean;
  units: Units;
}

export interface ProjectFile {
  format: typeof FORMAT;
  version: typeof VERSION;
  savedAt: string;
  arrays: ArrayEntry[];
  meshes: MeshEntry[];
  view: { camera: CameraState | null; projection: Projection; grid: boolean };
  /** Format 2: run history settings. */
  runs: RunsConfig;
  /** Recovery file only: the project it belongs to. */
  recovery?: { meta: ProjectMeta };
}

const t = (key: MessageKey, params?: Message["params"]) => translate(useApp.getState().lang, key, params);

// ------------------------------------------------------------------------------------ paths

const isWindowsPath = (p: string) => /^[a-z]:[\\/]|^\\\\/i.test(p);
const parts = (p: string) => p.split(/[\\/]+/).filter((s, i) => s !== "" || i === 0);

export function dirname(p: string): string {
  const i = Math.max(p.lastIndexOf("/"), p.lastIndexOf("\\"));
  return i < 0 ? "" : p.slice(0, i);
}

export function basename(p: string): string {
  return p.slice(Math.max(p.lastIndexOf("/"), p.lastIndexOf("\\")) + 1);
}

const stem = (p: string) => basename(p).replace(/\.[^.]+$/, "");

function joinPath(dir: string, rel: string): string {
  const sep = isWindowsPath(dir) || dir.includes("\\") ? "\\" : "/";
  const out = parts(dir);
  for (const s of rel.split(/[\\/]+/)) {
    if (s === "" || s === ".") continue;
    if (s === "..") out.pop();
    else out.push(s);
  }
  return out.join(sep);
}

/** `to` relative to folder `from` ('/' separated), or null across drives / roots. */
function relativePath(from: string, to: string): string | null {
  const win = isWindowsPath(from) || isWindowsPath(to);
  const a = parts(from);
  const b = parts(to);
  const eq = (x: string, y: string) => (win ? x.toLowerCase() === y.toLowerCase() : x === y);
  if (!a.length || !b.length || !eq(a[0]!, b[0]!)) return null;
  let i = 0;
  while (i < a.length && i < b.length && eq(a[i]!, b[i]!)) i++;
  return [...Array(a.length - i).fill(".."), ...b.slice(i)].join("/");
}

const fileRef = (path: string, base: string | null): FileRef => ({ path: (base && relativePath(base, path)) ?? path, absolute: path });

/** The file a reference points to now: next to the project first, then the recorded absolute path. */
async function locate(ref: FileRef, base: string | null): Promise<string | null> {
  const candidates = [...(base && !isWindowsPath(ref.path) && !ref.path.startsWith("/") ? [joinPath(base, ref.path)] : []), ref.absolute];
  for (const c of candidates) if ((await pathInfo([c])).length) return c;
  return null;
}

// -------------------------------------------------------------------------------- serialize

function serialize(base: string | null): ProjectFile {
  const s = useApp.getState();
  const arrays: ArrayEntry[] = [];
  const meshes: MeshEntry[] = [];
  for (const o of s.objects) {
    if (o.kind === "pa" && o.path) {
      const h = o.flight.result?.header ?? null;
      arrays.push({
        file: fileRef(o.path, base),
        name: o.customName ?? null,
        visible: o.visible,
        template: o.template,
        voltages: o.voltages,
        electrodeNames: o.electrodeNames,
        hiddenElectrodes: o.hiddenElectrodes,
        source: { ...o.source, file: o.source.file ? { ...fileRef(o.source.file.path, base), name: o.source.file.name } : null },
        liveFly: o.liveFly,
        showTrajectories: o.showTrajectories,
        showMirrors: o.showMirrors,
        slice: o.slice,
        outputDir: o.outputDir,
        results: h && { ions: h.ions, surfaces: h.surfaces, tof: h.tof, keMean: h.keMean, flyMs: h.flyMs },
        run: shownRun(o),
      });
    } else if (o.kind === "mesh" && o.path) {
      meshes.push({ file: fileRef(o.path, base), name: o.customName ?? null, visible: o.visible, units: o.units });
    }
  }
  return {
    format: FORMAT,
    version: VERSION,
    savedAt: new Date().toISOString(),
    arrays,
    meshes,
    view: { camera: cameraState(), projection: s.projection, grid: s.gridVisible },
    runs: useRuns.getState().config,
  };
}

const parse = (text: string) => parseProject(text) as ProjectFile;

// ----------------------------------------------------------------------------- scene / meta

export function projectTitle(meta: ProjectMeta, lang = useApp.getState().lang): string {
  return meta.name ?? (meta.nameKey ? translate(lang, meta.nameKey) : translate(lang, "project.untitled"));
}

/** Empties the scene (sessions close once no undo step references them). */
function closeScene(meta: ProjectMeta) {
  // The autosave offer is for the start of a session; the new project's own autosave replaces it.
  quietly(() => useApp.setState({ objects: [], selectedId: null, selectedElectrode: null, project: meta, recovery: null }));
  resetHistory();
  resetRuns(meta.path);
}

/** Asks before throwing away unsaved changes. */
export async function confirmDiscard(): Promise<boolean> {
  const s = useApp.getState();
  if (!s.dirty) return true;
  const message = t("project.discard.body", { name: projectTitle(s.project) });
  if (!isTauri()) return window.confirm(message);
  return ask(message, { title: t("project.discard.title"), kind: "warning", okLabel: t("project.discard.ok"), cancelLabel: t("project.discard.cancel") });
}

function setRecent(recent: RecentProject[]) {
  localStorage.setItem(RECENT_KEY, JSON.stringify(recent));
  useApp.setState({ recent });
}

function rememberRecent(path: string) {
  const others = useApp.getState().recent.filter((r) => r.path.toLowerCase() !== path.toLowerCase());
  setRecent([{ path, name: stem(path), savedAt: Date.now() }, ...others].slice(0, RECENT_MAX));
}

export function forgetRecent(path: string) {
  setRecent(useApp.getState().recent.filter((r) => r.path !== path));
}

// --------------------------------------------------------------------------------- commands

/** Ctrl+N: back to the start screen (the scene stays until something new is opened). */
export function showStart() {
  useApp.getState().setHome(true);
}

export async function saveProject(saveAs = false): Promise<boolean> {
  const app = useApp.getState();
  if (!isTauri()) {
    app.toast("info", { key: "project.desktopOnly.title" }, { key: "project.desktopOnly.body" });
    return false;
  }
  if (!app.objects.some((o) => o.kind === "pa" || o.kind === "mesh")) {
    app.toast("info", { key: "project.empty.title" }, { key: "project.empty.body" });
    return false;
  }
  let path = saveAs ? null : app.project.path;
  if (!path) {
    const name = projectTitle(app.project).replace(/[\\/:*?"<>|]/g, "_");
    const folder = app.project.dir ?? (app.project.path ? dirname(app.project.path) : null);
    const picked = await save({
      title: t("project.saveTitle"),
      defaultPath: folder ? joinPath(folder, `${name}.${PROJECT_EXTENSION}`) : `${name}.${PROJECT_EXTENSION}`,
      filters: [{ name: t("project.filter"), extensions: [PROJECT_EXTENSION] }],
    });
    if (!picked) return false;
    path = picked.toLowerCase().endsWith(`.${PROJECT_EXTENSION}`) ? picked : `${picked}.${PROJECT_EXTENSION}`;
  }
  try {
    await writeText(path, JSON.stringify(serialize(dirname(path)), null, 2));
    // Runs flown before the first save (or kept in the old folder on "save as") go into X.simrep.d.
    await saveRuns(path);
  } catch (err) {
    const reason: Message = { key: "project.error.write", params: { detail: backendError(err).detail } };
    app.log("error", { key: "log.projectSaveFailed", params: { file: basename(path), reason } });
    app.toast("error", { key: "toast.projectSaveFailed.title", params: { file: basename(path) } }, reason);
    return false;
  }
  useApp.setState({ project: { name: stem(path), nameKey: null, path, dir: dirname(path) } });
  markSaved();
  rememberRecent(path);
  void clearRecovery();
  app.log("success", { key: "log.projectSaved", params: { file: path } });
  app.toast("success", { key: "toast.projectSaved.title", params: { name: stem(path) } }, { key: "toast.projectSaved.body", params: { dir: dirname(path) } });
  return true;
}

/** Opens a .simrep (asks for one without `path`). */
export async function openProject(path?: string): Promise<void> {
  const app = useApp.getState();
  if (!isTauri()) {
    app.toast("info", { key: "project.desktopOnly.title" }, { key: "project.desktopOnly.body" });
    return;
  }
  if (!path) {
    const picked = await open({ title: t("project.openTitle"), filters: [{ name: t("project.filter"), extensions: [PROJECT_EXTENSION] }] });
    if (typeof picked !== "string") return;
    path = picked;
  }
  if (!(await confirmDiscard())) return;
  let doc: ProjectFile;
  try {
    doc = parse(await readText(path));
  } catch (err) {
    const e = backendError(err);
    const reason: Message = e.code === "notFound" ? { key: "project.error.notFound" } : { key: "project.error.read", params: { detail: e.detail } };
    if (e.code === "notFound") forgetRecent(path);
    app.log("error", { key: "log.projectOpenFailed", params: { file: basename(path), reason } });
    app.toast("error", { key: "toast.projectOpenFailed.title", params: { file: basename(path) } }, reason);
    return;
  }
  await loadDocument(doc, dirname(path), { name: stem(path), nameKey: null, path, dir: dirname(path) }, true);
  rememberRecent(path);
  app.log("success", { key: "log.projectOpened", params: { file: path } });
}

/** `stored`: show the arrays' saved runs when they still match (not for the autosave, whose runs may be gone). */
async function loadDocument(doc: ProjectFile, base: string | null, meta: ProjectMeta, stored = false) {
  const app = useApp.getState();
  app.setBusy({ key: "busy.opening", params: { name: projectTitle(meta) } });
  closeScene(meta);
  app.setHome(false);
  if (meta.path) await openRuns(meta.path, doc.runs);
  else resetRuns(null, doc.runs);
  const missing: string[] = [];
  const fly: { id: string; run: string | null }[] = [];
  try {
    await quietlyAsync(async () => {
      for (const a of doc.arrays) {
        const path = await locate(a.file, base);
        if (!path) {
          missing.push(a.file.path);
          continue;
        }
        const [id] = await importPaths([path], { toasts: false });
        if (!id) continue;
        const sourceFile = a.source.file ? await locate(a.source.file, base) : null;
        if (a.source.file && !sourceFile) missing.push(a.source.file.path);
        useApp.getState().updatePa(id, (o) => ({
          customName: a.name ?? undefined,
          visible: a.visible,
          template: a.template,
          voltages: o.fast?.state === "ready" ? a.voltages : o.voltages,
          electrodeNames: a.electrodeNames,
          hiddenElectrodes: a.hiddenElectrodes,
          // Older projects lack newer source fields (precision, drawn count): keep the defaults.
          source: { ...o.source, ...a.source, file: sourceFile && a.source.file ? { path: sourceFile, name: a.source.file.name } : null, mode: sourceFile || a.source.mode === "beam" ? a.source.mode : "beam" },
          liveFly: a.liveFly,
          showTrajectories: a.showTrajectories,
          showMirrors: a.showMirrors,
          slice: { ...o.slice, ...a.slice },
          outputDir: a.outputDir,
        }));
        if (a.results || a.run) fly.push({ id, run: stored ? a.run : null });
      }
      for (const m of doc.meshes) {
        const path = await locate(m.file, base);
        if (!path) {
          missing.push(m.file.path);
          continue;
        }
        const [id] = await importPaths([path], { toasts: false });
        if (id) useApp.setState((s) => ({ objects: s.objects.map((o) => (o.id === id && o.kind === "mesh" ? { ...o, customName: m.name ?? undefined, visible: m.visible, units: m.units } : o)) }));
      }
    });
    useApp.setState({ projection: doc.view.projection, gridVisible: doc.view.grid, selectedId: null, selectedElectrode: null });
    if (doc.view.camera) app.requestView({ type: "camera", state: doc.view.camera });
  } finally {
    resetHistory();
    app.setBusy(null);
  }
  let loaded = 0;
  let stale = 0;
  for (const { id, run } of fly) {
    const o = useApp.getState().objects.find((x) => x.id === id);
    const outcome = run && o?.kind === "pa" ? await restoreRun(o, run) : "missing";
    if (outcome === "loaded") loaded++;
    else {
      if (outcome === "stale") stale++;
      void flyNow(id);
    }
  }
  if (loaded) app.log("success", { key: "log.runsLoaded", params: { count: loaded } });
  if (stale) {
    app.log("info", { key: "log.runsStale", params: { count: stale } });
    app.toast("info", { key: "toast.runsStale.title" }, { key: "log.runsStale", params: { count: stale } });
  }
  if (missing.length) {
    app.log("warning", { key: "log.projectMissing", params: { files: missing.join(", ") } });
    app.toast("warning", { key: "toast.projectMissing.title", params: { count: missing.length } }, { key: "toast.projectMissing.body", params: { files: missing.join(", ") } });
  } else {
    app.toast("success", { key: "toast.projectOpened.title", params: { name: projectTitle(meta) } });
  }
}

/** New project from a gallery template: generate, refine, set voltages + source, fly. */
export async function newFromTemplate(id: TemplateId): Promise<void> {
  const app = useApp.getState();
  if (!isTauri()) {
    app.toast("info", { key: "project.desktopOnly.title" }, { key: "template.desktopOnly" });
    return;
  }
  if (!(await confirmDiscard())) return;
  const def = templateById(id);
  const title: Message = { key: def.titleKey };
  app.setBusy({ key: "busy.template", params: { name: title } });
  const started = performance.now();
  try {
    closeScene({ ...UNTITLED, nameKey: def.titleKey });
    const created = await createTemplate(id);
    useApp.setState({ project: { ...UNTITLED, nameKey: def.titleKey, dir: created.dir } });
    const arrayId = await quietlyAsync(async () => {
      const [pid] = await importPaths([created.path], { toasts: false });
      if (!pid) throw new Error(t("template.error.open"));
      useApp.getState().updatePa(pid, () => ({ template: id }));
      app.setBusy({ key: "busy.refining", params: { name: title } });
      await startRefine(pid, "auto", false);
      if (!(await waitForRefine(pid))) throw new Error(t("template.error.refine"));
      useApp.getState().updatePa(pid, (o) => ({
        voltages: { ...def.voltages },
        source: { ...o.source, ...def.source, mode: "beam", file: null },
        slice: { ...o.slice, ...def.slice },
      }));
      return pid;
    });
    resetHistory();
    app.setHome(false);
    app.select(arrayId);
    app.requestView({ type: "preset", view: def.view, fit: true });
    await flyNow(arrayId);
    const ms = Math.round(performance.now() - started);
    app.log("success", { key: "log.templateReady", params: { name: title, dir: created.dir, ms } });
    app.toast("success", { key: "toast.templateReady.title", params: { name: title } }, { key: "toast.templateReady.body", params: { ms } });
  } catch (err) {
    const detail = err instanceof Error ? err.message : backendError(err).detail;
    app.log("error", { key: "log.templateFailed", params: { name: title, detail } });
    app.toast("error", { key: "toast.templateFailed.title", params: { name: title } }, { key: "toast.templateFailed.body", params: { detail } });
  } finally {
    app.setBusy(null);
  }
}

/** "Open SIMION files…" from the start screen: a new untitled project with the picked files. */
export async function newFromFiles(paths: string[]) {
  if (!(await confirmDiscard())) return;
  closeScene(UNTITLED);
  useApp.getState().setHome(false);
  await importPaths(paths);
}

/** Dropped / picked paths: a project file opens as the project, anything else joins the scene. */
export async function openPaths(paths: string[]) {
  const project = paths.find((p) => p.toLowerCase().endsWith(`.${PROJECT_EXTENSION}`));
  if (project) return openProject(project);
  useApp.getState().setHome(false);
  await importPaths(paths);
}

// ---------------------------------------------------------------------------------- autosave

let recoveryFile: Promise<string> | null = null;
const recoveryLocation = () => (recoveryFile ??= recoveryPath());

async function writeRecovery() {
  const s = useApp.getState();
  if (!s.dirty || !s.objects.some((o) => o.kind === "pa" || o.kind === "mesh")) return;
  const doc = serialize(s.project.path ? dirname(s.project.path) : null);
  doc.recovery = { meta: s.project };
  try {
    await writeText(await recoveryLocation(), JSON.stringify(doc));
  } catch (err) {
    s.log("warning", { key: "log.autosaveFailed", params: { detail: backendError(err).detail } });
  }
}

async function clearRecovery() {
  try {
    await removeFile(await recoveryLocation());
  } catch (err) {
    useApp.getState().log("warning", { key: "log.autosaveFailed", params: { detail: backendError(err).detail } });
  }
}

/** Reopens the autosaved project (start screen); it stays marked unsaved. */
export async function restoreRecovery() {
  const app = useApp.getState();
  try {
    const doc = parse(await readText(await recoveryLocation()));
    const meta = doc.recovery?.meta ?? UNTITLED;
    await loadDocument(doc, meta.path ? dirname(meta.path) : null, meta);
    markUnsaved();
  } catch (err) {
    app.log("error", { key: "log.autosaveFailed", params: { detail: err instanceof Error ? err.message : backendError(err).detail } });
  }
}

export async function discardRecovery() {
  useApp.setState({ recovery: null });
  await clearRecovery();
}

let started = false;

/** Start-up: recent projects, a leftover autosave, the window title, autosave on change. */
export function initProjects() {
  if (started) return;
  started = true;
  try {
    const recent = JSON.parse(localStorage.getItem(RECENT_KEY) ?? "[]") as RecentProject[];
    if (Array.isArray(recent)) useApp.setState({ recent: recent.filter((r) => typeof r?.path === "string") });
  } catch {
    localStorage.removeItem(RECENT_KEY); // unreadable list: start a new one
  }

  const title = () => {
    const s = useApp.getState();
    const text = `${s.dirty ? "● " : ""}${projectTitle(s.project, s.lang)} — SimRep`;
    document.title = text;
    if (isTauri()) void getCurrentWindow().setTitle(text).catch(() => undefined);
  };
  title();
  useApp.subscribe((s, prev) => {
    if (s.dirty !== prev.dirty || s.project !== prev.project || s.lang !== prev.lang) title();
  });

  if (!isTauri()) return;
  void (async () => {
    try {
      const doc = parse(await readText(await recoveryLocation()));
      const meta = doc.recovery?.meta ?? UNTITLED;
      if (doc.arrays.length || doc.meshes.length) useApp.setState({ recovery: { meta, savedAt: Date.parse(doc.savedAt) } });
    } catch (err) {
      if (backendError(err).code !== "notFound") useApp.getState().log("warning", { key: "log.autosaveFailed", params: { detail: backendError(err).detail } });
    }
  })();

  let timer: ReturnType<typeof setTimeout> | null = null;
  useApp.subscribe((s, prev) => {
    if (s.objects === prev.objects && s.dirty === prev.dirty) return;
    if (timer) clearTimeout(timer);
    timer = setTimeout(() => void writeRecovery(), AUTOSAVE_MS);
  });
}
