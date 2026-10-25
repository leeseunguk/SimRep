import { formatBytes, type Message } from "../i18n";
import { newId, useApp, type PaObject } from "../state/store";
import { openPa, type PathInfo } from "./backend";
import { computeLabels } from "./electrodes";
import { defaultSource } from "./flight";
import type { FileFormat } from "./formats";
import {
  defaultSlice,
  paKind,
  parsePacket,
  PA_MAX_POINTS,
  PaReadError,
  peekPointCount,
  type PaGrid,
  type PaInfo,
  type PaFormatId,
} from "./pa";
import type { PaSurface } from "./paSurface";
import PaWorker from "./pa.worker?worker";
import type { PaWorkerRequest, PaWorkerResponse } from "./pa.worker";

/** Quad budget for the electrode surface (52 B each); larger arrays are drawn coarser. */
const MAX_QUADS = 3_000_000;

let worker: Worker | null = null;
let nextRequest = 1;
const inflight = new Map<number, (r: PaWorkerResponse) => void>();

function paWorker(): Worker {
  if (worker) return worker;
  worker = new PaWorker();
  worker.onmessage = (e: MessageEvent<PaWorkerResponse>) => {
    inflight.get(e.data.id)?.(e.data);
    inflight.delete(e.data.id);
  };
  worker.onerror = (e) => {
    for (const [id, resolve] of inflight) resolve({ id, ok: false, code: "unexpected", detail: e.message });
    inflight.clear();
    worker?.terminate();
    worker = null;
  };
  return worker;
}

type DistributiveOmit<T, K extends PropertyKey> = T extends unknown ? Omit<T, K> : never;

function request(req: DistributiveOmit<PaWorkerRequest, "id">, transfer: Transferable[] = []): Promise<PaWorkerResponse> {
  return new Promise((resolve) => {
    const id = nextRequest++;
    inflight.set(id, resolve);
    paWorker().postMessage({ ...req, id }, transfer);
  });
}

/** Browser preview: the TypeScript decoder in the worker builds the same packet as the native core. */
async function decodeInBrowser(bytes: ArrayBuffer): Promise<ArrayBuffer> {
  const r = await request({ type: "decode", buffer: bytes }, [bytes]);
  if (!r.ok) throw new PaReadError(r.code, r.detail);
  if (r.type !== "decode") throw new PaReadError("unexpected", "worker answered the wrong request");
  return r.packet;
}

/** Electrode surface for a grid, built in the worker (the worker gets copies). */
export async function buildPaSurface(info: PaInfo, grid: PaGrid, labels: Uint16Array<ArrayBuffer> | null): Promise<PaSurface> {
  const r = await request({
    type: "surface",
    info,
    potentials: grid.potentials.slice(),
    electrodes: grid.electrodes.slice(),
    labels: labels?.slice() ?? null,
    maxQuads: MAX_QUADS,
  });
  if (!r.ok) throw new PaReadError(r.code, r.detail);
  if (r.type !== "surface") throw new PaReadError("unexpected", "worker answered the wrong request");
  return { quads: r.quads, count: r.count, step: r.step };
}

export function formatOfName(name: string): PaFormatId {
  const n = name.toLowerCase();
  return n.endsWith(".pa#") ? "paHash" : n.endsWith(".pa0") ? "pa0" : "pa";
}

/** Voltages of a fast-adjust status, by electrode number. */
export function voltagesOf(info: PaInfo): Record<number, number> {
  const out: Record<number, number> = {};
  if (info.fast?.state === "ready") for (const e of info.fast.electrodes) out[e.number] = e.voltage;
  return out;
}

function failureMessage(e: PaReadError, format: Message, sizes: { actual: string; expected: string }): Message {
  switch (e.code) {
    case "tooLarge":
      return { key: "pa.error.tooLarge", params: { points: Number(e.detail) || 0, limit: PA_MAX_POINTS } };
    case "notPa":
      return { key: "pa.error.notPa", params: { format } };
    case "empty":
      return { key: "pa.error.empty" };
    case "truncated":
      return { key: "pa.error.truncated", params: sizes };
    case "notFound":
      return { key: "pa.error.notFound" };
    case "badHeader":
    case "badPa0":
    case "unexpected":
      return { key: `pa.error.${e.code}` };
  }
}

/** Header + grid size implied by a PA header (the .pa0 trailer comes on top). */
function expectedBytes(head: ArrayBuffer | null): number | null {
  if (!head) return null;
  const points = peekPointCount(head) ?? 0;
  return (new DataView(head).getInt32(0, true) === -2 ? 56 : 32) + points * 8;
}

interface Source {
  name: string;
  size: number;
  path: string | null;
  /** Resolves to an SPA1 packet. `head` receives the first 28 bytes when they are available. */
  packet(id: string, head: { bytes: ArrayBuffer | null }): Promise<ArrayBuffer>;
}

/** Browser preview / HTML5 drop: the file's bytes. */
export function importPa(file: File, format: FileFormat) {
  return importFrom(format, {
    name: file.name,
    size: file.size,
    path: null,
    async packet(_id, head) {
      // Check the header before reading a huge file into memory.
      head.bytes = await file.slice(0, 28).arrayBuffer();
      const points = peekPointCount(head.bytes);
      if (points !== null && points > PA_MAX_POINTS) throw new PaReadError("tooLarge", String(points));
      return decodeInBrowser(await file.arrayBuffer());
    },
  });
}

/** Desktop: the native core reads the file by path (and keeps it for refine / flying). */
export function importPaPath(entry: PathInfo, format: FileFormat, toasts = true) {
  return importFrom(format, { name: entry.name, size: entry.size, path: entry.path, packet: (id) => openPa(id, entry.path) }, toasts);
}

/** Resolves to the new array's id, or null when it could not be opened. */
async function importFrom(format: FileFormat, src: Source, toasts = true): Promise<string | null> {
  const app = useApp.getState();
  const formatLabel: Message = { key: format.labelKey };
  const id = newId("pa");
  app.addObject({ id, kind: "loading", fileName: src.name, fileSize: src.size, visible: true });
  app.log("info", { key: "log.importStarted", params: { file: src.name, size: formatBytes(app.lang, src.size) } });
  const start = performance.now();

  let object: PaObject;
  const head: { bytes: ArrayBuffer | null } = { bytes: null };
  try {
    const { info, grid } = parsePacket(await src.packet(id, head));
    const formatId = format.id as PaFormatId;
    const electrodeSet = await computeLabels({ id, info, grid, formatId, fast: info.fast ?? null });
    const surface = await buildPaSurface(info, grid, electrodeSet.labels);
    object = {
      id,
      kind: "pa",
      fileName: src.name,
      fileSize: src.size,
      formatId,
      info,
      grid,
      surface,
      showMirrors: true,
      // A .pa# has no solved field yet, so equipotential lines would only trace electrode edges.
      slice: { ...defaultSlice(info), contours: format.id !== "paHash" },
      visible: true,
      path: src.path,
      fast: info.fast ?? null,
      voltages: voltagesOf(info),
      electrodeSet,
      electrodeNames: {},
      hiddenElectrodes: [],
      template: null,
      source: defaultSource(info),
      liveFly: true,
      showTrajectories: true,
      outputDir: null,
      refine: null,
      flight: { status: "idle", result: null, error: null },
    };
  } catch (err) {
    const e = err instanceof PaReadError ? err : new PaReadError("unexpected", err instanceof Error ? err.message : String(err));
    const now = useApp.getState();
    now.removeObject(id);
    const expected = expectedBytes(head.bytes);
    const reason = failureMessage(e, formatLabel, {
      actual: formatBytes(now.lang, src.size),
      expected: formatBytes(now.lang, expected ?? src.size),
    });
    now.log("error", { key: "log.importFailed", params: { file: src.name, reason } });
    now.log("info", { key: "log.paErrorDetail", params: { detail: `${e.code}: ${e.detail}` } });
    now.toast("error", { key: "toast.failed.title", params: { file: src.name } }, reason);
    return null;
  }

  const now = useApp.getState();
  if (!now.objects.some((o) => o.id === id)) return null; // removed while reading

  now.replaceObject(id, object);
  now.select(id);
  now.requestView({ type: "fit" });
  const { info } = object;
  const kind: Message = { key: `pa.kind.${paKind(info)}` };
  const size = `${info.nx} × ${info.ny}${info.nz > 1 ? ` × ${info.nz}` : ""}`;
  now.log("success", {
    key: "log.paDone",
    params: { file: src.name, kind, size, points: info.points, quads: object.surface.count, ms: Math.round(performance.now() - start) },
  });
  if (object.surface.step > 1) now.log("warning", { key: "log.paCoarse", params: { file: src.name, step: object.surface.step } });
  if (info.fast?.state === "missing") now.log("warning", { key: "log.fastMissing", params: { file: src.name, detail: info.fast.detail } });
  if (toasts) now.toast("success", { key: "toast.imported.title", params: { file: src.name } }, { key: "toast.paImported.body", params: { kind, size } });
  return id;
}
