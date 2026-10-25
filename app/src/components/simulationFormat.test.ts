import { describe, expect, it } from "vitest";
import type { PlotTheme } from "../results/plot/chart";
import type { RefineState } from "../state/store";
import { decade, refineProgress, residualChart, stageLabel } from "./simulationFormat";

const refine = (patch: Partial<RefineState>): RefineState => ({
  job: 1, status: "running", device: null, stages: [1, 2, 3], stage: 0, cycle: 0, residual: null,
  history: [], past: [], tolerance: 1e-10, elapsedMs: 0, note: null, done: null, original: null, ...patch,
});
const theme = { primary: "#f80", axis: "#888", series: ["#00f", "#0f0"] } as PlotTheme;
const labels = { title: "Relative residual", x: "V-cycle", target: "Target 1e-10", earlier: "Earlier solutions" };

describe("refine progress", () => {
  it("advances by log residual within a solution and by solution overall", () => {
    expect(refineProgress(refine({ history: [1], residual: 1 }))).toBe(0);
    expect(refineProgress(refine({ history: [1, 1e-5], residual: 1e-5 }))).toBeCloseTo(0.5 / 3, 12);
    expect(refineProgress(refine({ stage: 2, history: [1, 1e-10], residual: 1e-10 }))).toBe(1);
    expect(refineProgress(refine({ status: "done" }))).toBe(1);
  });

  it("follows the target the core names", () => {
    expect(refineProgress(refine({ tolerance: 1e-6, history: [1, 1e-3], residual: 1e-3 }))).toBeCloseTo(0.5 / 3, 12);
  });

  it("names the solution being refined", () => {
    expect(stageLabel(refine({ stage: 1 }))).toEqual({ key: "refine.stage.electrode", params: { i: 2, n: 3, k: 2 } });
    expect(stageLabel(refine({ stages: [null] }))).toEqual({ key: "refine.stage.single" });
    expect(stageLabel(refine({ stages: [0, 1] }))).toEqual({ key: "refine.stage.fixed", params: { i: 1, n: 2 } });
  });
});

describe("residual chart", () => {
  it("draws the current solution over earlier ones, with the target in view", () => {
    const spec = residualChart(refine({ stage: 1, past: [[1, 1e-4, 5e-11]], history: [1, 1e-3, 1e-6] }), theme, labels);
    expect(spec.y.log).toBe(true);
    expect(spec.y.domain[0]).toBeLessThan(5e-11);
    expect(spec.x.domain).toEqual([0, 4]);
    const lines = spec.layers.filter((l) => l.kind === "line");
    expect(lines.map((l) => l.kind === "line" && l.color)).toEqual(["#0f0", "#888", "#f80"]);
    const target = lines[0]!;
    expect(target.kind === "line" && Array.from(target.ys)).toEqual([1e-10, 1e-10]);
    expect(spec.legend?.map((l) => l.label)).toEqual(["Target 1e-10", "Earlier solutions"]);
  });

  it("starts with an empty curve before the first residual", () => {
    const spec = residualChart(refine({}), theme, labels);
    expect(spec.y.domain[0]).toBeLessThan(1e-10);
    expect(spec.legend).toHaveLength(1);
  });

  it("labels decades compactly", () => {
    expect([1, 0.1, 1e-10].map(decade)).toEqual(["1", "1e-1", "1e-10"]);
  });
});
