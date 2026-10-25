/**
 * A small plotting engine for the results panel: one chart model drawn through a `Painter`, so the
 * same code paints the on-screen canvas, a hi-DPI PNG export and an SVG export. Kept in-house (no
 * dependency): the plots needed are few (histograms, step overlays, lines, rasters) and this keeps
 * theming on the CSS tokens and the bundle a few kB.
 */

export interface TextStyle {
  color: string;
  align?: "left" | "center" | "right";
  baseline?: "top" | "middle" | "bottom";
  mono?: boolean;
  bold?: boolean;
  /** Rotate -90° about (x, y) (vertical axis labels). */
  vertical?: boolean;
}

export interface Raster {
  width: number;
  height: number;
  /** RGBA, row 0 = top. */
  data: Uint8ClampedArray<ArrayBuffer>;
}

export interface Painter {
  rect(x: number, y: number, w: number, h: number, fill: string): void;
  /** Polyline through flat [x0, y0, x1, y1, ...]; with `fill`, closed and filled instead of stroked. */
  path(points: ArrayLike<number>, style: { stroke?: string; width?: number; dash?: number[]; fill?: string }): void;
  circle(x: number, y: number, r: number, style: { stroke?: string; width?: number; fill?: string }): void;
  text(s: string, x: number, y: number, style: TextStyle): void;
  measure(s: string, mono?: boolean): number;
  image(r: Raster, x: number, y: number, w: number, h: number): void;
  clip(x: number, y: number, w: number, h: number): void;
  unclip(): void;
}

export interface PlotTheme {
  bg: string;
  grid: string;
  axis: string;
  text: string;
  textStrong: string;
  primary: string;
  primaryFill: string;
  band: string;
  select: string;
  series: string[];
  xyz: [string, string, string];
  heat: string[];
  font: string;
  mono: string;
  fontSize: number;
  tickGap: number;
  lineW: number;
  selectW: number;
  markerR: number;
  point: number;
}

export function readPlotTheme(el: Element = document.documentElement): PlotTheme {
  const css = getComputedStyle(el);
  const v = (name: string) => css.getPropertyValue(name).trim();
  const px = (name: string) => parseFloat(v(name));
  return {
    bg: v("--plot-bg"),
    grid: v("--plot-grid"),
    axis: v("--plot-axis"),
    text: v("--plot-text"),
    textStrong: v("--plot-text-strong"),
    primary: v("--plot-primary"),
    primaryFill: v("--plot-primary-fill"),
    band: v("--plot-band"),
    select: v("--plot-select"),
    series: [1, 2, 3, 4, 5].map((i) => v(`--plot-series-${i}`)),
    xyz: [v("--plot-x"), v("--plot-y"), v("--plot-z")],
    heat: [0, 1, 2, 3, 4].map((i) => v(`--vp-field-${i}`)),
    font: v("--font-sans"),
    mono: v("--font-mono"),
    fontSize: px("--plot-font"),
    tickGap: px("--plot-tick-gap"),
    lineW: px("--plot-line-w"),
    selectW: px("--plot-select-w"),
    markerR: px("--plot-marker-r"),
    point: px("--plot-point"),
  };
}

export interface Axis {
  label: string;
  domain: [number, number];
  log?: boolean;
}

export type Layer =
  /** Histogram: filled bars (`fill`) or an outline (`stroke`, overlays). */
  | { kind: "hist"; lo: number; hi: number; counts: ArrayLike<number>; fill?: string; stroke?: string; width?: number }
  | { kind: "line"; xs: ArrayLike<number>; ys: ArrayLike<number>; color: string; width?: number }
  | { kind: "vline"; x: number; color: string; dash?: number[]; width?: number }
  | { kind: "band"; x0: number; x1: number; color: string }
  | { kind: "raster"; raster: Raster; x: [number, number]; y: [number, number] }
  | { kind: "marker"; x: number; y: number; color: string };

export interface ChartSpec {
  x: Axis;
  y: Axis;
  layers: Layer[];
  /** Same mm per pixel on both axes (detector maps). */
  equal?: boolean;
  legend?: { label: string; color: string }[];
  title?: string;
}

export interface Frame {
  /** Plot area in CSS px. */
  left: number;
  top: number;
  width: number;
  height: number;
  x: [number, number];
  y: [number, number];
  logY: boolean;
  logX: boolean;
}

const tr = (v: number, log: boolean | undefined) => (log ? Math.log10(Math.max(v, 1e-300)) : v);

export function toPx(f: Frame, x: number, y: number): [number, number] {
  const tx = (tr(x, f.logX) - tr(f.x[0], f.logX)) / (tr(f.x[1], f.logX) - tr(f.x[0], f.logX));
  const ty = (tr(y, f.logY) - tr(f.y[0], f.logY)) / (tr(f.y[1], f.logY) - tr(f.y[0], f.logY));
  return [f.left + tx * f.width, f.top + (1 - ty) * f.height];
}

export function fromPx(f: Frame, px: number, py: number): [number, number] {
  const tx = (px - f.left) / f.width;
  const ty = 1 - (py - f.top) / f.height;
  const un = (t: number, d: [number, number], log: boolean) =>
    log ? 10 ** (Math.log10(d[0]) + t * (Math.log10(d[1]) - Math.log10(d[0]))) : d[0] + t * (d[1] - d[0]);
  return [un(tx, f.x, f.logX), un(ty, f.y, f.logY)];
}

/** 1-2-5 ticks covering [lo, hi]. */
export function niceTicks(lo: number, hi: number, target: number): number[] {
  const span = hi - lo;
  if (!(span > 0) || !Number.isFinite(span)) return [lo];
  const raw = span / Math.max(1, target);
  const p = 10 ** Math.floor(Math.log10(raw));
  const m = raw / p;
  const step = (m < 1.5 ? 1 : m < 3.5 ? 2 : m < 7.5 ? 5 : 10) * p;
  const out: number[] = [];
  for (let v = Math.ceil(lo / step - 1e-9) * step; v <= hi + step * 1e-9; v += step) out.push(Math.abs(v) < step * 1e-9 ? 0 : v);
  return out;
}

function logTicks(lo: number, hi: number): number[] {
  const out: number[] = [];
  for (let e = Math.floor(Math.log10(lo)); e <= Math.ceil(Math.log10(hi)); e++) {
    const v = 10 ** e;
    if (v >= lo * 0.999 && v <= hi * 1.001) out.push(v);
  }
  return out;
}

/** Short tick text: enough digits to tell neighbouring ticks apart. */
export function tickText(v: number, step: number): string {
  if (v === 0) return "0";
  const a = Math.abs(v);
  if (a >= 1e5 || a < 1e-3) return v.toExponential(a >= 1e5 ? 0 : 1).replace("e+", "e");
  const digits = Math.max(0, Math.min(6, -Math.floor(Math.log10(Math.abs(step) || 1) + 1e-9)));
  return v.toFixed(digits);
}

/** Lays out axes for `spec` in a w×h box (CSS px) and paints everything. Returns the plot frame. */
export function drawChart(p: Painter, spec: ChartSpec, w: number, h: number, t: PlotTheme, x0 = 0, y0 = 0): Frame {
  const fs = t.fontSize;
  // Small multiples: the x label moves up next to the title and the y label (the title carries
  // the unit) is dropped, leaving the height to the data.
  const compact = !!spec.title && h < fs * 17;
  const titleH = spec.title ? fs * 1.8 : 0;
  const pad = fs * 0.8;
  const logY = !!spec.y.log;
  const logX = !!spec.x.log;
  let y = [...spec.y.domain] as [number, number];
  let x = [...spec.x.domain] as [number, number];
  if (logY) y = [Math.max(y[0], 0.5), Math.max(y[1], 1)];
  // y tick labels set the left margin.
  const yTarget = Math.max(2, Math.floor(h / (fs * (compact ? 3.2 : 4))));
  const yTicks = logY ? logTicks(y[0], y[1]) : niceTicks(y[0], y[1], yTarget);
  const yStep = yTicks.length > 1 ? yTicks[1]! - yTicks[0]! : 1;
  const yText = yTicks.map((v) => (logY ? tickText(v, v) : tickText(v, yStep)));
  const left = x0 + pad + (compact ? 0 : fs * 1.6) + Math.max(fs * 2, ...yText.map((s) => p.measure(s, true))) + fs * 0.5;
  const right = x0 + w - pad - fs * 0.5;
  const top = y0 + pad + titleH;
  const bottom = y0 + h - pad - fs * (compact ? 1.6 : 3.1);
  const width = Math.max(10, right - left);
  const height = Math.max(10, bottom - top);
  const fl = left;
  const ft = top;
  if (spec.equal && !logX && !logY) {
    // Same scale both ways: widen the shorter domain around its centre.
    const sx = (x[1] - x[0]) / width;
    const sy = (y[1] - y[0]) / height;
    if (sx > sy) {
      const c = (y[0] + y[1]) / 2;
      const half = (sx * height) / 2;
      y = [c - half, c + half];
    } else {
      const c = (x[0] + x[1]) / 2;
      const half = (sy * width) / 2;
      x = [c - half, c + half];
    }
  }
  const frame: Frame = { left: fl, top: ft, width, height, x, y, logX, logY };

  p.rect(x0, y0, w, h, t.bg);
  if (spec.title) p.text(spec.title, x0 + pad, y0 + pad, { color: t.textStrong, baseline: "top", bold: true });

  const yt = spec.equal ? niceTicks(y[0], y[1], yTarget) : yTicks;
  const ytStep = yt.length > 1 ? yt[1]! - yt[0]! : yStep;
  const xt = logX ? logTicks(x[0], x[1]) : niceTicks(x[0], x[1], Math.max(2, Math.floor(width / (fs * 7))));
  const xStep = xt.length > 1 ? xt[1]! - xt[0]! : 1;

  // A short plot (small multiples) keeps every grid line but only labels that do not touch the last one.
  let lastLabelY = Infinity;
  for (const v of yt) {
    if (v < y[0] || v > y[1]) continue;
    const [, py] = toPx(frame, x[0], v);
    p.path([fl, py, fl + width, py], { stroke: t.grid, width: 1 });
    if (Math.abs(lastLabelY - py) < t.tickGap) continue;
    lastLabelY = py;
    p.text(logY ? tickText(v, v) : tickText(v, ytStep), fl - fs * 0.5, py, { color: t.text, align: "right", baseline: "middle", mono: true });
  }
  for (const v of xt) {
    if (v < x[0] || v > x[1]) continue;
    const [px] = toPx(frame, v, y[0]);
    p.path([px, ft, px, ft + height], { stroke: t.grid, width: 1 });
    p.text(tickText(v, xStep), px, ft + height + fs * 0.5, { color: t.text, align: "center", baseline: "top", mono: true });
  }

  p.clip(fl, ft, width, height);
  for (const layer of spec.layers) drawLayer(p, frame, layer, t);
  p.unclip();

  p.path([fl, ft, fl, ft + height, fl + width, ft + height], { stroke: t.axis, width: 1 });
  if (compact) {
    p.text(spec.x.label, x0 + w - pad, y0 + pad, { color: t.text, align: "right", baseline: "top" });
  } else {
    p.text(spec.x.label, fl + width / 2, ft + height + fs * 2, { color: t.textStrong, align: "center", baseline: "top" });
    p.text(spec.y.label, x0 + pad, ft + height / 2, { color: t.textStrong, align: "center", baseline: "top", vertical: true });
  }

  if (spec.legend?.length) {
    let ly = ft + fs * 0.6;
    const lx = fl + width - fs * 0.6;
    for (const item of spec.legend) {
      const tw = p.measure(item.label);
      p.rect(lx - tw - fs * 1.6, ly - fs * 0.15, fs, fs * 0.3, item.color);
      p.text(item.label, lx, ly, { color: t.textStrong, align: "right", baseline: "middle" });
      ly += fs * 1.5;
    }
  }
  return frame;
}

function drawLayer(p: Painter, f: Frame, layer: Layer, t: PlotTheme) {
  switch (layer.kind) {
    case "hist": {
      const n = layer.counts.length;
      if (n === 0) return;
      const w = (layer.hi - layer.lo) / n;
      const base = f.logY ? f.y[0] : 0;
      const pts: number[] = [];
      const [sx, sy] = toPx(f, layer.lo, base);
      pts.push(sx, sy);
      for (let i = 0; i < n; i++) {
        const c = Math.max(layer.counts[i]!, base);
        const [ax, ay] = toPx(f, layer.lo + i * w, c);
        const [bx] = toPx(f, layer.lo + (i + 1) * w, c);
        pts.push(ax, ay, bx, ay);
      }
      const [ex, ey] = toPx(f, layer.hi, base);
      pts.push(ex, ey);
      if (layer.fill) p.path(pts, { fill: layer.fill });
      if (layer.stroke) p.path(pts, { stroke: layer.stroke, width: layer.width ?? t.lineW });
      return;
    }
    case "line": {
      const n = Math.min(layer.xs.length, layer.ys.length);
      const pts = new Float64Array(n * 2);
      for (let i = 0; i < n; i++) {
        const [px, py] = toPx(f, layer.xs[i]!, layer.ys[i]!);
        pts[i * 2] = px;
        pts[i * 2 + 1] = py;
      }
      p.path(pts, { stroke: layer.color, width: layer.width ?? t.lineW });
      return;
    }
    case "vline": {
      const [px] = toPx(f, layer.x, f.y[0]);
      p.path([px, f.top, px, f.top + f.height], { stroke: layer.color, width: layer.width ?? 1, dash: layer.dash });
      return;
    }
    case "band": {
      const [a] = toPx(f, layer.x0, f.y[0]);
      const [b] = toPx(f, layer.x1, f.y[0]);
      p.rect(Math.min(a, b), f.top, Math.abs(b - a), f.height, layer.color);
      return;
    }
    case "raster": {
      const [ax, ay] = toPx(f, layer.x[0], layer.y[1]);
      const [bx, by] = toPx(f, layer.x[1], layer.y[0]);
      p.image(layer.raster, ax, ay, bx - ax, by - ay);
      return;
    }
    case "marker": {
      const [px, py] = toPx(f, layer.x, layer.y);
      p.circle(px, py, t.markerR, { stroke: t.bg, width: t.selectW * 2 });
      p.circle(px, py, t.markerR, { stroke: layer.color, width: t.selectW });
      return;
    }
  }
}

/** Several charts in one figure (export of a tab with small multiples). */
export interface Panel {
  spec: ChartSpec;
  x: number;
  y: number;
  w: number;
  h: number;
}

export function drawFigure(p: Painter, panels: Panel[], t: PlotTheme, width: number, height: number) {
  // Gaps between small multiples get the plot background too (no transparent seams).
  p.rect(0, 0, width, height, t.bg);
  for (const panel of panels) drawChart(p, panel.spec, panel.w, panel.h, t, panel.x, panel.y);
}

// ------------------------------------------------------------------------------------- colour

function parseColor(c: string): [number, number, number] {
  const hex = c.trim().replace(/^#/, "");
  if (/^[0-9a-f]{6}$/i.test(hex)) {
    const n = parseInt(hex, 16);
    return [(n >> 16) & 255, (n >> 8) & 255, n & 255];
  }
  const m = c.match(/[\d.]+/g);
  return m ? [Number(m[0]), Number(m[1]), Number(m[2])] : [128, 128, 128];
}

/** 256-entry RGB lookup through the stops (linear in sRGB). */
export function colorRamp(stops: string[]): Uint8Array {
  const rgb = stops.map(parseColor);
  const lut = new Uint8Array(256 * 3);
  for (let i = 0; i < 256; i++) {
    const t = (i / 255) * (rgb.length - 1);
    const k = Math.min(rgb.length - 2, Math.floor(t));
    const f = t - k;
    for (let c = 0; c < 3; c++) lut[i * 3 + c] = Math.round(rgb[k]![c]! + (rgb[k + 1]![c]! - rgb[k]![c]!) * f);
  }
  return lut;
}

/** Counts (row 0 = bottom) to an RGBA raster (row 0 = top); empty cells stay transparent. */
export function heatRaster(counts: Float32Array, nx: number, ny: number, lut: Uint8Array, log: boolean): Raster {
  let max = 0;
  for (let i = 0; i < counts.length; i++) if (counts[i]! > max) max = counts[i]!;
  const data = new Uint8ClampedArray(nx * ny * 4);
  const norm = log ? 1 / Math.log1p(max) : 1 / max;
  for (let row = 0; row < ny; row++) {
    for (let col = 0; col < nx; col++) {
      const c = counts[row * nx + col]!;
      if (c <= 0) continue;
      const v = log ? Math.log1p(c) * norm : c * norm;
      const k = Math.round(Math.min(1, v) * 255) * 3;
      const o = ((ny - 1 - row) * nx + col) * 4;
      data[o] = lut[k]!;
      data[o + 1] = lut[k + 1]!;
      data[o + 2] = lut[k + 2]!;
      data[o + 3] = 255;
    }
  }
  return { width: nx, height: ny, data };
}

/** Points as small squares in a raster of w×h px (row 0 = top), coloured by `colorOf(i)` (0..1). */
export function pointRaster(
  xs: Float32Array,
  ys: Float32Array,
  idx: Uint32Array,
  xr: [number, number],
  yr: [number, number],
  w: number,
  h: number,
  size: number,
  lut: Uint8Array,
  colorOf: (i: number) => number,
): Raster {
  const data = new Uint8ClampedArray(w * h * 4);
  const sx = w / (xr[1] - xr[0]);
  const sy = h / (yr[1] - yr[0]);
  const half = Math.floor(size / 2);
  for (let k = 0; k < idx.length; k++) {
    const i = idx[k]!;
    const px = Math.floor((xs[i]! - xr[0]) * sx) - half;
    const py = Math.floor((yr[1] - ys[i]!) * sy) - half;
    const c = Math.round(Math.min(1, Math.max(0, colorOf(i))) * 255) * 3;
    for (let dy = 0; dy < size; dy++) {
      const yy = py + dy;
      if (yy < 0 || yy >= h) continue;
      for (let dx = 0; dx < size; dx++) {
        const xx = px + dx;
        if (xx < 0 || xx >= w) continue;
        const o = (yy * w + xx) * 4;
        data[o] = lut[c]!;
        data[o + 1] = lut[c + 1]!;
        data[o + 2] = lut[c + 2]!;
        data[o + 3] = 230;
      }
    }
  }
  return { width: w, height: h, data };
}
