import { computeBounds, FLOATS_PER_VERTEX, type Bounds, type MeshData, type Vec3 } from "./mesh";

/** Mesh issues of one part (src-tauri/src/geometry.rs `Issues`). */
export interface MeshIssues {
  duplicateVertices: number;
  degenerateTriangles: number;
  duplicateTriangles: number;
  holes: number;
  openEdges: number;
  nonManifoldEdges: number;
  /** Inconsistent winding, inward shells and orientation conflicts. */
  flipped: number;
  closed: boolean;
}

export interface PartReport {
  before: MeshIssues;
  /** After the safe repair, when it ran. */
  after: MeshIssues | null;
  removedTriangles: number;
  flippedTriangles: number;
}

export interface GeometryPart {
  index: number;
  name: string;
  /** GEM electrode value (2 for e(2)); null for meshes. */
  sourceValue: number | null;
  triangles: number;
  bounds: [Vec3, Vec3] | null;
  report: PartReport | null;
  /** Can be sampled as a solid. */
  closed: boolean;
  /** CAD only: valid B-rep (the exact surface can be used); null for GEM and meshes. */
  exact: boolean | null;
}

export interface GeometryInfo {
  kind: "gem" | "mesh" | "cad";
  path: string;
  fileName: string;
  units: string;
  defaultUnits: string;
  parts: GeometryPart[];
  bounds: [Vec3, Vec3] | null;
  gem: {
    dims: [number, number, number];
    spacing: [number, number, number];
    symmetry: "planar" | "cylindrical";
    mirrors: [boolean, boolean, boolean];
    refinable: boolean;
    electrostatic: boolean;
  } | null;
  suggestion: {
    spacing: number;
    dims: [number, number, number];
    origin: Vec3;
    featureMm: number | null;
    gapMm: number | null;
    limitedBy: "pa_define" | "feature" | "gap" | "budget";
  };
  previewTriangles: number;
  previewStep: number;
  ms: number;
}

/** See the SGP1 layout in src-tauri/src/geometry.rs. Part meshes are views into `buf`. */
export function parseGeometryPacket(buf: ArrayBuffer): { info: GeometryInfo; meshes: MeshData[] } {
  const bytes = new Uint8Array(buf);
  if (String.fromCharCode(...bytes.subarray(0, 4)) !== "SGP1") throw new Error("the native core answered with an unknown geometry packet");
  const headerLen = new DataView(buf).getUint32(4, true);
  const info = JSON.parse(new TextDecoder().decode(bytes.subarray(8, 8 + headerLen))) as GeometryInfo;
  let at = 8 + headerLen;
  const meshes = info.parts.map((p) => {
    const floats = p.triangles * 3 * FLOATS_PER_VERTEX;
    const vertices = new Float32Array(buf, at, floats);
    at += floats * 4;
    const bounds: Bounds = p.bounds ? { min: [...p.bounds[0]], max: [...p.bounds[1]] } : computeBounds(vertices);
    return { vertices, triangleCount: p.triangles, bounds };
  });
  return { info, meshes };
}

/** Rough RAM while refining and flying, and disk for .pa# + .pa0 + one .paN per electrode. */
export function gridCost(points: number, electrodes: number): { ramBytes: number; diskBytes: number } {
  // PA (f64 + flag), one f64 basis per electrode + the base, multigrid work (~3 f64), flight field (~1 f64 per basis).
  const ramBytes = points * (9 + 8 * (electrodes + 1) + 24 + 8 * (electrodes + 1));
  const diskBytes = points * 8 * (electrodes + 2);
  return { ramBytes, diskBytes };
}
