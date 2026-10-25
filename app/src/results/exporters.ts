import { isTauri } from "@tauri-apps/api/core";
import { save } from "@tauri-apps/plugin-dialog";
import { translate } from "../i18n";
import { backendError, errorMessage, writeBytes } from "../scene/backend";
import type { PaObject } from "../state/store";
import { useApp } from "../state/store";
import { dirname } from "../state/project";
import { surfaceKey, type RunData } from "./analysis";
import { drawFigure, readPlotTheme, type Panel } from "./plot/chart";
import { CanvasPainter, SvgPainter } from "./plot/painters";
import type { Trace } from "./trace";
import { captureViewport } from "../viewport/viewState";

/**
 * Exports from the results panel: CSV (end states, one ion's time series), plots as PNG/SVG and a
 * PNG of the 3D view. Desktop: native save dialog, then the bytes go to Rust raw (write_bytes).
 * Browser preview: a download.
 */

export type ExportKind = "csv" | "png" | "svg";
const MIME: Record<ExportKind, string> = { csv: "text/csv", png: "image/png", svg: "image/svg+xml" };

/** Where exports go by default: the project folder, else next to the array. */
function defaultDir(o: PaObject | null): string | null {
  const app = useApp.getState();
  return app.project.dir ?? (app.project.path ? dirname(app.project.path) : null) ?? (o?.path ? dirname(o.path) : null);
}

export function exportStem(o: PaObject | null): string {
  return (o?.fileName ?? "simrep").replace(/\.[^.]+$/, "");
}

export async function saveFile(name: string, kind: ExportKind, data: Uint8Array<ArrayBuffer> | string, o: PaObject | null): Promise<void> {
  const app = useApp.getState();
  const t = (key: Parameters<typeof translate>[1], params?: Parameters<typeof translate>[2]) => translate(app.lang, key, params);
  const bytes = typeof data === "string" ? new TextEncoder().encode(data) : data;
  if (!isTauri()) {
    const url = URL.createObjectURL(new Blob([bytes], { type: MIME[kind] }));
    const a = document.createElement("a");
    a.href = url;
    a.download = name;
    a.click();
    setTimeout(() => URL.revokeObjectURL(url), 10_000);
    return;
  }
  const dir = defaultDir(o);
  const sep = dir?.includes("\\") ? "\\" : "/";
  const path = await save({
    title: t("results.export.saveTitle"),
    defaultPath: dir ? `${dir}${sep}${name}` : name,
    filters: [{ name: t(`results.export.filter.${kind}`), extensions: [kind] }],
  });
  if (!path) return;
  try {
    await writeBytes(path, bytes);
    app.log("success", { key: "log.exported", params: { path } });
    app.toast("success", { key: "toast.exported.title" }, { key: "toast.exported.body", params: { path } });
  } catch (err) {
    const detail = errorMessage(backendError(err));
    app.log("error", { key: "log.exportFailed", params: { path, detail } });
    app.toast("error", { key: "toast.exportFailed.title" }, { key: "toast.exportFailed.body", params: { detail } });
  }
}

// ---------------------------------------------------------------------------------------- CSV

const f32 = (v: number) => (Number.isFinite(v) ? String(Number(v.toPrecision(7))) : "");

/** End state of every ion (or `indices`), one row per ion. `velocity`: vx, vy, vz per ion (`flightVelocities`). */
export function endStatesCsv(run: RunData, velocity: Float32Array, indices?: ArrayLike<number>): string {
  const { cols, surface, surfaces } = run;
  const keys = surfaces.map((s) => surfaceKey(s.surface));
  const rows = ["ion,surface,x_mm,y_mm,z_mm,r_mm,tof_us,ke_ev,vx_mm_per_us,vy_mm_per_us,vz_mm_per_us"];
  const n = indices ? indices.length : run.n;
  for (let k = 0; k < n; k++) {
    const i = indices ? indices[k]! : k;
    rows.push(
      [
        i,
        keys[surface[i]!] ?? "",
        f32(cols.x[i]!),
        f32(cols.y[i]!),
        f32(cols.z[i]!),
        f32(cols.r[i]!),
        f32(cols.tof[i]!),
        f32(cols.ke[i]!),
        f32(velocity[i * 3]!),
        f32(velocity[i * 3 + 1]!),
        f32(velocity[i * 3 + 2]!),
      ].join(","),
    );
  }
  return rows.join("\r\n") + "\r\n";
}

/** One ion's time series (f64, full precision). */
export function traceCsv(tr: Trace): string {
  const rows = ["t_us,x_mm,y_mm,z_mm,vx_mm_per_us,vy_mm_per_us,vz_mm_per_us,speed_mm_per_us,ke_ev,potential_v"];
  for (let i = 0; i < tr.t.length; i++) {
    rows.push([tr.t[i], tr.x[i], tr.y[i], tr.z[i], tr.vx[i], tr.vy[i], tr.vz[i], tr.speed[i], tr.ke[i], tr.potential[i]].join(","));
  }
  return rows.join("\r\n") + "\r\n";
}

// -------------------------------------------------------------------------------------- plots

/** What the active tab shows, for "export plot" (registered by the tab while mounted). */
export interface Figure {
  name: string;
  width: number;
  height: number;
  panels: Panel[];
}

let figureSource: (() => Figure | null) | null = null;

export function registerFigure(get: () => Figure | null): () => void {
  figureSource = get;
  return () => {
    if (figureSource === get) figureSource = null;
  };
}

export function currentFigure(): Figure | null {
  return figureSource?.() ?? null;
}

/** PNG at `scale`× the on-screen size. */
export async function figurePng(fig: Figure, scale = 2): Promise<Uint8Array<ArrayBuffer>> {
  const canvas = document.createElement("canvas");
  canvas.width = Math.round(fig.width * scale);
  canvas.height = Math.round(fig.height * scale);
  const ctx = canvas.getContext("2d")!;
  ctx.scale(scale, scale);
  const theme = readPlotTheme();
  drawFigure(new CanvasPainter(ctx, theme), fig.panels, theme, fig.width, fig.height);
  return blobBytes(await new Promise<Blob | null>((r) => canvas.toBlob(r, "image/png")));
}

export function figureSvg(fig: Figure): string {
  const theme = readPlotTheme();
  const p = new SvgPainter(theme);
  drawFigure(p, fig.panels, theme, fig.width, fig.height);
  return p.document(fig.width, fig.height);
}

async function blobBytes(blob: Blob | null): Promise<Uint8Array<ArrayBuffer>> {
  if (!blob) throw new Error("the image could not be encoded");
  return new Uint8Array(await blob.arrayBuffer());
}

/** PNG of the 3D view (on its background), or null if the viewport is not ready. */
export async function viewportPng(): Promise<Uint8Array<ArrayBuffer> | null> {
  const blob = await captureViewport();
  return blob ? blobBytes(blob) : null;
}
