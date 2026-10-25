import type { Message } from "../i18n";
import type { ChartSpec, Layer, PlotTheme } from "../results/plot/chart";
import type { Surface } from "../scene/flight";
import type { RefineState } from "../state/store";

/** Residual target until the native refine's start message names it (TOLERANCE in src-tauri/src/refine.rs). */
export const REFINE_TOLERANCE = 1e-10;

/** 0..1 over all solutions; within one, log-residual progress from its start to the target. */
export function refineProgress(r: RefineState): number {
  const n = Math.max(r.stages.length, 1);
  if (r.status === "done") return 1;
  const start = r.history[0];
  const now = r.residual;
  let within = 0;
  if (start && now && start > r.tolerance) {
    within = Math.log(start / now) / Math.log(start / r.tolerance);
  }
  return Math.min(1, Math.max(0, (r.stage + Math.min(1, Math.max(0, within))) / n));
}

/** Decade tick text: "1", "1e-4". */
export const decade = (v: number) => (v === 1 ? "1" : v.toExponential(0).replace("e+", "e"));

/**
 * Log-scale convergence chart of a running refine: the current solution's relative residual per
 * V-cycle, earlier solutions behind it, and the target as a dashed line. The y range always
 * reaches below the target so the line stays in view and the scale does not jump as it converges.
 */
export function residualChart(r: RefineState, theme: PlotTheme, labels: { title: string; x: string; target: string; earlier: string }): ChartSpec {
  const series = [...r.past, r.history];
  let lo = r.tolerance;
  let hi = 1;
  let cycles = 4;
  for (const h of series) {
    cycles = Math.max(cycles, h.length - 1);
    for (const v of h) if (v > 0) [lo, hi] = [Math.min(lo, v), Math.max(hi, v)];
  }
  const line = (h: number[], color: string, width?: number): Layer => ({ kind: "line", xs: h.map((_, i) => i), ys: h, color, width });
  const layers: Layer[] = [
    { kind: "line", xs: [0, cycles], ys: [r.tolerance, r.tolerance], color: theme.series[1] ?? theme.axis, dash: [4, 3], width: 1 },
    ...r.past.map((h) => line(h, theme.axis, 1)),
    line(r.history, theme.primary),
  ];
  const legend = [{ label: labels.target, color: theme.series[1] ?? theme.axis }];
  if (r.past.length) legend.push({ label: labels.earlier, color: theme.axis });
  return {
    title: labels.title,
    x: { label: labels.x, domain: [0, cycles] },
    y: { label: "", domain: [lo / 4, hi * 1.5], log: true, floor: 0, format: decade },
    layers,
    legend,
  };
}

/** "Electrode 2 (2 of 3)" for the solution being refined. */
export function stageLabel(r: RefineState): Message {
  const slot = r.stages[r.stage];
  const params = { i: r.stage + 1, n: r.stages.length };
  if (slot === null || slot === undefined) return { key: "refine.stage.single" };
  if (slot === 0) return { key: "refine.stage.fixed", params };
  return { key: "refine.stage.electrode", params: { ...params, k: slot } };
}

export function surfaceLabel(s: Surface): Message {
  switch (s.kind) {
    case "electrode":
      if (s.number !== null) return { key: "fly.surface.electrode", params: { n: s.number } };
      if (s.voltage !== null) return { key: "fly.surface.electrodeV", params: { v: Number(s.voltage.toPrecision(4)) } };
      return { key: "fly.surface.electrodeAny" };
    case "wall":
      return s.face === "r" ? { key: "fly.surface.radius" } : { key: "fly.surface.wall", params: { face: s.face.toUpperCase() } };
    case "timeLimit":
      return { key: "fly.surface.timeLimit" };
    case "stalled":
      return { key: "fly.surface.stalled" };
  }
}
