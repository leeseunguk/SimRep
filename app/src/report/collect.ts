import { formatNumber, translate, type Message } from "../i18n";
import { refusalLine, refusals } from "../refusals";
import { runOf } from "../results/analysis";
import { activeArray } from "../results/store";
import { deviceLabel, type DeviceInfo } from "../scene/backend";
import { electrodeName, paElectrodes } from "../scene/electrodes";
import { paKind } from "../scene/pa";
import { projectTitle } from "../state/project";
import { flatten, FLIGHT_TOLERANCES } from "../state/runFormat";
import { listRuns, loadRun, type StoredRun } from "../state/runs";
import { useApp, type PaObject } from "../state/store";
import { captureViewport } from "../viewport/viewState";
import type { ArraySummary, ReportModel, RunColumn, SettingsSummary, TranscriptItem } from "./model";
import { summarize, type ResultSummary } from "./summary";
import { base64 } from "./svg";

/**
 * Gathers a report model from the app: the project, the active array (the selected one with a
 * flight, else the first flown, else the first array), its current in-memory flight, the runs
 * picked in the export dialog (state/runs.ts contract), the 3D view snapshot and the warnings the
 * app shows. Without any stored run the report uses the current flight result.
 */

export interface CollectOptions {
  runIds: string[];
  include3d: boolean;
  transcript: TranscriptItem[] | null;
}

export const runLabel = (lang: ReturnType<typeof useApp.getState>["lang"], r: Pick<StoredRun, "label" | "seq">) =>
  r.label || translate(lang, "report.runs.defaultLabel", { n: r.seq });

/** The array the report is about. */
export function reportArray(): PaObject | null {
  const s = useApp.getState();
  return activeArray(s.objects, s.selectedId) ?? (s.objects.find((o): o is PaObject => o.kind === "pa" && o.id === s.selectedId) ?? s.objects.find((o): o is PaObject => o.kind === "pa") ?? null);
}

function arraySummary(o: PaObject): ArraySummary {
  const lang = useApp.getState().lang;
  const t = (key: Parameters<typeof translate>[1], params?: Parameters<typeof translate>[2]) => translate(lang, key, params);
  const num = (v: number) => formatNumber(lang, v);
  const i = o.info;
  return {
    name: o.customName ?? o.fileName,
    fileName: o.fileName,
    path: o.path,
    format: o.formatId,
    kind: paKind(i),
    dims: [i.nx, i.ny, i.nz],
    spacing: [i.dx, i.dy, i.dz],
    sizeMm: [(i.nx - 1) * i.dx, (i.ny - 1) * i.dy, (i.nz - 1) * i.dz],
    mirror: i.mirror,
    antiMirror: i.antiMirror,
    field: i.field,
    points: i.points,
    electrodePoints: i.electrodePoints,
    cuts: i.cuts ?? null,
    electrodes: paElectrodes(o).map((e) => ({ name: electrodeName(o, e, t, num), number: e.number, voltage: e.voltage })),
  };
}

function settingsSummary(o: PaObject): SettingsSummary {
  const lang = useApp.getState().lang;
  const h = o.flight.result?.header ?? null;
  const r = o.refine;
  const name = (d: DeviceInfo) => {
    const label = deviceLabel(d);
    return typeof label === "string" ? label : translate(lang, label.key, label.params);
  };
  return {
    refine: r
      ? {
          device: r.device ? name(r.device) : null,
          tolerance: r.tolerance,
          cycles: r.done?.cycles ?? null,
          residual: r.done?.residual ?? null,
        }
      : null,
    precision: o.source.precision,
    executed: h?.executed ?? null,
    deviceName: h ? name(h.device) : null,
    collision: o.source.collision ?? "allCorners",
    exactSurfaces: o.source.exactSurfaces ?? true,
    relativistic: true,
    durationUs: o.source.durationUs,
    drawn: o.source.drawCount,
    tolerances: { ...FLIGHT_TOLERANCES },
  };
}

/** Warnings the app shows about this array, its flight and the selected runs, plus warning logs. */
function warnings(o: PaObject | null, runs: StoredRun[]): Message[] {
  const app = useApp.getState();
  const out: Message[] = [];
  const h = o?.flight.result?.header;
  if (o && h) {
    if (h.fallback) out.push(refusalLine(refusals.liveOnCpu(h.fallback)));
    if (h.executed.precision === "f32") out.push({ key: "results.liveNote" });
    if (h.surface && h.surface.accuracy !== "source") out.push({ key: "results.surfaces.approximateTitle", params: { file: h.surface.file } });
    // Exact surfaces requested from a sidecar the flight could not use (missing or unreadable).
    if ((o.source.exactSurfaces ?? true) && o.info.cuts && !h.surface) out.push({ key: "report.warn.sidecarMissing", params: { file: o.info.cuts } });
    if (!(o.source.exactSurfaces ?? true) && o.info.cuts) out.push({ key: "report.warn.sidecarUnused", params: { file: o.fileName } });
    if (h.thinned) out.push({ key: "results.thinned" });
  }
  if (o?.fast?.state === "missing") out.push({ key: "volts.missing.body", params: { detail: o.fast.detail } });
  if (app.compute.state === "cpu") out.push(refusalLine(refusals.computeOnCpu(app.compute.reason)));
  for (const r of runs) {
    if (r.stored.dropped)
      out.push({
        key: "report.warn.runDropped",
        params: { label: runLabel(app.lang, r), detail: { key: r.stored.dropped === "trajectories" ? "log.runTrajectoriesDropped" : "log.runResultsDropped", params: { cap: Math.round(r.stored.bytes / 1048576) } } },
      });
  }
  // Everything the log reported as a warning or error (gap checks, sampling, missing files, ...).
  const time = new Intl.DateTimeFormat(app.lang === "ko" ? "ko-KR" : "en-GB", { hour: "2-digit", minute: "2-digit", second: "2-digit" });
  for (const l of app.logs) if (l.level === "warning" || l.level === "error") out.push({ key: "report.warn.log", params: { time: time.format(l.time), message: l.message } });
  // The same text twice (a repeated log line) is listed once.
  const seen = new Set<string>();
  return out.filter((m) => {
    const k = translate(app.lang, m.key, m.params);
    if (seen.has(k)) return false;
    seen.add(k);
    return true;
  });
}

async function snapshot(): Promise<ReportModel["view3d"]> {
  const gpu = useApp.getState().gpu;
  if (gpu.state === "unavailable" || gpu.state === "lost") return { unavailable: { key: "report.view3d.notReady", params: { detail: gpu.reason } } };
  const pending = captureViewport();
  if (!pending) return { unavailable: { key: "report.view3d.notMounted" } };
  try {
    const blob = await pending;
    return { dataUri: `data:image/png;base64,${base64(new Uint8Array(await blob.arrayBuffer()))}` };
  } catch (err) {
    return { unavailable: { key: "report.view3d.unreadable", params: { detail: err instanceof Error ? err.message : String(err) } } };
  }
}

async function runColumn(r: StoredRun): Promise<RunColumn | null> {
  const lang = useApp.getState().lang;
  const loaded = await loadRun(r.id);
  if (!loaded) return null;
  const label = runLabel(lang, r);
  return {
    label,
    createdAt: r.createdAt,
    summary: summarize(r.header, loaded.run, { histogram: loaded.histogramData, detectorMap: loaded.detectorMap }, { kind: "run", label }),
    settings: flatten(loaded.settings),
  };
}

export async function collectReport(opts: CollectOptions): Promise<ReportModel> {
  const app = useApp.getState();
  const o = reportArray();
  const all = await listRuns();
  const picked = opts.runIds.map((id) => all.find((r) => r.id === id)).filter((r): r is StoredRun => !!r);
  const runs = (await Promise.all(picked.map(runColumn))).filter((r): r is RunColumn => r !== null);

  let results: ResultSummary | null = null;
  const f = o?.flight.result;
  if (f) results = summarize(f.header, runOf(f), null, { kind: "current" });
  else if (runs[0]) results = runs[0].summary;
  else if (all[0]) {
    // No flight in memory: the newest stored run stands in.
    const c = await runColumn(all[0]);
    results = c?.summary ?? null;
  }

  return {
    lang: app.lang,
    generatedAt: new Date(),
    simrepVersion: app.runtime.kind === "tauri" && app.runtime.version ? app.runtime.version : translate(app.lang, "report.versionUnknown"),
    projectName: projectTitle(app.project, app.lang),
    projectPath: app.project.path,
    array: o ? arraySummary(o) : null,
    source: o?.source ?? null,
    settings: o ? settingsSummary(o) : null,
    results,
    view3d: opts.include3d ? await snapshot() : null,
    runs,
    warnings: warnings(o, picked),
    transcript: opts.transcript,
  };
}
