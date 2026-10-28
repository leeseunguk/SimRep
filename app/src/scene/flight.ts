import type { Vec3 } from "./mesh";
import { displayBounds, paKind, type PaInfo } from "./pa";
import type { BeamShape, DeviceInfo, FlyCollision, FlyPrecision, Reason } from "./backend";
import type { Placement } from "./placement";

/** Ion source of one potential array (SIMION units: amu, e, eV, mm, degrees). */
export interface IonSource {
  mode: "beam" | "file";
  count: number;
  massAmu: number;
  chargeE: number;
  energyEv: number;
  position: Vec3;
  /** SIMION convention: azimuth about +Y from +X (+90 points to -Z), elevation up from the XZ plane. */
  azimuthDeg: number;
  elevationDeg: number;
  coneDeg: number;
  shape: BeamShape;
  /** Line length / disk diameter, mm. */
  widthMm: number;
  /** .ion / .fly2 particle file (desktop path) when mode = "file". */
  file: { path: string; name: string } | null;
  /**
   * Field-free workbench box around the array (SIMION workbench bounds); off = stop at the array
   * edge. `placement`: where the array sits on the workbench (.iob instance, imported model origin).
   */
  bench: { enabled: boolean; min: Vec3; max: Vec3; placement?: Placement | null; from?: string | null };
  durationUs: number;
  /** Live preview on the GPU, or every ion on the CPU in f64. */
  precision: FlyPrecision;
  /** When an ion hits an electrode (absent in older projects: SIMION's all-corner rule). */
  collision?: FlyCollision;
  /** Trajectories drawn (evenly spread over all ions); results always count every ion. */
  drawCount: number;
  /** Splat on the array's .cuts.json surfaces when it has one (absent in older projects: on). */
  exactSurfaces?: boolean;
}

/** Most trajectories drawn (src-tauri/src/fly.rs MAX_DRAWN) and most ions flown (MAX_IONS). */
export const MAX_DRAWN = 10_000;
export const MAX_IONS = 1_000_000;
export const DEFAULT_DRAWN = 300;

export function directionOf(s: Pick<IonSource, "azimuthDeg" | "elevationDeg">): Vec3 {
  const az = (s.azimuthDeg * Math.PI) / 180;
  const el = (s.elevationDeg * Math.PI) / 180;
  return [Math.cos(el) * Math.cos(az), Math.sin(el), -Math.cos(el) * Math.sin(az)];
}

const round = (v: number) => Number(v.toPrecision(4));

/**
 * A beam that shows something on any array: along +X from the array's -X edge, a line across Y
 * spanning 3/4 of the radius (cylindrical) or of the Y extent, 100 amu +1 e at 100 eV.
 */
export function defaultSource(info: PaInfo): IonSource {
  const b = displayBounds(info, true);
  const cylindrical = paKind(info) === "cylindrical";
  const yMid = cylindrical ? 0 : (b.min[1] + b.max[1]) / 2;
  // 3D arrays: start mid-depth (z = 0 is the array's -z wall).
  const zMid = info.nz > 1 ? (b.min[2] + b.max[2]) / 2 : 0;
  const span = cylindrical ? b.max[1] : b.max[1] - b.min[1];
  return {
    mode: "beam",
    count: 200,
    massAmu: 100,
    chargeE: 1,
    energyEv: 100,
    position: [round(b.min[0]), round(yMid), round(zMid)],
    azimuthDeg: 0,
    elevationDeg: 0,
    coneDeg: 0,
    shape: "line",
    widthMm: round(span * 0.75 * (cylindrical ? 2 : 1)),
    file: null,
    bench: { enabled: false, min: b.min.map(round) as Vec3, max: b.max.map(round) as Vec3 },
    durationUs: 100,
    precision: "live",
    collision: "allCorners",
    drawCount: DEFAULT_DRAWN,
    exactSurfaces: true,
  };
}

/** Where ions ended (`Surface` in src-tauri/src/fly.rs). */
export type Surface =
  | { kind: "electrode"; number: number | null; voltage: number | null }
  | { kind: "wall"; face: "-x" | "+x" | "-y" | "+y" | "-z" | "+z" | "r" }
  | { kind: "timeLimit" }
  | { kind: "stalled" };

export interface FlightHeader {
  ions: number;
  /** Ions with trajectory points (the others contribute end points and statistics only). */
  drawn: number;
  points: number;
  surfaces: { surface: Surface; count: number; tofMean: number }[];
  tof: { n: number; min: number; max: number; mean: number; std: number };
  keMean: number;
  /** Device that flew every ion. */
  device: DeviceInfo;
  /** Requested mode; live requests can fall back to CPU f64. */
  precision: FlyPrecision;
  /** Actual end-state execution, separate from the requested mode. */
  executed: { device: "cpu" | "gpu"; precision: "f64" | "f32"; surfaces: "exact" | "grid" };
  /** Why a live flight ran on the CPU (no GPU, still starting, field too large, exact surfaces). */
  fallback: Reason | null;
  /**
   * Exact electrode surfaces the ions splatted on (null: the grid's electrode points). `source`:
   * v3 source geometry (exact); `sampled` / `approximate`: older v2 / v1 sidecars.
   */
  surface: { file: string; accuracy: "source" | "sampled" | "approximate" } | null;
  fieldMs: number;
  /** Background field build + upload to the GPU, reported by its first GPU flight (0 afterwards). */
  prepareMs: number;
  /** Compute time: all ions, end states (GPU dispatch through completion, or the CPU flight). */
  flyMs: number;
  /** GPU ion buffer setup/upload + result readback around the compute (0 on the CPU). */
  transferMs: number;
  /** Trajectories of the drawn ions on the CPU (0 when `pathsGpu`). */
  pathsMs: number;
  /** The drawn trajectories were sampled by the GPU flight itself. */
  pathsGpu: boolean;
  totalMs: number;
  thinned: boolean;
  /** GPU memory of the resident field incl. its flight scratch, bytes (0 on the CPU). */
  gpuBytes: number;
  /** Most ions per GPU dispatch (0 on the CPU). */
  dispatchIons: number;
  /** Width of the packet's surface indices (1, 2 or 4 bytes). */
  surfaceBytes: 1 | 2 | 4;
  /** Key of this flight's final velocities, which the packet leaves out (`flightVelocities`). */
  token: number;
}

export interface Flight {
  header: FlightHeader;
  /** ions + 1 offsets into `points` (in points, not floats). */
  offsets: Uint32Array<ArrayBuffer>;
  /** x, y, z mm per trajectory point. */
  points: Float32Array<ArrayBuffer>;
  /** x, y, z mm, TOF us per ion. */
  ends: Float64Array<ArrayBuffer>;
  /** Index into header.surfaces per ion. */
  surface: Uint32Array<ArrayBuffer>;
  /** Final kinetic energy per ion, eV. */
  ke: Float64Array<ArrayBuffer>;
  /** UI round trip, ms. */
  roundTripMs: number;
  /** Final vx, vy, vz mm/us per ion, when known here (a flight loaded from the project folder). */
  velocities?: Float64Array<ArrayBuffer>;
}

/**
 * See the STR4 layout in src-tauri/src/fly.rs. `points`, `ends` and `ke` view `buf`; the packet's
 * drawn-only offsets and narrow surface indices are widened here to one entry per ion.
 */
export function parseFlight(buf: ArrayBuffer, roundTripMs: number): Flight {
  const bytes = new Uint8Array(buf);
  if (String.fromCharCode(...bytes.subarray(0, 4)) !== "STR4") throw new Error("the native core answered with an unknown flight packet");
  const headerLen = new DataView(buf).getUint32(4, true);
  const header = JSON.parse(new TextDecoder().decode(bytes.subarray(8, 8 + headerLen))) as FlightHeader;
  const n = header.ions;
  const w = header.surfaceBytes;
  const endAt = Math.ceil((8 + headerLen + 4 * (2 * header.drawn + 1 + 3 * header.points)) / 8) * 8;
  const size = endAt + 40 * n + w * n;
  if (size > buf.byteLength) throw new Error("the native core answered with a short flight packet");
  let at = 8 + headerLen;
  const take = <T>(make: (b: ArrayBuffer, o: number, n: number) => T, count: number, width = 4) => {
    const v = make(buf, at, count);
    at += count * width;
    return v;
  };
  const drawn = take((b, o, k) => new Uint32Array(b, o, k), header.drawn);
  const drawnOffsets = take((b, o, k) => new Uint32Array(b, o, k), header.drawn + 1);
  const points = take((b, o, k) => new Float32Array(b, o, k), header.points * 3);
  at = endAt;
  const ends = take((b, o, k) => new Float64Array(b, o, k), n * 4, 8);
  const ke = take((b, o, k) => new Float64Array(b, o, k), n, 8);
  const packed = take((b, o, k) => (w === 1 ? new Uint8Array(b, o, k) : w === 2 ? new Uint16Array(b, o, k) : new Uint32Array(b, o, k)), n, w);
  const surface = new Uint32Array(packed);
  // Ion i's points are offsets[i]..offsets[i + 1]: an empty range unless it is drawn.
  const offsets = new Uint32Array(n + 1);
  let d = 0;
  for (let i = 0; i < n; i++) {
    offsets[i] = drawnOffsets[d]!;
    if (drawn[d] === i) d++;
  }
  offsets[n] = drawnOffsets[d]!;
  return { header, offsets, points, ends, surface, ke, roundTripMs };
}
