import { describe, expect, it } from "vitest";
import { pinchStep } from "./touch";

describe("pinchStep", () => {
  it("zooms in when the fingers spread, keeping their midpoint", () => {
    const s = pinchStep([{ x: 100, y: 100 }, { x: 200, y: 100 }], [{ x: 50, y: 100 }, { x: 250, y: 100 }]);
    expect(s.zoom).toBeCloseTo(0.5);
    expect(s.mid).toEqual({ x: 150, y: 100 });
    expect(s.pan).toEqual({ x: 0, y: 0 });
  });

  it("zooms out when the fingers close", () => {
    expect(pinchStep([{ x: 0, y: 0 }, { x: 100, y: 0 }], [{ x: 25, y: 0 }, { x: 75, y: 0 }]).zoom).toBeCloseTo(2);
  });

  it("pans by the midpoint movement at a constant spread", () => {
    const s = pinchStep([{ x: 0, y: 0 }, { x: 100, y: 0 }], [{ x: 10, y: 20 }, { x: 110, y: 20 }]);
    expect(s.zoom).toBeCloseTo(1);
    expect(s.pan).toEqual({ x: 10, y: 20 });
  });

  it("does not zoom on coincident fingers", () => {
    expect(pinchStep([{ x: 5, y: 5 }, { x: 5, y: 5 }], [{ x: 5, y: 5 }, { x: 9, y: 5 }]).zoom).toBe(1);
  });
});
