import { isTauri } from "@tauri-apps/api/core";
import type { Message } from "../i18n";
import { quietly } from "../state/history";
import { useApp, type PaObject, type RefineState } from "../state/store";
import {
  backendError,
  deviceLabel,
  errorMessage,
  fastAdjust,
  fly,
  refineCancel,
  reasonMessage,
  refineStart,
  type BackendError,
  type FlyRequest,
  type RefineDevice,
  type RefineMessage,
} from "./backend";
import { REFINE_TOLERANCE } from "../components/simulationFormat";
import { computeLabels } from "./electrodes";
import { DEFAULT_DRAWN, directionOf, parseFlight } from "./flight";
import { buildPaSurface, formatOfName, voltagesOf } from "./importPa";
import { parsePacket, parsePotentialsPacket, type PaGrid, type PaInfo } from "./pa";
import { isIdentity } from "./placement";

/**
 * Refine, fast adjust and flying for potential arrays (desktop only: the native core does the
 * work, see src-tauri/src/{refine,fly}.rs). Results land in the scene store; the viewport follows.
 */

const pa = (id: string): PaObject | null => {
  const o = useApp.getState().objects.find((x) => x.id === id);
  return o?.kind === "pa" ? o : null;
};

export const simulationAvailable = () => isTauri();

const sameRecord = (a: Record<number, number>, b: Record<number, number>) => {
  const ka = Object.keys(a);
  return ka.length === Object.keys(b).length && ka.every((k) => a[Number(k)] === b[Number(k)]);
};

// Voltage and ion-source edits follow the store, whoever made them (panel, undo / redo, project
// load): the native core re-superposes the field and, once ions have been flown, re-flies them.
if (isTauri()) {
  useApp.subscribe((s, prev) => {
    if (s.objects === prev.objects) return;
    const before = new Map(prev.objects.map((o) => [o.id, o]));
    for (const o of s.objects) {
      const p = before.get(o.id);
      if (o.kind !== "pa" || p?.kind !== "pa" || o === p) continue;
      // A relabel (refine result) brings its own field; only user voltages need a fast adjust.
      const voltages = o.fast?.state === "ready" && o.fast === p.fast && !sameRecord(o.voltages, p.voltages);
      const ions = o.source !== p.source && o.flight.result !== null;
      if (voltages || ions) scheduleLive(o.id, { voltages, ions });
    }
  });
}

function range(values: Float32Array): [number, number] {
  let min = Infinity;
  let max = -Infinity;
  for (let i = 0; i < values.length; i++) {
    const v = values[i]!;
    if (v < min) min = v;
    if (v > max) max = v;
  }
  return [min, max];
}

// ----------------------------------------------------------------------------- electrode surface

/** Rebuilds the electrode surface (its colours follow the potentials) for the latest grid only. */
const surfaceJobs = new Map<string, { running: boolean; dirty: boolean }>();

function scheduleSurface(id: string) {
  const job = surfaceJobs.get(id) ?? { running: false, dirty: false };
  surfaceJobs.set(id, job);
  job.dirty = true;
  if (job.running) return;
  job.running = true;
  void (async () => {
    try {
      while (job.dirty) {
        job.dirty = false;
        const o = pa(id);
        if (!o) break;
        const grid = o.grid;
        const surface = await buildPaSurface(o.info, grid, o.electrodeSet?.labels ?? null);
        if (pa(id)?.grid === grid) useApp.getState().updatePa(id, () => ({ surface }));
      }
    } catch (err) {
      useApp.getState().log("error", { key: "log.paErrorDetail", params: { detail: String(err) } });
    } finally {
      job.running = false;
      surfaceJobs.delete(id);
    }
  })();
}

/** A parsed packet from the native core for an existing array (fast adjust or refine result). */
function applyPacket(id: string, { info, grid }: { info: PaInfo; grid: PaGrid }, relabel: boolean) {
  // A refine result relabels the array (system change, not an undo step).
  quietly(() => useApp.getState().updatePa(id, (o) => ({
    info,
    grid,
    // `fast` keeps the voltages stored in the file (the baseline for "reset"); live packets carry
    // the adjusted ones.
    ...(relabel && {
      fast: info.fast ?? null,
      fileName: info.fileName ?? o.fileName,
      fileSize: info.fileSize ?? o.fileSize,
      path: info.path ?? o.path,
      formatId: formatOfName(info.fileName ?? o.fileName),
      voltages: voltagesOf(info),
      flight: { status: "idle", result: null, error: null },
    }),
  })));
  if (relabel) void relabelElectrodes(id);
  else scheduleSurface(id);
}

/** New electrode labels after the array changed kind (a refined .pa# is a .pa0 now), then the surface. */
async function relabelElectrodes(id: string) {
  const o = pa(id);
  if (!o) return;
  try {
    const electrodeSet = await computeLabels(o);
    if (pa(id)?.grid === o.grid) useApp.getState().updatePa(id, () => ({ electrodeSet }));
  } catch (err) {
    useApp.getState().log("error", { key: "log.paErrorDetail", params: { detail: errorMessage(backendError(err)) } });
  }
  scheduleSurface(id);
}

// ------------------------------------------------------------------------------------- refine

/** A coded reason states itself; technical text is sorted into write errors and the rest. */
function refineFailure(e: Pick<BackendError, "detail" | "reason">): Message {
  if (e.reason) return reasonMessage(e.reason);
  if (/os error|denied|access/i.test(e.detail)) return { key: "refine.error.write", params: { detail: e.detail } };
  return { key: "refine.error.other", params: { detail: e.detail } };
}

function patchRefine(id: string, fn: (r: RefineState) => Partial<RefineState>) {
  useApp.getState().updatePa(id, (o) => (o.refine ? { refine: { ...o.refine, ...fn(o.refine) } } : {}));
}

function restoreOriginal(id: string) {
  useApp.getState().updatePa(id, (o) => (o.refine?.original ? { info: o.refine.original.info, grid: o.refine.original.grid } : {}));
}

function onRefineMessage(id: string, name: string, notify: boolean, m: RefineMessage | ArrayBuffer) {
  const app = useApp.getState();
  if (m instanceof ArrayBuffer) {
    const magic = String.fromCharCode(...new Uint8Array(m, 0, 4));
    if (magic === "SPV1") {
      // Progressive preview of the solution being refined (same grid, new potentials).
      const potentials = new Float32Array(m, 8);
      const [potentialMin, potentialMax] = range(potentials);
      app.updatePa(id, (o) =>
        o.refine?.status === "running" && potentials.length === o.info.points
          ? { grid: { potentials, electrodes: o.grid.electrodes } satisfies PaGrid, info: { ...o.info, potentialMin, potentialMax } satisfies PaInfo }
          : {},
      );
    } else if (magic === "SPA1") {
      applyPacket(id, parsePacket(m), true);
    }
    return;
  }
  switch (m.type) {
    case "start":
      patchRefine(id, () => ({ status: "running", device: m.device, stages: m.stages, tolerance: m.tolerance }));
      app.log("info", { key: "log.refineStarted", params: { file: name, device: deviceLabel(m.device), count: m.stages.length } });
      break;
    case "progress":
      patchRefine(id, (r) => ({
        stage: m.stage,
        cycle: m.cycle,
        residual: m.residual,
        elapsedMs: m.elapsedMs,
        history: m.stage === r.stage ? [...r.history, m.residual] : [m.residual],
        past: m.stage === r.stage ? r.past : [...r.past, r.history],
      }));
      break;
    case "stageDone":
      break;
    case "device":
      patchRefine(id, () => ({ device: m.device, note: reasonMessage(m.reason) }));
      app.log("warning", { key: "log.refineDevice", params: { device: deviceLabel(m.device), reason: reasonMessage(m.reason) } });
      break;
    case "done": {
      const done = { files: m.files, dir: m.dir, ms: m.ms, cycles: m.cycles, residual: m.residual };
      patchRefine(id, () => ({ status: "done", job: null, device: m.device, done, original: null }));
      const params = { file: name, files: m.files.join(", "), dir: m.dir, ms: Math.round(m.ms), cycles: m.cycles, residual: m.residual.toExponential(1) };
      app.log("success", { key: "log.refineDone", params });
      if (notify) app.toast("success", { key: "toast.refined.title", params: { file: name } }, { key: "toast.refined.body", params });
      break;
    }
    case "cancelled":
      restoreOriginal(id);
      patchRefine(id, () => ({ status: "cancelled", job: null, original: null }));
      app.log("info", { key: "log.refineCancelled", params: { file: name } });
      break;
    case "error": {
      restoreOriginal(id);
      const reason = refineFailure({ detail: m.message, reason: m.reason ?? undefined });
      patchRefine(id, () => ({ status: "error", job: null, note: reason, original: null }));
      app.log("error", { key: "log.refineFailed", params: { file: name, reason } });
      app.toast("error", { key: "toast.refineFailed.title", params: { file: name } }, reason);
      break;
    }
  }
}

/** `notify`: toast on success (template setup reports once at its end instead). */
export async function startRefine(id: string, device: RefineDevice, notify = true, voltages: [number, number][] = []) {
  const o = pa(id);
  if (!o || o.refine?.status === "running" || o.refine?.status === "starting") return;
  const app = useApp.getState();
  const name = o.fileName;
  quietly(() => app.updatePa(id, () => ({
    refine: {
      job: null,
      status: "starting",
      device: null,
      stages: [],
      stage: 0,
      cycle: 0,
      residual: null,
      history: [],
      past: [],
      tolerance: REFINE_TOLERANCE,
      elapsedMs: 0,
      note: null,
      done: null,
      original: { info: o.info, grid: o.grid },
    },
    // Show equipotential lines as the field forms.
    slice: { ...o.slice, contours: true },
  })));
  try {
    const job = await refineStart(id, o.outputDir, device, (m) => onRefineMessage(id, name, notify, m), voltages);
    patchRefine(id, (r) => (r.status === "starting" || r.status === "running" ? { job } : {}));
  } catch (err) {
    const e = backendError(err);
    app.updatePa(id, () => ({ refine: null }));
    const reason = refineFailure(e);
    app.log("error", { key: "log.refineFailed", params: { file: name, reason } });
    app.toast("error", { key: "toast.refineFailed.title", params: { file: name } }, reason);
  }
}

export function cancelRefine(id: string) {
  const job = pa(id)?.refine?.job;
  if (job != null) void refineCancel(job);
}

// ------------------------------------------------------------------------- flying + live mode

function voltageList(o: PaObject): [number, number][] {
  if (o.fast?.state !== "ready") return [];
  return o.fast.electrodes.map((e) => [e.number, o.voltages[e.number] ?? e.voltage]);
}

export function flyRequest(o: PaObject): FlyRequest {
  const s = o.source;
  const bench = s.bench;
  return {
    id: o.id,
    voltages: voltageList(o),
    source:
      s.mode === "file" && s.file
        ? { kind: "file", path: s.file.path }
        : {
            kind: "beam",
            count: Math.round(s.count),
            massAmu: s.massAmu,
            chargeE: s.chargeE,
            energyEv: s.energyEv,
            positionMm: s.position,
            direction: directionOf(s),
            coneDeg: s.coneDeg,
            shape: s.shape,
            widthMm: s.widthMm,
          },
    bounds: bench.enabled ? [...bench.min, ...bench.max] : null,
    placement: isIdentity(bench.placement) ? null : bench.placement!,
    durationUs: s.durationUs,
    relativistic: true,
    precision: s.precision ?? "live",
    collision: s.collision ?? "allCorners",
    draw: Math.round(s.drawCount ?? DEFAULT_DRAWN),
    surfaces: s.exactSurfaces ?? true,
  };
}

/** Why an array cannot be flown yet (null = ready). */
export function flyBlocker(o: PaObject): Message | null {
  if (!simulationAvailable()) return { key: "sim.desktopOnly" };
  if (o.formatId === "paHash") return { key: "fly.blocked.unrefined" };
  if (o.info.field !== "electrostatic") return { key: "fly.blocked.magnetic" };
  if (o.refine?.status === "running" || o.refine?.status === "starting") return { key: "fly.blocked.refining" };
  if (o.source.mode === "file" && !o.source.file) return { key: "fly.blocked.noFile" };
  return null;
}

let flySeq = 0;
/** Newest flight whose result was applied, per array: results may arrive out of order (live re-fly pipelines). */
const appliedFly = new Map<string, number>();

/** False (and records the claim) unless `seq` is newer than the result already shown. */
function claimResult(id: string, seq: number): boolean {
  if ((appliedFly.get(id) ?? 0) > seq) return false;
  appliedFly.set(id, seq);
  return true;
}

export async function flyNow(id: string, live = false) {
  const o = pa(id);
  if (!o || flyBlocker(o)) return;
  const app = useApp.getState();
  const seq = ++flySeq;
  if (!live) app.updatePa(id, (p) => ({ flight: { ...p.flight, status: "running", error: null } }));
  const started = performance.now();
  try {
    const result = parseFlight(await fly(flyRequest(o)), 0);
    result.roundTripMs = performance.now() - started;
    if (!claimResult(id, seq)) return; // a newer flight's result is already shown
    app.updatePa(id, () => ({ flight: { status: "done", result, error: null } }));
    if (!live) {
      const h = result.header;
      app.log("success", {
        key: "log.flyDone",
        params: { file: o.fileName, ions: h.ions, device: deviceLabel(h.device), ms: Math.round(h.flyMs), points: h.points },
      });
    }
  } catch (err) {
    if (!claimResult(id, seq)) return;
    const e = backendError(err);
    const reason: Message = e.reason ? reasonMessage(e.reason) : { key: "fly.error.other", params: { detail: e.detail } };
    app.updatePa(id, (p) => ({ flight: { ...p.flight, status: "error", error: reason } }));
    app.log("error", { key: "log.flyFailed", params: { file: o.fileName, reason } });
    if (!live) app.toast("error", { key: "toast.flyFailed.title", params: { file: o.fileName } }, reason);
  }
}

/**
 * Live mode: voltage / source edits are coalesced per array, latest wins.
 * - The section (fast adjust) runs one request at a time; edits made meanwhile are sent once more
 *   with the latest values when it returns.
 * - Re-flying keeps up to LIVE_DEPTH requests in flight, each with the newest values when it is
 *   sent (edits made meanwhile only mark the job dirty). The native core queues each flight on
 *   the GPU right behind the previous one (submit/collect, src-tauri/src/live.rs), so the GPU
 *   stays busy while a finished flight is read back, its packet crosses the IPC (about 13 ms per
 *   MB) and is drawn.
 *   A result older than the one already shown is dropped (see claimResult).
 */
const LIVE_DEPTH = 3;
const sectionJobs = new Map<string, { running: boolean; dirty: boolean }>();
const flyJobs = new Map<string, { inFlight: number; dirty: boolean }>();

function scheduleLive(id: string, what: { voltages?: boolean; ions?: boolean }) {
  if (what.voltages) scheduleSection(id);
  if (what.voltages || what.ions) scheduleFly(id);
}

function scheduleSection(id: string) {
  const job = sectionJobs.get(id) ?? { running: false, dirty: false };
  sectionJobs.set(id, job);
  job.dirty = true;
  if (job.running) return;
  job.running = true;
  void (async () => {
    try {
      while (job.dirty) {
        job.dirty = false;
        const o = pa(id);
        if (!o) break;
        const packet = await fastAdjust(id, voltageList(o));
        // Live packets carry only the potentials; the electrode flags are the array's own.
        const now = pa(id);
        if (!now) break;
        applyPacket(id, parsePotentialsPacket(packet, now.grid.electrodes), false);
      }
    } catch (err) {
      useApp.getState().log("error", { key: "log.fastAdjustFailed", params: { detail: errorMessage(backendError(err)) } });
    } finally {
      job.running = false;
      sectionJobs.delete(id);
    }
  })();
}

function scheduleFly(id: string) {
  const job = flyJobs.get(id) ?? { inFlight: 0, dirty: false };
  flyJobs.set(id, job);
  job.dirty = true;
  pumpFly(id, job);
}

function pumpFly(id: string, job: { inFlight: number; dirty: boolean }) {
  while (job.dirty && job.inFlight < LIVE_DEPTH) {
    job.dirty = false;
    const o = pa(id);
    if (!o || !o.liveFly || !o.flight.result || flyBlocker(o)) break;
    job.inFlight++;
    void flyNow(id, true).finally(() => {
      job.inFlight--;
      pumpFly(id, job);
      if (job.inFlight === 0 && !job.dirty) flyJobs.delete(id);
    });
  }
  if (job.inFlight === 0 && !job.dirty) flyJobs.delete(id);
}

export function setElectrodeVoltage(id: string, electrode: number, voltage: number) {
  if (!Number.isFinite(voltage)) return;
  useApp.getState().updatePa(id, (o) => ({ voltages: { ...o.voltages, [electrode]: voltage } }));
}

export function resetVoltages(id: string) {
  useApp.getState().updatePa(id, (o) => ({ voltages: voltagesOf({ ...o.info, fast: o.fast }) }));
}

/** Ion source edits re-fly too (live mode, see the subscription above), once a flight exists. */
export function updateSource(id: string, patch: Partial<PaObject["source"]>) {
  useApp.getState().updatePa(id, (o) => ({ source: { ...o.source, ...patch } }));
}

/** After startRefine: resolves when the job ends, true if it converged and was written. */
export function waitForRefine(id: string): Promise<boolean> {
  return new Promise((resolve) => {
    const check = () => {
      const r = pa(id)?.refine;
      if (!r) return false; // removed, or the job never started
      if (r.status === "starting" || r.status === "running") return null;
      return r.status === "done";
    };
    const now = check();
    if (now !== null) return resolve(now);
    const stop = useApp.subscribe(() => {
      const done = check();
      if (done === null) return;
      stop();
      resolve(done);
    });
  });
}
