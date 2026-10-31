/** Two-finger camera gesture: one step from the previous to the current finger positions (CSS px). */
export interface Point {
  x: number;
  y: number;
}

export interface PinchStep {
  /** Camera distance factor (Camera.zoomAt): fingers moving apart zoom in (< 1). */
  zoom: number;
  /** Midpoint of the fingers now: the point kept fixed while zooming. */
  mid: Point;
  /** Movement of the midpoint (Camera.pan, same convention as a pan drag). */
  pan: Point;
}

const midpoint = (a: Point, b: Point): Point => ({ x: (a.x + b.x) / 2, y: (a.y + b.y) / 2 });

export function pinchStep(prev: [Point, Point], next: [Point, Point]): PinchStep {
  const d0 = Math.hypot(prev[1].x - prev[0].x, prev[1].y - prev[0].y);
  const d1 = Math.hypot(next[1].x - next[0].x, next[1].y - next[0].y);
  const m0 = midpoint(prev[0], prev[1]);
  const mid = midpoint(next[0], next[1]);
  return { zoom: d0 > 0 && d1 > 0 ? d0 / d1 : 1, mid, pan: { x: mid.x - m0.x, y: mid.y - m0.y } };
}
