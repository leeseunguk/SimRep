import type { Bounds } from "./mesh";

/**
 * SIMION potential arrays (.pa, .pa#, .pa0) as the viewer sees them.
 *
 * Grid point (i, j, k) sits at (i*dx, j*dy, k*dz) mm, x fastest. Mirrored axes are reflected
 * through 0 for display. Cylindrical arrays are 2D: x is the axis, y the radius; they are shown
 * revolved about X with the x-r plane at z = 0. Planar 2D arrays (nz = 1) extend infinitely in Z;
 * they are shown extruded a finite depth around z = 0.
 */

export type PaFormatId = "pa" | "paHash" | "pa0";
export type Axis = 0 | 1 | 2;
export type PaKind = "3d" | "planar2d" | "cylindrical";

export interface Flags3 {
  x: boolean;
  y: boolean;
  z: boolean;
}

/** Header + statistics. Produced by `read_pa` in src-tauri/src/pa.rs (and `paDecode.ts` in the browser). */
export interface PaInfo {
  /** -1: SIMION 7/8.0 header (1 mm cells), -2: SIMION 8.1+ header with cell sizes. */
  mode: -1 | -2;
  symmetry: "planar" | "cylindrical";
  nx: number;
  ny: number;
  nz: number;
  dx: number;
  dy: number;
  dz: number;
  mirror: Flags3;
  antiMirror: Flags3;
  field: "electrostatic" | "magnetic";
  refinable: boolean;
  maxVoltage: number;
  ng: number;
  points: number;
  electrodePoints: number;
  potentialMin: number;
  potentialMax: number;
  electrodeMin: number | null;
  electrodeMax: number | null;
  /** Sorted distinct electrode potentials (first 64). */
  electrodeValues: number[];
  /** Exact up to 1001. */
  distinctElectrodeValues: number;
  pa0: { adjustable: number; electrodeNumbers: number[]; referenceVoltage: number; fastScalable: boolean } | null;
  /** Desktop only (packets from the native core): the file and its live fast-adjust state. */
  path?: string;
  fileName?: string;
  fileSize?: number;
  fast?: FastStatus | null;
  /** Sibling `<stem>.cuts.json` (exact electrode surfaces for flights), file name; null without one. */
  cuts?: string | null;
}

/** Fast adjustment of a .pa0: ready with the current electrode voltages, or why it is unavailable. */
export type FastStatus = { state: "ready"; electrodes: { number: number; voltage: number }[] } | { state: "missing"; detail: string };

export interface PaGrid {
  potentials: Float32Array<ArrayBuffer>;
  /** 1 = electrode point. */
  electrodes: Uint8Array<ArrayBuffer>;
}

export type PaErrorCode = "empty" | "notPa" | "truncated" | "badHeader" | "badPa0" | "tooLarge" | "notFound" | "unexpected";

export class PaReadError extends Error {
  constructor(
    readonly code: PaErrorCode,
    readonly detail: string,
  ) {
    super(`${code}: ${detail}`);
  }
}

/** Largest array the viewer accepts (keep in sync with MAX_POINTS in src-tauri/src/pa.rs). */
export const PA_MAX_POINTS = 40_000_000;

// ------------------------------------------------------------------------------------ packet

const MAGIC = "SPA1";

/** Header point count from the first 28 bytes, or null when they are not a PA header. */
export function peekPointCount(head: ArrayBuffer): number | null {
  if (head.byteLength < 28) return null;
  const v = new DataView(head);
  const mode = v.getInt32(0, true);
  if (mode !== -1 && mode !== -2) return null;
  return Math.max(0, v.getInt32(16, true)) * Math.max(0, v.getInt32(20, true)) * Math.max(0, v.getInt32(24, true));
}

/** See the layout in src-tauri/src/pa.rs. Views share `buf`, nothing is copied. */
export function parsePacket(buf: ArrayBuffer): { info: PaInfo; grid: PaGrid } {
  const bytes = new Uint8Array(buf);
  const magic = String.fromCharCode(...bytes.subarray(0, 4));
  if (magic !== MAGIC) throw new PaReadError("unexpected", "the native core answered with an unknown packet");
  const headerLen = new DataView(buf).getUint32(4, true);
  const info = JSON.parse(new TextDecoder().decode(bytes.subarray(8, 8 + headerLen))) as PaInfo;
  const n = info.points;
  const potOffset = 8 + headerLen;
  if (buf.byteLength < potOffset + n * 5) throw new PaReadError("unexpected", "the native core answered with a short packet");
  return {
    info,
    grid: {
      potentials: new Float32Array(buf, potOffset, n),
      electrodes: new Uint8Array(buf, potOffset + n * 4, n),
    },
  };
}

/**
 * Live fast-adjust packet (`potentials_packet` in src-tauri/src/pa.rs): the header and u16
 * quantized potentials, without the electrode flags, which a voltage change never alters: the
 * caller passes the array's current ones.
 */
export function parsePotentialsPacket(buf: ArrayBuffer, electrodes: Uint8Array<ArrayBuffer>): { info: PaInfo; grid: PaGrid } {
  const bytes = new Uint8Array(buf);
  if (String.fromCharCode(...bytes.subarray(0, 4)) !== "SPQ1") throw new PaReadError("unexpected", "the native core answered with an unknown packet");
  const view = new DataView(buf);
  const headerLen = view.getUint32(4, true);
  const info = JSON.parse(new TextDecoder().decode(bytes.subarray(8, 8 + headerLen))) as PaInfo;
  const n = info.points;
  const at = 8 + headerLen;
  if (buf.byteLength < at + 16 + n * 2) throw new PaReadError("unexpected", "the native core answered with a short packet");
  if (electrodes.length !== n) throw new PaReadError("unexpected", "the fast-adjusted array does not match the open one");
  const offset = view.getFloat64(at, true);
  const step = view.getFloat64(at + 8, true);
  const codes = new Uint16Array(buf, at + 16, n);
  const potentials = new Float32Array(n);
  for (let i = 0; i < n; i++) potentials[i] = offset + codes[i]! * step;
  return { info, grid: { potentials, electrodes } };
}

export function encodePacket(info: PaInfo, potentials: Float32Array, electrodes: Uint8Array): ArrayBuffer {
  const json = new TextEncoder().encode(JSON.stringify(info));
  const padded = Math.ceil(json.length / 4) * 4;
  const n = info.points;
  const buf = new ArrayBuffer(8 + padded + n * 5);
  const bytes = new Uint8Array(buf);
  bytes.set([...MAGIC].map((c) => c.charCodeAt(0)), 0);
  new DataView(buf).setUint32(4, padded, true);
  bytes.fill(32, 8, 8 + padded);
  bytes.set(json, 8);
  new Float32Array(buf, 8 + padded, n).set(potentials);
  bytes.set(electrodes, 8 + padded + n * 4);
  return buf;
}

// ---------------------------------------------------------------------------------- geometry

export function paKind(info: PaInfo): PaKind {
  if (info.symmetry === "cylindrical") return "cylindrical";
  return info.nz === 1 ? "planar2d" : "3d";
}

export const dims = (info: PaInfo): [number, number, number] => [info.nx, info.ny, info.nz];
export const spacing = (info: PaInfo): [number, number, number] => [info.dx, info.dy, info.dz];

/** Axes reflected through 0 for display. The radial revolution of cylindrical arrays is separate. */
export function displayMirrors(info: PaInfo, showMirrors: boolean): Flags3 {
  if (!showMirrors) return { x: false, y: false, z: false };
  const kind = paKind(info);
  return {
    x: info.mirror.x,
    y: kind !== "cylindrical" && info.mirror.y,
    z: kind === "3d" && info.mirror.z,
  };
}

/** Half-depth of the extrusion used to show a planar 2D array (it is infinite in Z). */
export function extrusionDepth(info: PaInfo): number {
  return 0.3 * Math.max((info.nx - 1) * info.dx, (info.ny - 1) * info.dy, 1e-3);
}

/** Displayed extent along one axis (mm). */
export function axisExtent(info: PaInfo, axis: Axis, showMirrors: boolean): [number, number] {
  const kind = paKind(info);
  if (kind === "cylindrical" && axis !== 0) {
    const r = (info.ny - 1) * info.dy;
    return [-r, r];
  }
  if (kind === "planar2d" && axis === 2) {
    const d = extrusionDepth(info);
    return [-d, d];
  }
  const hi = (dims(info)[axis] - 1) * spacing(info)[axis];
  const m = displayMirrors(info, showMirrors);
  return [(axis === 0 ? m.x : axis === 1 ? m.y : m.z) ? -hi : 0, hi];
}

export function displayBounds(info: PaInfo, showMirrors: boolean): Bounds {
  const x = axisExtent(info, 0, showMirrors);
  const y = axisExtent(info, 1, showMirrors);
  const z = axisExtent(info, 2, showMirrors);
  return { min: [x[0], y[0], z[0]], max: [x[1], y[1], z[1]] };
}

// ------------------------------------------------------------------------------------ slice

export type ContourDensity = "few" | "normal" | "many";
export const CONTOUR_LINES: Record<ContourDensity, number> = { few: 10, normal: 20, many: 40 };

export interface SliceState {
  visible: boolean;
  /** Normal of the slice plane. 2D and cylindrical arrays always use Z (their own x-y plane). */
  axis: Axis;
  /** Signed grid index along `axis` (negative only on a displayed mirror). */
  coord: number;
  contours: boolean;
  density: ContourDensity;
  /** Hide geometry between the camera and the slice. */
  cut: boolean;
}

export function sliceAxes(info: PaInfo): Axis[] {
  return paKind(info) === "3d" ? [0, 1, 2] : [2];
}

/** In-plane axes (u, v) for a slice normal. */
export function planeAxes(axis: Axis): [Axis, Axis] {
  return axis === 0 ? [1, 2] : axis === 1 ? [0, 2] : [0, 1];
}

export function coordRange(info: PaInfo, axis: Axis, showMirrors: boolean): [number, number] {
  if (paKind(info) !== "3d") return [0, 0];
  const n = dims(info)[axis] - 1;
  const m = displayMirrors(info, showMirrors);
  return [(axis === 0 ? m.x : axis === 1 ? m.y : m.z) ? -n : 0, n];
}

export function clampCoord(info: PaInfo, axis: Axis, coord: number, showMirrors: boolean): number {
  const [lo, hi] = coordRange(info, axis, showMirrors);
  return Math.min(hi, Math.max(lo, Math.round(coord)));
}

export function defaultSlice(info: PaInfo): SliceState {
  const axis: Axis = 2;
  const [lo, hi] = coordRange(info, axis, true);
  return { visible: true, axis, coord: lo < 0 ? 0 : Math.round((lo + hi) / 2), contours: true, density: "normal", cut: true };
}

export interface SlicePlane {
  axis: Axis;
  /** World position of the plane along `axis` (mm). */
  position: number;
  /** Grid index of the stored plane that is shown. */
  index: number;
  /** -1 when the plane lies on the anti-mirrored side of `axis`. */
  normalSign: number;
  u: Axis;
  v: Axis;
  uRange: [number, number];
  vRange: [number, number];
  /** Mirror (and sign flip) applied when sampling at negative u / v. */
  mirrorU: boolean;
  mirrorV: boolean;
  antiU: boolean;
  antiV: boolean;
}

export function slicePlane(info: PaInfo, slice: SliceState, showMirrors: boolean): SlicePlane {
  const kind = paKind(info);
  const axis = kind === "3d" ? slice.axis : 2;
  const coord = kind === "3d" ? clampCoord(info, axis, slice.coord, showMirrors) : 0;
  const [u, v] = planeAxes(axis);
  const m = displayMirrors(info, showMirrors);
  const mirrored = (a: Axis) => (a === 0 ? m.x : a === 1 ? m.y : m.z) || (kind === "cylindrical" && a === 1);
  const anti = (a: Axis) => (a === 0 ? info.antiMirror.x : a === 1 ? info.antiMirror.y : info.antiMirror.z) && kind !== "cylindrical";
  return {
    axis,
    position: kind === "3d" ? coord * spacing(info)[axis] : 0,
    index: Math.abs(coord),
    normalSign: coord < 0 && anti(axis) ? -1 : 1,
    u,
    v,
    uRange: axisExtent(info, u, showMirrors),
    vRange: axisExtent(info, v, showMirrors),
    mirrorU: mirrored(u),
    mirrorV: mirrored(v),
    antiU: anti(u),
    antiV: anti(v),
  };
}

/** One stored grid plane as an rg32float image: r = potential (V), g = 1 on electrode points (2 on the selected electrode, 0 on hidden ones). */
export interface SliceImage {
  data: Float32Array<ArrayBuffer>;
  width: number;
  height: number;
  /** mm between neighbouring texels. */
  du: number;
  dv: number;
}

/** Electrode labels with the selected / hidden ones (scene/electrodes.ts). */
export interface SliceMarks {
  labels: Uint16Array;
  selected: number | null;
  hidden: readonly number[];
}

export function extractSlice(info: PaInfo, grid: PaGrid, plane: SlicePlane, maxDim: number, marks: SliceMarks | null = null): SliceImage {
  const n = dims(info);
  const d = spacing(info);
  const nu = n[plane.u];
  const nv = n[plane.v];
  const su = Math.ceil(nu / maxDim);
  const sv = Math.ceil(nv / maxDim);
  const width = Math.ceil(nu / su);
  const height = Math.ceil(nv / sv);
  const stride: [number, number, number] = [1, info.nx, info.nx * info.ny];
  const base = plane.index * stride[plane.axis];
  const data = new Float32Array(width * height * 2);
  const { potentials, electrodes } = grid;
  const hidden = new Set(marks?.hidden ?? []);
  const mark = (idx: number) => {
    if (!electrodes[idx]) return 0;
    if (!marks) return 1;
    const label = marks.labels[idx]!;
    return hidden.has(label) ? 0 : label === marks.selected ? 2 : 1;
  };
  for (let j = 0; j < height; j++) {
    const row = base + j * sv * stride[plane.v];
    for (let i = 0; i < width; i++) {
      const idx = row + i * su * stride[plane.u];
      const o = (j * width + i) * 2;
      data[o] = potentials[idx]! * plane.normalSign;
      data[o + 1] = mark(idx);
    }
  }
  return { data, width, height, du: d[plane.u] * su, dv: d[plane.v] * sv };
}

/** Bilinear potential + nearest electrode flag at in-plane (u, v) mm; matches the slice shader. */
export function sampleSlice(img: SliceImage, plane: SlicePlane, u: number, v: number): { value: number; electrode: boolean } {
  let sign = 1;
  if (plane.mirrorU && u < 0) {
    u = -u;
    if (plane.antiU) sign = -sign;
  }
  if (plane.mirrorV && v < 0) {
    v = -v;
    if (plane.antiV) sign = -sign;
  }
  const gu = Math.min(Math.max(u / img.du, 0), img.width - 1);
  const gv = Math.min(Math.max(v / img.dv, 0), img.height - 1);
  const i0 = Math.floor(gu);
  const j0 = Math.floor(gv);
  const i1 = Math.min(i0 + 1, img.width - 1);
  const j1 = Math.min(j0 + 1, img.height - 1);
  const fu = gu - i0;
  const fv = gv - j0;
  const at = (i: number, j: number) => img.data[(j * img.width + i) * 2]!;
  const value =
    (at(i0, j0) * (1 - fu) + at(i1, j0) * fu) * (1 - fv) + (at(i0, j1) * (1 - fu) + at(i1, j1) * fu) * fv;
  const electrode = img.data[(Math.round(gv) * img.width + Math.round(gu)) * 2 + 1]! > 0.5;
  return { value: value * sign, electrode };
}

/** 1, 2 or 5 x 10^k closest to range / lines; 0 when the array has a single potential. */
export function contourStep(min: number, max: number, density: ContourDensity): number {
  if (!(max > min)) return 0;
  const raw = (max - min) / CONTOUR_LINES[density];
  const p = 10 ** Math.floor(Math.log10(raw));
  const m = raw / p;
  return (m < 1.5 ? 1 : m < 3.5 ? 2 : m < 7.5 ? 5 : 10) * p;
}
