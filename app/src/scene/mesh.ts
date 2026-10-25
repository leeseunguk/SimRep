export type Vec3 = [number, number, number];

export interface Bounds {
  min: Vec3;
  max: Vec3;
}

/**
 * Triangle soup, interleaved per vertex as [px, py, pz, nx, ny, nz] (non-indexed, scene units = mm).
 * Non-indexed keeps STL import trivial and lets flat-shaded and smooth surfaces share one layout.
 */
export interface MeshData {
  vertices: Float32Array<ArrayBuffer>;
  triangleCount: number;
  bounds: Bounds;
}

export const FLOATS_PER_VERTEX = 6;
export const BYTES_PER_VERTEX = FLOATS_PER_VERTEX * 4;

export function computeBounds(vertices: Float32Array): Bounds {
  const min: Vec3 = [Infinity, Infinity, Infinity];
  const max: Vec3 = [-Infinity, -Infinity, -Infinity];
  for (let i = 0; i < vertices.length; i += FLOATS_PER_VERTEX) {
    for (let a = 0; a < 3; a++) {
      const v = vertices[i + a]!;
      if (v < min[a]!) min[a] = v;
      if (v > max[a]!) max[a] = v;
    }
  }
  return { min, max };
}

export function isEmptyBounds(b: Bounds): boolean {
  return !(b.min[0] <= b.max[0]);
}

export function unionBounds(a: Bounds | null, b: Bounds): Bounds {
  if (!a) return { min: [...b.min], max: [...b.max] };
  return {
    min: [Math.min(a.min[0], b.min[0]), Math.min(a.min[1], b.min[1]), Math.min(a.min[2], b.min[2])],
    max: [Math.max(a.max[0], b.max[0]), Math.max(a.max[1], b.max[1]), Math.max(a.max[2], b.max[2])],
  };
}

export function scaleBounds(b: Bounds, s: number): Bounds {
  return {
    min: [b.min[0] * s, b.min[1] * s, b.min[2] * s],
    max: [b.max[0] * s, b.max[1] * s, b.max[2] * s],
  };
}

export function boundsCenter(b: Bounds): Vec3 {
  return [(b.min[0] + b.max[0]) / 2, (b.min[1] + b.max[1]) / 2, (b.min[2] + b.max[2]) / 2];
}

export function boundsSize(b: Bounds): Vec3 {
  return [b.max[0] - b.min[0], b.max[1] - b.min[1], b.max[2] - b.min[2]];
}
