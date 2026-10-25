import { FLOATS_PER_VERTEX, type Bounds, type MeshData, type Vec3 } from "../scene/mesh";

export interface Pickable {
  id: string;
  mesh: MeshData;
  scale: number;
  visible: boolean;
}

export interface PickableBox {
  id: string;
  bounds: Bounds;
  visible: boolean;
}

/**
 * CPU ray cast (slab test, then Moller-Trumbore). Runs on click only, so a linear scan is fine.
 * Boxes (potential arrays) are hit by their bounding box.
 */
export function pick(items: readonly Pickable[], origin: Vec3, dir: Vec3, boxes: readonly PickableBox[] = []): string | null {
  let bestT = Infinity;
  let bestId: string | null = null;
  for (const box of boxes) {
    if (!box.visible) continue;
    const t = rayBox(origin, dir, box.bounds);
    if (t !== null && t < bestT) {
      bestT = t;
      bestId = box.id;
    }
  }
  for (const item of items) {
    if (!item.visible) continue;
    // Object space = world / scale. Keeping dir unnormalized keeps t comparable across objects.
    const s = 1 / item.scale;
    const o: Vec3 = [origin[0] * s, origin[1] * s, origin[2] * s];
    const d: Vec3 = [dir[0] * s, dir[1] * s, dir[2] * s];
    const boxT = rayBox(o, d, item.mesh.bounds);
    if (boxT === null || boxT > bestT) continue;
    const t = rayMesh(o, d, item.mesh.vertices, bestT);
    if (t < bestT) {
      bestT = t;
      bestId = item.id;
    }
  }
  return bestId;
}

function rayBox(o: Vec3, d: Vec3, b: Bounds): number | null {
  let tmin = -Infinity;
  let tmax = Infinity;
  for (let a = 0; a < 3; a++) {
    const inv = 1 / d[a]!;
    let t0 = (b.min[a]! - o[a]!) * inv;
    let t1 = (b.max[a]! - o[a]!) * inv;
    if (t0 > t1) [t0, t1] = [t1, t0];
    tmin = Math.max(tmin, t0);
    tmax = Math.min(tmax, t1);
  }
  if (tmax < Math.max(tmin, 0)) return null;
  return Math.max(tmin, 0);
}

function rayMesh(o: Vec3, d: Vec3, v: Float32Array, limit: number): number {
  let best = limit;
  const stride = FLOATS_PER_VERTEX;
  for (let i = 0; i < v.length; i += stride * 3) {
    const ax = v[i]!, ay = v[i + 1]!, az = v[i + 2]!;
    const e1x = v[i + stride]! - ax, e1y = v[i + stride + 1]! - ay, e1z = v[i + stride + 2]! - az;
    const e2x = v[i + 2 * stride]! - ax, e2y = v[i + 2 * stride + 1]! - ay, e2z = v[i + 2 * stride + 2]! - az;
    const px = d[1] * e2z - d[2] * e2y, py = d[2] * e2x - d[0] * e2z, pz = d[0] * e2y - d[1] * e2x;
    const det = e1x * px + e1y * py + e1z * pz;
    if (Math.abs(det) < 1e-12) continue;
    const inv = 1 / det;
    const tx = o[0] - ax, ty = o[1] - ay, tz = o[2] - az;
    const u = (tx * px + ty * py + tz * pz) * inv;
    if (u < 0 || u > 1) continue;
    const qx = ty * e1z - tz * e1y, qy = tz * e1x - tx * e1z, qz = tx * e1y - ty * e1x;
    const w = (d[0] * qx + d[1] * qy + d[2] * qz) * inv;
    if (w < 0 || u + w > 1) continue;
    const t = (e2x * qx + e2y * qy + e2z * qz) * inv;
    if (t > 0 && t < best) best = t;
  }
  return best;
}
