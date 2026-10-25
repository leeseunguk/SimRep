import { dims, extrusionDepth, paKind, spacing, type PaGrid, type PaInfo } from "./pa";

/**
 * Electrode surface of a potential array as instanced quads: 4 corners (xyz, mm, stored
 * quadrant only) + the electrode potential + its electrode label (scene/electrodes.ts) = 14
 * floats per quad. Only faces between electrode and
 * non-electrode points are emitted, and runs of equal faces are merged, so a solid block costs
 * its surface, not its volume. Mirrored copies are drawn by the GPU with a reflection.
 *
 * - 3D: one box per grid point, spanning half a cell either side (clipped to the array).
 * - Planar 2D: the boxes of the x-y plane extruded +-extrusionDepth in Z.
 * - Cylindrical: the boundary edges of the x-r plane revolved about X.
 */
export const QUAD_FLOATS = 14;
export const REVOLVE_SEGMENTS = 96;

export interface PaSurface {
  quads: Float32Array<ArrayBuffer>;
  count: number;
  /** 1 = full resolution; n = every n-th grid point (kept under the quad budget). */
  step: number;
}

class Overflow extends Error {}

class Sink {
  buf = new Float32Array(QUAD_FLOATS * 4096);
  count = 0;
  constructor(readonly max: number) {}

  push(c: Float64Array, potential: number, label: number) {
    if (this.count >= this.max) throw new Overflow();
    let o = this.count * QUAD_FLOATS;
    if (o + QUAD_FLOATS > this.buf.length) {
      const grown = new Float32Array(Math.min(this.buf.length * 2, this.max * QUAD_FLOATS));
      grown.set(this.buf);
      this.buf = grown;
    }
    this.buf.set(c, o);
    o += 12;
    this.buf[o] = potential;
    this.buf[o + 1] = label;
    this.count++;
  }
}

export function buildSurface(info: PaInfo, grid: PaGrid, labels: Uint16Array | null, maxQuads: number): PaSurface {
  for (let step = 1; ; step *= 2) {
    const sink = new Sink(maxQuads);
    try {
      if (paKind(info) === "cylindrical") revolve(info, grid, labels, step, sink);
      else boxes(info, grid, labels, step, sink);
      return { quads: sink.buf.slice(0, sink.count * QUAD_FLOATS), count: sink.count, step };
    } catch (e) {
      if (!(e instanceof Overflow)) throw e;
    }
  }
}

/** Electrode mask, potentials and labels sampled every `step` points (x-y only for 2D arrays). */
function coarse(info: PaInfo, grid: PaGrid, labels: Uint16Array | null, step: number) {
  const n = dims(info);
  const c: [number, number, number] = [Math.ceil(n[0] / step), Math.ceil(n[1] / step), Math.ceil(n[2] / step)];
  if (step === 1) return { c, e: grid.electrodes, p: grid.potentials, l: labels };
  const count = c[0] * c[1] * c[2];
  const e = new Uint8Array(count);
  const p = new Float32Array(count);
  const l = labels ? new Uint16Array(count) : null;
  let o = 0;
  for (let k = 0; k < c[2]; k++)
    for (let j = 0; j < c[1]; j++) {
      const row = (k * step * n[1] + j * step) * n[0];
      for (let i = 0; i < c[0]; i++, o++) {
        e[o] = grid.electrodes[row + i * step]!;
        p[o] = grid.potentials[row + i * step]!;
        if (l) l[o] = labels![row + i * step]!;
      }
    }
  return { c, e, p, l };
}

/** Cell extents per axis: lo[a][i]..hi[a][i] in mm, half a (coarse) cell either side, clipped. */
function cellEdges(info: PaInfo, c: [number, number, number], step: number) {
  const n = dims(info);
  const d = spacing(info);
  const planar2d = paKind(info) === "planar2d";
  const depth = extrusionDepth(info);
  const lo: Float64Array[] = [];
  const hi: Float64Array[] = [];
  for (let a = 0; a < 3; a++) {
    lo.push(new Float64Array(c[a]!));
    hi.push(new Float64Array(c[a]!));
    for (let i = 0; i < c[a]!; i++) {
      if (planar2d && a === 2) {
        lo[a]![i] = -depth;
        hi[a]![i] = depth;
      } else {
        lo[a]![i] = Math.max(0, i * step - step / 2) * d[a]!;
        hi[a]![i] = Math.min(n[a]! - 1, i * step + step / 2) * d[a]!;
      }
    }
  }
  return { lo, hi };
}

/**
 * Walks every face direction (axis a, sign) with the run axis innermost and calls `emit` for
 * each maximal run of boundary faces with the same potential and electrode label.
 */
function forEachFaceRun(
  c: [number, number, number],
  e: Uint8Array,
  p: Float32Array,
  l: Uint16Array | null,
  axes: readonly number[],
  runAxisOf: (a: number) => number,
  emit: (a: number, sign: number, ia: number, it: number, r0: number, r1: number, potential: number, label: number) => void,
) {
  const stride = [1, c[0], c[0] * c[1]];
  for (const a of axes) {
    const r = runAxisOf(a);
    const t = 3 - a - r;
    for (const sign of [-1, 1]) {
      for (let it = 0; it < c[t]!; it++) {
        for (let ia = 0; ia < c[a]!; ia++) {
          const nb = ia + sign;
          const edge = nb < 0 || nb >= c[a]!;
          let start = -1;
          let pot = 0;
          let lab = 0;
          for (let ir = 0; ir <= c[r]!; ir++) {
            let face = false;
            let v = 0;
            let w = 0;
            if (ir < c[r]!) {
              const idx = ia * stride[a]! + ir * stride[r]! + it * stride[t]!;
              face = e[idx] === 1 && (edge || e[idx + sign * stride[a]!] !== 1);
              v = p[idx]!;
              w = l ? l[idx]! : 0;
            }
            if (start >= 0 && (!face || v !== pot || w !== lab)) {
              emit(a, sign, ia, it, start, ir - 1, pot, lab);
              start = -1;
            }
            if (face && start < 0) {
              start = ir;
              pot = v;
              lab = w;
            }
          }
        }
      }
    }
  }
}

function boxes(info: PaInfo, grid: PaGrid, labels: Uint16Array | null, step: number, sink: Sink) {
  const { c, e, p, l } = coarse(info, grid, labels, step);
  const { lo, hi } = cellEdges(info, c, step);
  const q = new Float64Array(12);
  forEachFaceRun(c, e, p, l, [0, 1, 2], (a) => (a === 0 ? 1 : 0), (a, sign, ia, it, s, end, potential, label) => {
    const r = a === 0 ? 1 : 0;
    const t = 3 - a - r;
    const f = sign > 0 ? hi[a]![ia]! : lo[a]![ia]!;
    const r0 = lo[r]![s]!, r1 = hi[r]![end]!;
    const t0 = lo[t]![it]!, t1 = hi[t]![it]!;
    if (r1 <= r0 || t1 <= t0) return;
    const corners = [
      [r0, t0],
      [r1, t0],
      [r1, t1],
      [r0, t1],
    ];
    corners.forEach(([rv, tv], k) => {
      q[k * 3 + a] = f;
      q[k * 3 + r] = rv!;
      q[k * 3 + t] = tv!;
    });
    sink.push(q, potential, label);
  });
}

function revolve(info: PaInfo, grid: PaGrid, labels: Uint16Array | null, step: number, sink: Sink) {
  const { c, e, p, l } = coarse(info, grid, labels, step);
  const { lo, hi } = cellEdges(info, c, step);
  const cos = new Float64Array(REVOLVE_SEGMENTS + 1);
  const sin = new Float64Array(REVOLVE_SEGMENTS + 1);
  for (let k = 0; k <= REVOLVE_SEGMENTS; k++) {
    cos[k] = Math.cos((2 * Math.PI * k) / REVOLVE_SEGMENTS);
    sin[k] = Math.sin((2 * Math.PI * k) / REVOLVE_SEGMENTS);
  }
  const q = new Float64Array(12);
  // x faces (annuli) run along r; r faces (cylinder bands) run along x. z is the unused third axis.
  forEachFaceRun(c, e, p, l, [0, 1], (a) => (a === 0 ? 1 : 0), (a, sign, ia, _it, s, end, potential, label) => {
    let xa: number, ra: number, xb: number, rb: number;
    if (a === 0) {
      xa = xb = sign > 0 ? hi[0]![ia]! : lo[0]![ia]!;
      ra = lo[1]![s]!;
      rb = hi[1]![end]!;
    } else {
      ra = rb = sign > 0 ? hi[1]![ia]! : lo[1]![ia]!;
      if (ra <= 0) return; // on the axis
      xa = lo[0]![s]!;
      xb = hi[0]![end]!;
    }
    if (xa === xb && ra === rb) return;
    for (let k = 0; k < REVOLVE_SEGMENTS; k++) {
      q.set([xa, ra * cos[k]!, ra * sin[k]!, xb, rb * cos[k]!, rb * sin[k]!, xb, rb * cos[k + 1]!, rb * sin[k + 1]!, xa, ra * cos[k + 1]!, ra * sin[k + 1]!]);
      sink.push(q, potential, label);
    }
  });
}
