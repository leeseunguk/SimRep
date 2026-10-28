import type { Message } from "../i18n";
import { surfaceLabel } from "../components/simulationFormat";
import { autoBins, extent, extentAt, fwhm, heat, histogram, indicesOf, landed, mapWindow, padRange, planeAxes, resolvingPower, select, surfaceKey, type Histogram, type MapAxis, type RunData } from "../results/analysis";
import type { FlightHeader, Surface } from "../scene/flight";

/**
 * What the report shows of one flight: metrics, end-state counts and the figure data (TOF and
 * energy histograms, detector map), computed from the per-ion end states when they are at hand
 * (the current flight, a stored run with results) or taken from a stored run's plot summaries.
 * Pure (no store, no DOM): unit tested directly.
 */

export interface MapData {
  surface: Message;
  axes: [MapAxis, MapAxis];
  x: [number, number];
  y: [number, number];
  nx: number;
  ny: number;
  /** Row 0 = low y. */
  counts: Float32Array;
}

export interface ResultSummary {
  origin: { kind: "current" } | { kind: "run"; label: string };
  ions: number;
  arrivals: number;
  splats: { label: Message; count: number }[];
  tof: { mean: number; std: number; min: number; max: number } | null;
  keMean: number | null;
  /** FlightHeader.flyMs: the ion integration alone. */
  computeMs: number;
  tofHist: Histogram | null;
  keHist: Histogram | null;
  /** m/Δm = t / (2 Δt) from the TOF histogram's FWHM. */
  resolution: { value: number; fwhm: number; left: number; right: number; bins: number } | null;
  map: MapData | null;
}

/** Stored-run plot summaries (state/runs.ts RunPlots), structurally. */
export interface StoredPlots {
  histogram: { lo: number; hi: number; counts: number[] } | null;
  detectorMap: { surface: string; axes: [MapAxis, MapAxis]; x: [number, number]; y: [number, number]; nx: number; ny: number; counts: number[] } | null;
}

/** Detector map cells along the longer side. */
export const MAP_BINS = 64;

/** The Surface an analysis.surfaceKey names ("electrode:2", "electrode:V-10", "wall:+x", "timeLimit"). */
export function surfaceFromKey(key: string): Surface {
  const [kind, rest] = key.split(":", 2) as [string, string | undefined];
  if (kind === "electrode") {
    if (rest?.startsWith("V")) {
      const v = Number(rest.slice(1));
      return { kind: "electrode", number: null, voltage: Number.isFinite(v) ? v : null };
    }
    const n = Number(rest);
    return { kind: "electrode", number: Number.isInteger(n) ? n : null, voltage: null };
  }
  if (kind === "wall") return { kind: "wall", face: (rest ?? "r") as Extract<Surface, { kind: "wall" }>["face"] };
  return kind === "stalled" ? { kind: "stalled" } : { kind: "timeLimit" };
}

function histOf(values: Float64Array): Histogram | null {
  const e = extent(values);
  if (!e) return null;
  const [lo, hi] = padRange(e);
  return histogram(values, lo, hi, autoBins(values.length));
}

/** Density of the end points on the surface most ions landed on, square cells in mm. */
function mapOf(run: RunData): MapData | null {
  const best = [...run.surfaces].filter((s) => landed(s.surface)).sort((a, b) => b.count - a.count)[0];
  if (!best) return null;
  const idx = indicesOf(run, surfaceKey(best.surface));
  const axes = planeAxes("auto", best.surface);
  const xs = run.cols[axes[0]];
  const ys = run.cols[axes[1]];
  const ex = extentAt(xs, idx);
  const ey = extentAt(ys, idx);
  // Same window as the detector tab (results/analysis.ts mapWindow): square mm cells, a flat
  // pattern keeps a visible width.
  const w = ex && ey ? mapWindow([{ x: ex, y: ey }], MAP_BINS) : null;
  if (!w) return null;
  const { x, y, nx, ny } = w;
  return { surface: surfaceLabel(best.surface), axes, x, y, nx, ny, counts: heat(xs, ys, idx, x, y, nx, ny) };
}

/**
 * The report's view of a flight. `run`: its per-ion end states (null: a stored run without them,
 * then `plots` supplies the TOF histogram and detector map).
 */
export function summarize(header: FlightHeader, run: RunData | null, plots: StoredPlots | null, origin: ResultSummary["origin"]): ResultSummary {
  const tofHist = run ? histOf(select(run, "tof", "landed")) : plots?.histogram ? { lo: plots.histogram.lo, hi: plots.histogram.hi, counts: Float64Array.from(plots.histogram.counts) } : null;
  const keHist = run ? histOf(select(run, "ke", "landed")) : null;
  const fw = tofHist ? fwhm(tofHist) : null;
  const m = plots?.detectorMap;
  const map = run
    ? mapOf(run)
    : m
      ? { surface: surfaceLabel(surfaceFromKey(m.surface)), axes: m.axes, x: m.x, y: m.y, nx: m.nx, ny: m.ny, counts: Float32Array.from(m.counts) }
      : null;
  return {
    origin,
    ions: header.ions,
    arrivals: header.tof.n,
    splats: [...header.surfaces].sort((a, b) => b.count - a.count).map((s) => ({ label: surfaceLabel(s.surface), count: s.count })),
    tof: header.tof.n > 0 ? { mean: header.tof.mean, std: header.tof.std, min: header.tof.min, max: header.tof.max } : null,
    keMean: Number.isFinite(header.keMean) ? header.keMean : null,
    computeMs: header.flyMs,
    tofHist,
    keHist,
    resolution: fw && tofHist ? { value: resolvingPower(fw), fwhm: fw.width, left: fw.left, right: fw.right, bins: tofHist.counts.length } : null,
    map,
  };
}
