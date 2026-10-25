import { documentDir, join } from "@tauri-apps/api/path";
import type { Message } from "../i18n";
import { quietly } from "../state/history";
import { useApp, type PaObject } from "../state/store";
import { useWizard, wizardDims, type WizardPart, type WizardState } from "../state/wizard";
import { backendError, buildGeometry, errorMessage, inspectGeometry, openIob, type PathInfo } from "./backend";
import { parseGeometryPacket, type GeometryInfo } from "./geometry";
import { importPaths } from "./importFiles";
import type { Vec3 } from "./mesh";
import { IDENTITY, toWorld, type Placement } from "./placement";
import { startRefine, waitForRefine } from "./simulation";

/**
 * Import wizard flow (desktop): inspect a GEM / mesh file natively, let the user assign electrodes
 * and a grid, then build the .pa# (src-tauri/src/geometry.rs), open it and refine it (aligned:
 * staircase multigrid; cut-cell: the kept SDFs). Also: SIMION workbench (.iob) import.
 */

const stemOf = (name: string) => name.replace(/\.[^.]+$/, "");
const dirOf = (path: string) => path.replace(/[\\/][^\\/]*$/, "");

function defaultParts(info: GeometryInfo): WizardPart[] {
  return info.parts.map((p, i) => {
    const v = p.sourceValue;
    const number = v !== null && Number.isInteger(v) && v >= 1 && v <= 1000 ? v : i + 1;
    return { index: p.index, name: p.name, number, voltage: 0 };
  });
}

/** Drops floating-point noise from suggested grid values (-1.2000000000000002 -> -1.2). */
const tidy = (v: number) => Number(v.toPrecision(12));

/** Grid domain of a fresh inspection: the GEM's pa_define box, or the suggested box around a mesh. */
function defaultDomain(info: GeometryInfo): { spacing: number; origin: Vec3; size: Vec3 } {
  const s = info.suggestion;
  if (info.gem) {
    const g = info.gem;
    return { spacing: g.spacing[0], origin: [0, 0, 0], size: [0, 1, 2].map((a) => (g.dims[a]! - 1) * g.spacing[a]!) as Vec3 };
  }
  return { spacing: s.spacing, origin: s.origin.map(tidy) as Vec3, size: [0, 1, 2].map((a) => tidy((s.dims[a]! - 1) * s.spacing)) as Vec3 };
}

/** Folder for the built arrays: next to the source, unless that is a read-only install folder. */
async function defaultOutput(path: string): Promise<string> {
  const dir = dirOf(path);
  if (!/[\\/]program files( \(x86\))?[\\/]/i.test(path)) return dir;
  return join(await documentDir(), "SimRep", `${stemOf(path.split(/[\\/]/).pop() ?? "import")}-import`);
}

async function inspect(path: string, units: string | null, repair: boolean) {
  return parseGeometryPacket(await inspectGeometry(path, units, repair));
}

export async function openWizard(entry: PathInfo) {
  const app = useApp.getState();
  const wizard = useWizard.getState();
  wizard.open({
    path: entry.path,
    fileName: entry.name,
    status: "inspecting",
    error: null,
    cadUnavailable: false,
    info: null,
    meshes: [],
    units: null,
    repair: true,
    parts: [],
    method: entry.name.toLowerCase().endsWith(".gem") ? "aligned" : "cutcell",
    cadSurface: "tessellated",
    spacing: 1,
    origin: [0, 0, 0],
    size: [1, 1, 1],
    outputDir: "",
    stem: stemOf(entry.name),
    sidecar: true,
    device: "auto",
    step: "parts",
    highlight: null,
    build: null,
    objectId: null,
  });
  app.setHome(false);
  app.log("info", { key: "log.wizardOpened", params: { file: entry.name } });
  try {
    const [{ info, meshes }, outputDir] = await Promise.all([inspect(entry.path, null, true), defaultOutput(entry.path)]);
    if (useWizard.getState().wizard?.path !== entry.path) return;
    useWizard.getState().set({ status: "ready", info, meshes, parts: defaultParts(info), outputDir, ...defaultDomain(info) });
    app.requestView({ type: "fit" });
    app.log("success", { key: "log.wizardInspected", params: { file: entry.name, parts: info.parts.length, triangles: info.previewTriangles, ms: Math.round(info.ms) } });
  } catch (err) {
    const e = backendError(err);
    const detail = errorMessage(e);
    useWizard.getState().set({ status: "error", error: detail, cadUnavailable: e.code === "cadUnavailable" });
    app.log("error", { key: "log.wizardFailed", params: { file: entry.name, detail } });
  }
}

/** Units or repair changed: read the file again (the preview and the suggestion follow). */
export async function reinspect(patch: { units?: string | null; repair?: boolean }) {
  const w = useWizard.getState().wizard;
  if (!w) return;
  const units = patch.units !== undefined ? patch.units : w.units;
  const repair = patch.repair ?? w.repair;
  useWizard.getState().set({ units, repair, status: "inspecting", error: null });
  try {
    const { info, meshes } = await inspect(w.path, units, repair);
    const now = useWizard.getState().wizard;
    if (!now || now.path !== w.path) return;
    // Keep the user's assignments; a unit change rescales the suggested grid.
    const parts = info.parts.map((_, i) => now.parts[i] ?? defaultParts(info)[i]!);
    const domain = patch.units !== undefined ? defaultDomain(info) : {};
    useWizard.getState().set({ status: "ready", info, meshes, parts, ...domain });
    if (patch.units !== undefined) useApp.getState().requestView({ type: "fit" });
  } catch (err) {
    const e = backendError(err);
    useWizard.getState().set({ status: "error", error: errorMessage(e), cadUnavailable: e.code === "cadUnavailable" });
  }
}

export function closeWizard() {
  useWizard.getState().close();
}

/** Electrode names of the opened array, by electrode number (parts sharing one are joined). */
function namesOf(parts: WizardPart[]): Record<number, string> {
  const out: Record<number, string[]> = {};
  for (const p of parts) (out[p.number] ??= []).push(p.name);
  return Object.fromEntries(Object.entries(out).map(([n, names]) => [n, names.join(" + ")]));
}

/** Moves a fresh array's default ion source and box onto its workbench placement. */
function placeSource(o: PaObject, placement: Placement, from: string | null): PaObject["source"] {
  const s = o.source;
  const world = (p: Vec3) => toWorld(placement, p).map((v) => Number(v.toPrecision(6))) as Vec3;
  return { ...s, position: world(s.position), bench: { ...s.bench, min: world(s.bench.min), max: world(s.bench.max), placement, from } };
}

/** Build the .pa#, open it, refine it (progress shows in the wizard and the viewport chip). */
export async function runWizard() {
  const w = useWizard.getState().wizard;
  if (!w?.info) return;
  const app = useApp.getState();
  const dims = wizardDims(w);
  const set = useWizard.getState().set;
  set({ status: "building", error: null, step: "refine" });
  try {
    const build = await buildGeometry({
      path: w.path,
      units: w.units,
      repair: w.repair,
      method: w.method,
      parts: w.parts.map((p) => ({ index: p.index, number: p.number, voltage: p.voltage })),
      dims,
      spacing: [w.spacing, w.spacing, w.spacing],
      origin: w.origin,
      outputDir: w.outputDir,
      stem: w.stem,
      sidecar: w.method === "cutcell" && w.sidecar,
      cadSurface: w.cadSurface,
    });
    set({ build });
    app.log("success", { key: "log.wizardBuilt", params: { files: build.files.join(", "), points: build.points, ms: Math.round(build.ms) } });
    const [id] = await importPaths([build.paHash], { toasts: false });
    if (!id) throw new Error("the built array could not be opened");
    const moved = w.origin.some((v) => v !== 0) && !w.info.gem;
    quietly(() =>
      app.updatePa(id, (o) => ({
        electrodeNames: namesOf(w.parts),
        ...(moved && { source: placeSource(o, { ...IDENTITY, origin: w.origin }, null) }),
      })),
    );
    set({ objectId: id, status: "refining" });
    const voltages: [number, number][] = [...new Map(w.parts.map((p) => [p.number, p.voltage])).entries()];
    await startRefine(id, w.device, true, voltages);
    const ok = await waitForRefine(id);
    const after = useApp.getState().objects.find((o) => o.id === id);
    const note = after?.kind === "pa" ? after.refine?.note ?? null : null;
    set(ok ? { status: "done" } : { status: "error", error: note });
    app.select(id);
  } catch (err) {
    const detail: Message = err instanceof Error ? { key: "error.technical", params: { detail: err.message } } : errorMessage(backendError(err));
    set({ status: "error", error: detail });
    app.log("error", { key: "log.wizardFailed", params: { file: w.fileName, detail } });
  }
}

// ------------------------------------------------------------------------- SIMION workbench

/**
 * Opens a SIMION workbench (.iob): every PA instance it references (next to the .iob), placed as
 * in SIMION, each with the workbench bounds as its field-free flight box.
 */
export async function importIob(entry: PathInfo) {
  const app = useApp.getState();
  let iob;
  try {
    iob = await openIob(entry.path);
  } catch (err) {
    const reason: Message = { key: "iob.error", params: { detail: errorMessage(backendError(err)) } };
    app.log("error", { key: "log.importFailed", params: { file: entry.name, reason } });
    app.toast("error", { key: "toast.failed.title", params: { file: entry.name } }, reason);
    return;
  }
  const b = iob.bounds;
  const opened: string[] = [];
  const missing: string[] = [];
  for (const inst of iob.instances) {
    if (!inst.path) {
      missing.push(inst.fileName);
      continue;
    }
    const [id] = await importPaths([inst.path], { toasts: false });
    const o = id ? useApp.getState().objects.find((x) => x.id === id) : null;
    if (!o || o.kind !== "pa") {
      missing.push(inst.fileName);
      continue;
    }
    const scale = inst.gridUnitMm[0] / o.info.dx;
    const placement: Placement = { origin: inst.origin, axes: inst.axes, scale: Number.isFinite(scale) && scale > 0 ? scale : 1 };
    quietly(() =>
      app.updatePa(o.id, (p) => {
        const source = placeSource(p, placement, iob.fileName);
        return { source: { ...source, bench: { ...source.bench, enabled: true, min: [b[0], b[1], b[2]], max: [b[3], b[4], b[5]] } } };
      }),
    );
    opened.push(inst.fileName);
  }
  app.requestView({ type: "fit" });
  const params = { file: iob.fileName, count: opened.length, arrays: opened.join(", ") || "—", bounds: `[${b.slice(0, 3).join(", ")}] – [${b.slice(3).join(", ")}] mm` };
  app.log(opened.length ? "success" : "warning", { key: "log.iobOpened", params });
  if (missing.length) {
    const reason: Message = { key: "iob.missing", params: { files: missing.join(", ") } };
    app.log("warning", reason);
    app.toast("warning", { key: "toast.iobMissing.title", params: { file: iob.fileName } }, reason);
  } else {
    app.toast("success", { key: "toast.iobOpened.title", params: { file: iob.fileName } }, { key: "toast.iobOpened.body", params });
  }
  if (iob.instances.length > 1 && opened.length > 1) app.log("info", { key: "iob.multi", params: { count: opened.length } });
}

export type { WizardState };
