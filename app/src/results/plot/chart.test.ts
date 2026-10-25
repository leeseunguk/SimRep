import { describe, expect, it } from "vitest";
import { colorRamp, drawChart, fromPx, niceTicks, tickText, toPx, type Frame, type Painter, type PlotTheme } from "./chart";

const theme: PlotTheme = {
  bg: "#000", grid: "#111", axis: "#222", text: "#333", textStrong: "#444", primary: "#f80", primaryFill: "#f808",
  band: "#f802", select: "#fff", series: ["#00f", "#0f0", "#f0f", "#80f", "#ff0"], xyz: ["#f00", "#0f0", "#00f"],
  heat: ["#000000", "#ffffff"], font: "sans", mono: "mono", fontSize: 12, tickGap: 14, lineW: 1.5, selectW: 2, markerR: 5, point: 2,
};

/** Records what a chart paints (text and stroked paths). */
function recorder() {
  const texts: string[] = [];
  const paths: { points: number[]; stroke?: string; dash?: number[] }[] = [];
  const p: Painter = {
    rect: () => {},
    path: (points, style) => void paths.push({ points: Array.from(points), stroke: style.stroke, dash: style.dash }),
    circle: () => {},
    text: (s) => void texts.push(s),
    measure: (s) => s.length * 7,
    image: () => {},
    clip: () => {},
    unclip: () => {},
  };
  return { p, texts, paths };
}

describe("axis helpers", () => {
  it("places 1-2-5 ticks over the range", () => {
    expect(niceTicks(0, 10, 5)).toEqual([0, 2, 4, 6, 8, 10]);
    expect(niceTicks(-0.3, 0.3, 3)).toEqual([-0.2, 0, 0.2]);
    expect(niceTicks(5, 5, 4)).toEqual([5]);
  });

  it("writes ticks with enough digits for their step", () => {
    expect(tickText(0.25, 0.05)).toBe("0.25");
    expect(tickText(2, 1)).toBe("2");
    expect(tickText(1e-7, 1e-7)).toBe("1.0e-7");
    expect(tickText(0, 1)).toBe("0");
  });

  it("maps data to pixels and back, linear and log", () => {
    const f: Frame = { left: 10, top: 5, width: 200, height: 100, x: [0, 10], y: [1e-10, 1], logX: false, logY: true };
    const [px, py] = toPx(f, 5, 1e-5);
    expect(px).toBe(110);
    expect(py).toBeCloseTo(55, 9);
    const [x, y] = fromPx(f, px, py);
    expect(x).toBeCloseTo(5, 12);
    expect(Math.log10(y)).toBeCloseTo(-5, 9);
  });

  it("interpolates a colour ramp into a 256-entry RGB table", () => {
    const lut = colorRamp(["#000000", "#ffffff"]);
    expect(lut.length).toBe(256 * 3);
    expect(Array.from(lut.subarray(0, 3))).toEqual([0, 0, 0]);
    expect(Array.from(lut.subarray(255 * 3))).toEqual([255, 255, 255]);
  });
});

describe("drawChart", () => {
  it("keeps a log axis below 0.5 when the spec sets its floor", () => {
    const { p, texts, paths } = recorder();
    const frame = drawChart(
      p,
      {
        x: { label: "cycle", domain: [0, 10] },
        y: { label: "", domain: [1e-11, 1.5], log: true, floor: 0, format: (v) => v.toExponential(0) },
        layers: [{ kind: "line", xs: [0, 10], ys: [1e-10, 1e-10], color: "#0f0", dash: [4, 3] }],
      },
      320,
      240,
      theme,
    );
    expect(frame.y[0]).toBe(1e-11);
    expect(texts).toContain("1e-10");
    expect(paths.find((s) => s.stroke === "#0f0")?.dash).toEqual([4, 3]);
  });

  it("clamps count axes at 0.5 by default", () => {
    const { p } = recorder();
    const frame = drawChart(p, { x: { label: "", domain: [0, 1] }, y: { label: "", domain: [0, 100], log: true }, layers: [] }, 320, 240, theme);
    expect(frame.y[0]).toBe(0.5);
  });
});
