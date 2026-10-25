import type { Flight, FlightHeader, Surface } from "../scene/flight";

/**
 * Per-ion columns of a flight and the statistics the results panel plots. Everything works on
 * typed arrays in single passes: 100k ions bin in about a millisecond, so plots can follow a live
 * re-fly (see plotPerf).
 */

export type Quantity = "tof" | "ke" | "x" | "y" | "z" | "r";
export const QUANTITIES: readonly Quantity[] = ["tof", "ke", "x", "y", "z", "r"];
export const UNITS: Record<Quantity, string> = { tof: "µs", ke: "eV", x: "mm", y: "mm", z: "mm", r: "mm" };

/** One run's end states, from the current flight or a kept snapshot (arrays are shared, never mutated). */
export interface RunData {
  n: number;
  cols: Record<Quantity, Float64Array>;
  /** Index into `surfaces` per ion. */
  surface: Uint32Array;
  surfaces: FlightHeader["surfaces"];
}

const runs = new WeakMap<Flight, RunData>();

/** Columns of a flight (computed once per flight object). */
export function runOf(f: Flight): RunData {
  let run = runs.get(f);
  if (run) return run;
  const n = f.header.ions;
  const x = new Float64Array(n);
  const y = new Float64Array(n);
  const z = new Float64Array(n);
  const r = new Float64Array(n);
  const tof = new Float64Array(n);
  const e = f.ends;
  for (let i = 0; i < n; i++) {
    const k = i * 4;
    x[i] = e[k]!;
    y[i] = e[k + 1]!;
    z[i] = e[k + 2]!;
    tof[i] = e[k + 3]!;
    r[i] = Math.hypot(e[k + 1]!, e[k + 2]!);
  }
  run = { n, cols: { tof, ke: f.ke, x, y, z, r }, surface: f.surface, surfaces: f.header.surfaces };
  runs.set(f, run);
  return run;
}

/** Stable identity of an end surface across runs ("electrode:2", "wall:+x", ...). */
export function surfaceKey(s: Surface): string {
  switch (s.kind) {
    case "electrode":
      return s.number !== null ? `electrode:${s.number}` : `electrode:V${s.voltage ?? "?"}`;
    case "wall":
      return `wall:${s.face}`;
    default:
      return s.kind;
  }
}

export function landed(s: Surface): boolean {
  return s.kind === "electrode" || s.kind === "wall";
}

/** "landed" (electrodes + walls), "all", or one surfaceKey. */
export type IonFilter = string;

/** Which of a run's surfaces pass the filter (index = surface index). */
export function surfaceMask(run: RunData, filter: IonFilter): Uint8Array {
  return Uint8Array.from(run.surfaces, (s) =>
    filter === "all" ? 1 : filter === "landed" ? Number(landed(s.surface)) : Number(surfaceKey(s.surface) === filter),
  );
}

/** Values of `q` for ions passing the filter (and inside [lo, hi] when given). */
export function select(run: RunData, q: Quantity, filter: IonFilter, range?: [number, number] | null): Float64Array {
  const mask = surfaceMask(run, filter);
  const col = run.cols[q];
  const out = new Float64Array(run.n);
  let k = 0;
  const lo = range ? range[0] : -Infinity;
  const hi = range ? range[1] : Infinity;
  for (let i = 0; i < run.n; i++) {
    if (!mask[run.surface[i]!]) continue;
    const v = col[i]!;
    if (v >= lo && v <= hi && Number.isFinite(v)) out[k++] = v;
  }
  return out.subarray(0, k);
}

/** Indices of ions passing the filter. */
export function indicesOf(run: RunData, filter: IonFilter): Uint32Array {
  const mask = surfaceMask(run, filter);
  const out = new Uint32Array(run.n);
  let k = 0;
  for (let i = 0; i < run.n; i++) if (mask[run.surface[i]!]) out[k++] = i;
  return out.subarray(0, k);
}

export function extent(values: ArrayLike<number>): [number, number] | null {
  let lo = Infinity;
  let hi = -Infinity;
  for (let i = 0; i < values.length; i++) {
    const v = values[i]!;
    if (v < lo) lo = v;
    if (v > hi) hi = v;
  }
  return lo <= hi ? [lo, hi] : null;
}

/** A range that can be binned: a degenerate one grows to show its single value in the middle. */
export function padRange([lo, hi]: [number, number]): [number, number] {
  if (hi > lo) return [lo, hi];
  const pad = Math.abs(lo) * 1e-3 || 0.5;
  return [lo - pad, hi + pad];
}

/** Rice rule, clamped: ~90 bins for 100k ions, ~43 for 10k. */
export function autoBins(n: number): number {
  return Math.min(200, Math.max(10, Math.round(2 * Math.cbrt(n))));
}

export interface Histogram {
  lo: number;
  hi: number;
  counts: Float64Array;
}

export function histogram(values: ArrayLike<number>, lo: number, hi: number, bins: number): Histogram {
  const counts = new Float64Array(bins);
  const scale = bins / (hi - lo);
  for (let i = 0; i < values.length; i++) {
    const v = values[i]!;
    if (v < lo || v > hi) continue;
    counts[Math.min(bins - 1, Math.floor((v - lo) * scale))]!++;
  }
  return { lo, hi, counts };
}

/**
 * y range for overlaid histograms. Linear: from 0 with 8% headroom. Log: from `floor`, 0.5 for ion
 * counts; for fractions (normalized) the decade at or below half the smallest non-zero bin, so a bin
 * of one ion still shows and the axis bottom carries a tick label.
 */
export function histYRange(counts: ArrayLike<number>[], log: boolean, fractions: boolean): { domain: [number, number]; floor: number } {
  let max = 0;
  let min = Infinity;
  for (const c of counts) {
    for (let i = 0; i < c.length; i++) {
      const v = c[i]!;
      max = Math.max(max, v);
      if (v > 0) min = Math.min(min, v);
    }
  }
  const floor = fractions && Number.isFinite(min) ? 10 ** Math.floor(Math.log10(min / 2)) : 0.5;
  return { domain: log ? [floor, max * 2] : [0, (max || 1) * 1.08], floor };
}

export interface Stats {
  n: number;
  mean: number;
  std: number;
  min: number;
  max: number;
}

export function stats(values: ArrayLike<number>): Stats | null {
  const n = values.length;
  if (n === 0) return null;
  let sum = 0;
  let min = Infinity;
  let max = -Infinity;
  for (let i = 0; i < n; i++) {
    const v = values[i]!;
    sum += v;
    if (v < min) min = v;
    if (v > max) max = v;
  }
  const mean = sum / n;
  let sq = 0;
  for (let i = 0; i < n; i++) sq += (values[i]! - mean) ** 2;
  return { n, mean, std: Math.sqrt(sq / n), min, max };
}

export interface Fwhm {
  /** Centre of the tallest bin. */
  peak: number;
  left: number;
  right: number;
  width: number;
}

/** Full width at half maximum of the tallest peak, half-max crossings interpolated between bin centres. */
export function fwhm(h: Histogram): Fwhm | null {
  const c = h.counts;
  const bins = c.length;
  if (bins < 3) return null;
  let top = 0;
  for (let i = 1; i < bins; i++) if (c[i]! > c[top]!) top = i;
  const max = c[top]!;
  if (max <= 0) return null;
  const half = max / 2;
  const w = (h.hi - h.lo) / bins;
  const centre = (i: number) => h.lo + (i + 0.5) * w;
  let l = top;
  while (l > 0 && c[l - 1]! > half) l--;
  let r = top;
  while (r < bins - 1 && c[r + 1]! > half) r++;
  // Crossing between bin l-1 (<= half) and l (> half); the histogram edge if the peak touches it.
  const left = l === 0 ? h.lo : centre(l - 1) + ((half - c[l - 1]!) / (c[l]! - c[l - 1]!)) * w;
  const right = r === bins - 1 ? h.hi : centre(r) + ((c[r]! - half) / (c[r]! - c[r + 1]!)) * w;
  return { peak: centre(top), left, right, width: right - left };
}

/** Mass resolving power m/Δm = t / (2 Δt) for a TOF peak at t with FWHM Δt. */
export function resolvingPower(f: Fwhm): number {
  return f.width > 0 ? f.peak / (2 * f.width) : Infinity;
}

// ------------------------------------------------------------------------------- detector map

export type MapAxis = "x" | "y" | "z" | "r";
export type MapPlane = "auto" | "zy" | "xy" | "xz" | "xr";
export const PLANES: readonly Exclude<MapPlane, "auto">[] = ["zy", "xy", "xz", "xr"];

/** Axes of a detector map: the face's own plane for walls, x–r otherwise (electrodes, radius). */
export function planeAxes(plane: MapPlane, surface: Surface | null): [MapAxis, MapAxis] {
  const p =
    plane !== "auto"
      ? plane
      : surface?.kind === "wall" && surface.face !== "r"
        ? ({ x: "zy", y: "xz", z: "xy" } as const)[surface.face[1] as "x" | "y" | "z"]
        : "xr";
  return [p[0] as MapAxis, p[1] as MapAxis];
}

/** Extent of col[i] over the given ion indices. */
export function extentAt(col: Float64Array, idx: Uint32Array): [number, number] | null {
  let lo = Infinity;
  let hi = -Infinity;
  for (let k = 0; k < idx.length; k++) {
    const v = col[idx[k]!]!;
    if (v < lo) lo = v;
    if (v > hi) hi = v;
  }
  return lo <= hi ? [lo, hi] : null;
}

/** Statistics of col[i] over the given ion indices. */
export function statsAt(col: Float64Array, idx: Uint32Array): Stats | null {
  const n = idx.length;
  if (n === 0) return null;
  let sum = 0;
  let min = Infinity;
  let max = -Infinity;
  for (let k = 0; k < n; k++) {
    const v = col[idx[k]!]!;
    sum += v;
    if (v < min) min = v;
    if (v > max) max = v;
  }
  const mean = sum / n;
  let sq = 0;
  for (let k = 0; k < n; k++) sq += (col[idx[k]!]! - mean) ** 2;
  return { n, mean, std: Math.sqrt(sq / n), min, max };
}

/** 2D histogram of (xs, ys) for the given ion indices, row 0 = bottom (low y). */
export function heat(xs: Float64Array, ys: Float64Array, idx: Uint32Array, xr: [number, number], yr: [number, number], nx: number, ny: number): Float32Array {
  const out = new Float32Array(nx * ny);
  const sx = nx / (xr[1] - xr[0]);
  const sy = ny / (yr[1] - yr[0]);
  for (let k = 0; k < idx.length; k++) {
    const i = idx[k]!;
    const fx = (xs[i]! - xr[0]) * sx;
    const fy = (ys[i]! - yr[0]) * sy;
    if (!(fx >= 0 && fx <= nx && fy >= 0 && fy <= ny)) continue;
    out[Math.min(ny - 1, Math.floor(fy)) * nx + Math.min(nx - 1, Math.floor(fx))]!++;
  }
  return out;
}

/** Nearest ion to (x, y) among `idx` in a map's axes, within `maxDist` (data units, per axis scaled). */
export function nearestIon(xs: Float64Array, ys: Float64Array, idx: Uint32Array, x: number, y: number, sx: number, sy: number, maxPx: number): number | null {
  let best = -1;
  let bestD = maxPx * maxPx;
  for (let k = 0; k < idx.length; k++) {
    const i = idx[k]!;
    const d = ((xs[i]! - x) * sx) ** 2 + ((ys[i]! - y) * sy) ** 2;
    if (d < bestD) {
      bestD = d;
      best = i;
    }
  }
  return best >= 0 ? best : null;
}
