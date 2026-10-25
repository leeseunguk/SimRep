import { computeBounds, FLOATS_PER_VERTEX, type MeshData } from "./mesh";

export type StlErrorCode = "empty" | "truncated" | "noFacets" | "badNumber";

export class StlError extends Error {
  constructor(
    readonly code: StlErrorCode,
    readonly params: Record<string, number> = {},
  ) {
    super(`STL ${code}`);
  }
}

export interface StlResult extends MeshData {
  format: "binary" | "ascii";
}

const HEADER_BYTES = 80;
const BINARY_TRIANGLE_BYTES = 50;

/**
 * Parses ASCII or binary STL into a flat-shaded triangle soup.
 * Face normals are recomputed from the vertices because exporters often write zero or stale normals.
 */
export function parseStl(buffer: ArrayBuffer): StlResult {
  if (buffer.byteLength === 0) throw new StlError("empty");
  const view = new DataView(buffer);

  if (buffer.byteLength >= HEADER_BYTES + 4) {
    const declared = view.getUint32(HEADER_BYTES, true);
    const expectedSize = HEADER_BYTES + 4 + declared * BINARY_TRIANGLE_BYTES;
    // Size match is the reliable binary signal: some binary exporters start the header with "solid".
    if (declared > 0 && buffer.byteLength >= expectedSize) return parseBinary(view, declared);
    if (!looksLikeAscii(buffer)) {
      if (declared === 0) throw new StlError("noFacets");
      const found = Math.floor((buffer.byteLength - HEADER_BYTES - 4) / BINARY_TRIANGLE_BYTES);
      throw new StlError("truncated", { expected: declared, found });
    }
  }
  return parseAscii(buffer);
}

function looksLikeAscii(buffer: ArrayBuffer): boolean {
  const head = new TextDecoder().decode(new Uint8Array(buffer, 0, Math.min(buffer.byteLength, 1024)));
  return /^\s*solid/i.test(head) && /facet|endsolid/i.test(head);
}

function parseBinary(view: DataView, count: number): StlResult {
  const out = new Float32Array(count * 3 * FLOATS_PER_VERTEX);
  let offset = HEADER_BYTES + 4;
  let o = 0;
  for (let t = 0; t < count; t++) {
    const p = offset + 12; // skip the stored normal
    const ax = view.getFloat32(p, true), ay = view.getFloat32(p + 4, true), az = view.getFloat32(p + 8, true);
    const bx = view.getFloat32(p + 12, true), by = view.getFloat32(p + 16, true), bz = view.getFloat32(p + 20, true);
    const cx = view.getFloat32(p + 24, true), cy = view.getFloat32(p + 28, true), cz = view.getFloat32(p + 32, true);
    o = writeTriangle(out, o, ax, ay, az, bx, by, bz, cx, cy, cz);
    offset += BINARY_TRIANGLE_BYTES;
  }
  return { vertices: out, triangleCount: count, bounds: computeBounds(out), format: "binary" };
}

function parseAscii(buffer: ArrayBuffer): StlResult {
  const text = new TextDecoder().decode(buffer);
  const re = /vertex\s+(\S+)\s+(\S+)\s+(\S+)/g;
  const coords: number[] = [];
  let m: RegExpExecArray | null;
  while ((m = re.exec(text)) !== null) {
    for (let i = 1; i <= 3; i++) {
      const v = Number(m[i]);
      if (!Number.isFinite(v)) {
        const line = text.slice(0, m.index).split("\n").length;
        throw new StlError("badNumber", { line });
      }
      coords.push(v);
    }
  }
  const triangleCount = Math.floor(coords.length / 9);
  if (triangleCount === 0) throw new StlError("noFacets");
  const out = new Float32Array(triangleCount * 3 * FLOATS_PER_VERTEX);
  let o = 0;
  for (let t = 0; t < triangleCount; t++) {
    const c = t * 9;
    o = writeTriangle(
      out, o,
      coords[c]!, coords[c + 1]!, coords[c + 2]!,
      coords[c + 3]!, coords[c + 4]!, coords[c + 5]!,
      coords[c + 6]!, coords[c + 7]!, coords[c + 8]!,
    );
  }
  return { vertices: out, triangleCount, bounds: computeBounds(out), format: "ascii" };
}

function writeTriangle(
  out: Float32Array, o: number,
  ax: number, ay: number, az: number,
  bx: number, by: number, bz: number,
  cx: number, cy: number, cz: number,
): number {
  const ux = bx - ax, uy = by - ay, uz = bz - az;
  const vx = cx - ax, vy = cy - ay, vz = cz - az;
  let nx = uy * vz - uz * vy;
  let ny = uz * vx - ux * vz;
  let nz = ux * vy - uy * vx;
  const len = Math.hypot(nx, ny, nz);
  if (len > 0) {
    nx /= len;
    ny /= len;
    nz /= len;
  } else {
    // Degenerate sliver: zero normal; the shader falls back to view-facing shading.
    nx = ny = nz = 0;
  }
  for (const [x, y, z] of [[ax, ay, az], [bx, by, bz], [cx, cy, cz]] as const) {
    out[o++] = x;
    out[o++] = y;
    out[o++] = z;
    out[o++] = nx;
    out[o++] = ny;
    out[o++] = nz;
  }
  return o;
}
