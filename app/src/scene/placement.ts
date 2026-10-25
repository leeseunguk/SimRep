import type { Bounds, Vec3 } from "./mesh";

/**
 * Where an array sits on the workbench (a SIMION .iob instance, or the grid origin of an imported
 * model): array grid point (0,0,0) at `origin` (mm), the array's unit x/y/z axes in workbench
 * coordinates, array spacing multiplied by `scale`. Mirrors `Placement` in src-tauri/src/fly.rs.
 */
export interface Placement {
  origin: Vec3;
  axes: [Vec3, Vec3, Vec3];
  scale: number;
}

export const IDENTITY: Placement = { origin: [0, 0, 0], axes: [[1, 0, 0], [0, 1, 0], [0, 0, 1]], scale: 1 };

export function isIdentity(p: Placement | null | undefined): boolean {
  if (!p) return true;
  return p.scale === 1 && p.origin.every((v) => v === 0) && p.axes.every((a, i) => a.every((v, j) => v === (i === j ? 1 : 0)));
}

export function isRotated(p: Placement | null | undefined): boolean {
  return !!p && !p.axes.every((a, i) => a.every((v, j) => Math.abs(v - (i === j ? 1 : 0)) < 1e-12));
}

/** Array mm to workbench mm. */
export function toWorld(p: Placement, q: Vec3): Vec3 {
  const { origin: o, axes: a, scale: s } = p;
  return [0, 1, 2].map((k) => o[k]! + s * (a[0][k]! * q[0] + a[1][k]! * q[1] + a[2][k]! * q[2])) as Vec3;
}

/** A direction in array mm to workbench mm (scaled, not translated). */
export function dirToWorld(p: Placement, d: Vec3): Vec3 {
  const { axes: a, scale: s } = p;
  return [0, 1, 2].map((k) => s * (a[0][k]! * d[0] + a[1][k]! * d[1] + a[2][k]! * d[2])) as Vec3;
}

/** Workbench mm to array mm. */
export function toLocal(p: Placement, w: Vec3): Vec3 {
  const d = [0, 1, 2].map((k) => (w[k]! - p.origin[k]!) / p.scale);
  return p.axes.map((a) => a[0] * d[0]! + a[1] * d[1]! + a[2] * d[2]!) as Vec3;
}

export function dirToLocal(p: Placement, d: Vec3): Vec3 {
  return p.axes.map((a) => (a[0] * d[0] + a[1] * d[1] + a[2] * d[2]) / p.scale) as Vec3;
}

/** Axis-aligned workbench bounds of a box in array mm. */
export function boundsToWorld(p: Placement | null | undefined, b: Bounds): Bounds {
  if (!p || isIdentity(p)) return b;
  const min: Vec3 = [Infinity, Infinity, Infinity];
  const max: Vec3 = [-Infinity, -Infinity, -Infinity];
  for (let c = 0; c < 8; c++) {
    const w = toWorld(p, [c & 1 ? b.max[0] : b.min[0], c & 2 ? b.max[1] : b.min[1], c & 4 ? b.max[2] : b.min[2]]);
    for (let k = 0; k < 3; k++) {
      min[k] = Math.min(min[k]!, w[k]!);
      max[k] = Math.max(max[k]!, w[k]!);
    }
  }
  return { min, max };
}

/** Column-major 4x4 model matrix (array mm to workbench mm) for WGSL. */
export function modelMatrix(p: Placement | null | undefined): Float32Array {
  const q = p ?? IDENTITY;
  const m = new Float32Array(16);
  for (let c = 0; c < 3; c++) for (let r = 0; r < 3; r++) m[c * 4 + r] = q.axes[c]![r]! * q.scale;
  m[12] = q.origin[0];
  m[13] = q.origin[1];
  m[14] = q.origin[2];
  m[15] = 1;
  return m;
}
