import { isTauri } from "@tauri-apps/api/core";
import { save } from "@tauri-apps/plugin-dialog";
import { translate } from "../i18n";
import { backendError, errorMessage, writeBytes } from "../scene/backend";
import type { PaObject } from "../state/store";
import { useApp } from "../state/store";
import { dirname } from "../state/project";
import { drawFigure, readPlotTheme, type Panel } from "./plot/chart";
import { CanvasPainter, SvgPainter } from "./plot/painters";
import { captureViewport } from "../viewport/viewState";

export { endStatesCsv, traceCsv } from "./csv";

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
