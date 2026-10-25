import type { Message } from "../i18n";
import type { Surface } from "../scene/flight";
import type { RefineState } from "../state/store";

/** Residual target of the native refine (TOLERANCE in src-tauri/src/refine.rs). */
export const REFINE_TOLERANCE = 1e-10;

/** 0..1 over all solutions; within one, log-residual progress from its start to the target. */
export function refineProgress(r: RefineState): number {
  const n = Math.max(r.stages.length, 1);
  if (r.status === "done") return 1;
  const start = r.history[0];
  const now = r.residual;
  let within = 0;
  if (start && now && start > REFINE_TOLERANCE) {
    within = Math.log(start / now) / Math.log(start / REFINE_TOLERANCE);
  }
  return Math.min(1, Math.max(0, (r.stage + Math.min(1, Math.max(0, within))) / n));
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
