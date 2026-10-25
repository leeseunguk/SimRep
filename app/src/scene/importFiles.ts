import { formatBytes, type Message } from "../i18n";
import { refusals } from "../refusals";
import { newId, useApp, type MeshObject, type PaObject } from "../state/store";
import { pathInfo, readFile, type PathInfo } from "./backend";
import { detectFormat, type FileFormat } from "./formats";
import { importPa, importPaPath } from "./importPa";
import { importIob, openWizard } from "./importWizard";
import StlWorker from "./stl.worker?worker";
import type { StlWorkerResponse } from "./stl.worker";

type Failure = Extract<StlWorkerResponse, { ok: false }>;

let worker: Worker | null = null;
let nextRequest = 1;
const inflight = new Map<number, (r: StlWorkerResponse) => void>();

function stlWorker(): Worker {
  if (worker) return worker;
  worker = new StlWorker();
  worker.onmessage = (e: MessageEvent<StlWorkerResponse>) => {
    inflight.get(e.data.id)?.(e.data);
    inflight.delete(e.data.id);
  };
  worker.onerror = (e) => {
    // A crashed worker fails every pending request; the next import starts a fresh one.
    for (const [id, resolve] of inflight) resolve({ id, ok: false, code: "unexpected", params: {}, message: e.message });
    inflight.clear();
    worker?.terminate();
    worker = null;
  };
  return worker;
}

function parseStlInWorker(buffer: ArrayBuffer): Promise<StlWorkerResponse> {
  return new Promise((resolve) => {
    const id = nextRequest++;
    inflight.set(id, resolve);
    stlWorker().postMessage({ id, buffer }, [buffer]);
  });
}

function failureMessage(f: Failure): Message {
  switch (f.code) {
    case "empty":
    case "noFacets":
      return { key: `stl.error.${f.code}` };
    case "truncated":
    case "badNumber":
      return { key: `stl.error.${f.code}`, params: f.params };
    default:
      return { key: "stl.error.unexpected", params: { message: f.message } };
  }
}

/** Browser preview entry point (HTML5 drop and the file input): File objects, no paths. */
export async function importFiles(files: Iterable<File>) {
  for (const file of files) await importOne({ name: file.name, size: file.size, file });
}

/**
 * Desktop entry point (Tauri drag-drop event, the open dialog, project loading): native paths.
 * Resolves to the created object id per existing path (null where nothing was added).
 */
export async function importPaths(paths: string[], options: { toasts?: boolean } = {}): Promise<(string | null)[]> {
  let entries: PathInfo[];
  try {
    entries = await pathInfo(paths);
  } catch (err) {
    useApp.getState().log("error", { key: "log.backendFailed", params: { reason: String(err) } });
    return [];
  }
  const ids: (string | null)[] = [];
  for (const entry of entries) ids.push(await importOne({ name: entry.name, size: entry.size, entry, toasts: options.toasts ?? true }));
  return ids;
}

/** The array a dropped particle file belongs to: the selected one, else the newest. */
function targetArray(): PaObject | null {
  const { objects, selectedId } = useApp.getState();
  const arrays = objects.filter((o): o is PaObject => o.kind === "pa");
  return arrays.find((o) => o.id === selectedId) ?? arrays.at(-1) ?? null;
}

function attachParticles(name: string, entry: PathInfo | undefined, format: FileFormat) {
  const app = useApp.getState();
  if (!entry) {
    app.toast("info", { key: "toast.particlesDesktop.title", params: { file: name } }, { key: "toast.particlesDesktop.body" });
    return;
  }
  const target = targetArray();
  if (!target) {
    app.toast("warning", { key: "toast.particlesNoArray.title", params: { file: name } }, { key: "toast.particlesNoArray.body" });
    return;
  }
  app.updatePa(target.id, (o) => ({ source: { ...o.source, mode: "file", file: { path: entry.path, name } } }));
  app.select(target.id);
  app.log("info", { key: "log.particlesAttached", params: { file: name, format: { key: format.labelKey }, array: target.fileName } });
  app.toast("success", { key: "toast.particlesAttached.title", params: { file: name } }, { key: "toast.particlesAttached.body", params: { array: target.fileName } });
}

async function importOne({ name, size, file, entry, toasts = true }: { name: string; size: number; file?: File; entry?: PathInfo; toasts?: boolean }): Promise<string | null> {
  const app = useApp.getState();
  const format = detectFormat(name);

  if (!format) {
    app.log("warning", { key: "log.unsupported", params: { file: name } });
    const refusal = refusals.unsupportedFile(name);
    app.toast("warning", refusal.what, refusal.why);
    return null;
  }

  // Desktop: geometry goes through the import wizard, workbenches open their arrays.
  if (entry && format.wizard) {
    await openWizard(entry);
    return null;
  }
  if (format.id === "iob") {
    if (entry) await importIob(entry);
    else app.toast("info", { key: "toast.desktopOnly.title", params: { file: name } }, { key: "toast.desktopOnly.body" });
    return null;
  }

  if (!format.importable || (format.wizard && format.id !== "stl")) {
    const id = newId("file");
    app.addObject({ id, kind: "pending", fileName: name, fileSize: size, formatId: format.id, visible: true });
    app.select(id);
    const formatLabel: Message = { key: format.labelKey };
    app.log("info", { key: "log.importPending", params: { file: name, format: formatLabel } });
    app.toast(
      "info",
      { key: "toast.pending.title", params: { file: name } },
      { key: "toast.pending.body", params: { format: formatLabel } },
    );
    return id;
  }

  if (format.id === "ion" || format.id === "fly2") {
    attachParticles(name, entry, format);
    return null;
  }

  if (format.id === "pa" || format.id === "paHash" || format.id === "pa0") {
    if (entry) return importPaPath(entry, format, toasts);
    if (file) return importPa(file, format);
    return null;
  }

  const id = newId("mesh");
  app.addObject({ id, kind: "loading", fileName: name, fileSize: size, visible: true });
  app.log("info", { key: "log.importStarted", params: { file: name, size: formatBytes(app.lang, size) } });

  let result: StlWorkerResponse;
  try {
    result = await parseStlInWorker(entry ? await readFile(entry.path) : await file!.arrayBuffer());
  } catch (err) {
    result = { id: 0, ok: false, code: "unexpected", params: {}, message: err instanceof Error ? err.message : String(err) };
  }

  const now = useApp.getState();
  if (!now.objects.some((o) => o.id === id)) return null; // removed while reading

  if (!result.ok) {
    now.removeObject(id);
    const reason = failureMessage(result);
    now.log("error", { key: "log.importFailed", params: { file: name, reason } });
    now.toast("error", { key: "toast.failed.title", params: { file: name } }, reason);
    return null;
  }

  const mesh: MeshObject = {
    id,
    kind: "mesh",
    fileName: name,
    path: entry?.path ?? null,
    fileSize: size,
    stlFormat: result.format,
    units: "mm",
    mesh: { vertices: result.vertices, triangleCount: result.triangleCount, bounds: result.bounds },
    meshVersion: 1,
    visible: true,
  };
  now.replaceObject(id, mesh);
  now.select(id);
  now.requestView({ type: "fit" });
  const formatLabel: Message = { key: result.format === "binary" ? "format.stlBinary" : "format.stlAscii" };
  now.log("success", {
    key: "log.importDone",
    params: { file: name, count: result.triangleCount, format: formatLabel, ms: result.ms },
  });
  if (toasts)
    now.toast(
      "success",
      { key: "toast.imported.title", params: { file: name } },
      { key: "toast.imported.body", params: { count: result.triangleCount } },
    );
  return id;
}
