import { describe, expect, it } from "vitest";
import { autoBins, extent, fwhm, heat, histogram, histYRange, nearestIon, padRange, resolvingPower, stats } from "./analysis";

describe("histogram statistics", () => {
  it("bins inclusive of both edges", () => {
    const h = histogram([0, 0.5, 1, 1.5, 2, 3], 0, 2, 4);
    expect(Array.from(h.counts)).toEqual([1, 1, 1, 2]);
  });

  it("finds the FWHM of a symmetric peak and its resolving power", () => {
    // Counts 0 2 4 2 0 over [0, 5): half max 2 is reached at the centres of bins 1 and 3.
    const h = { lo: 0, hi: 5, counts: new Float64Array([0, 2, 4, 2, 0]) };
    const f = fwhm(h)!;
    expect(f.peak).toBe(2.5);
    expect(f.left).toBeCloseTo(1.5, 12);
    expect(f.right).toBeCloseTo(3.5, 12);
    expect(resolvingPower(f)).toBeCloseTo(2.5 / 4, 12);
    expect(fwhm({ lo: 0, hi: 1, counts: new Float64Array(5) })).toBeNull();
  });

  it("starts a normalized log axis below the smallest fraction, a count axis at 0.5", () => {
    const fractions = [new Float64Array([0, 1e-4, 0.6]), new Float64Array([0.25, 0, 0.75])];
    const normalized = histYRange(fractions, true, true);
    // 10 ** -5 differs by one ULP between V8 versions. The floor is the decade
    // below half the smallest fraction, not an engine-specific bit pattern.
    expect(Math.abs(normalized.floor - 1e-5)).toBeLessThanOrEqual(Number.EPSILON * 1e-5);
    expect(normalized.floor).toBeLessThanOrEqual(1e-4 / 2);
    expect(normalized.floor * 10).toBeGreaterThan(1e-4 / 2);
    expect(normalized.domain).toEqual([normalized.floor, 1.5]);
    const counts = [new Float64Array([0, 3, 40])];
    expect(histYRange(counts, true, false)).toEqual({ domain: [0.5, 80], floor: 0.5 });
    expect(histYRange(counts, false, false).domain).toEqual([0, 40 * 1.08]);
  });

  it("reports population statistics", () => {
    expect(stats([2, 4, 4, 4, 5, 5, 7, 9])).toEqual({ n: 8, mean: 5, std: 2, min: 2, max: 9 });
    expect(stats([])).toBeNull();
  });

  it("chooses bins by the Rice rule within 10..200", () => {
    expect(autoBins(1)).toBe(10);
    expect(autoBins(100_000)).toBe(93);
    expect(autoBins(1e9)).toBe(200);
  });

  it("widens a degenerate range around its value", () => {
    expect(extent([3, -1, 2])).toEqual([-1, 3]);
    expect(extent([])).toBeNull();
    expect(padRange([1, 2])).toEqual([1, 2]);
    expect(padRange([100, 100])).toEqual([99.9, 100.1]);
    expect(padRange([0, 0])).toEqual([-0.5, 0.5]);
  });
});

describe("detector map", () => {
  const xs = new Float64Array([0.1, 0.9, 0.9, 5]);
  const ys = new Float64Array([0.1, 0.1, 0.9, 5]);
  const idx = new Uint32Array([0, 1, 2, 3]);

  it("counts ions per cell, row 0 at the bottom of y", () => {
    // (0.1, 0.1) bottom-left, (0.9, 0.1) bottom-right, (0.9, 0.9) top-right; (5, 5) is outside.
    expect(Array.from(heat(xs, ys, idx, [0, 1], [0, 1], 2, 2))).toEqual([1, 1, 0, 1]);
  });

  it("picks the nearest ion within the pixel radius", () => {
    expect(nearestIon(xs, ys, idx, 0.88, 0.88, 100, 100, 5)).toBe(2);
    expect(nearestIon(xs, ys, idx, 3, 3, 100, 100, 5)).toBeNull();
  });
});
