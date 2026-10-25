import { encodePacket, PA_MAX_POINTS, PaReadError, type PaInfo } from "./pa";

/**
 * Browser-preview decoder for .pa/.pa#/.pa0. The desktop app decodes with simrep-io in Rust
 * (`read_pa` in src-tauri/src/pa.rs); this follows the same layout and checks so the browser
 * preview can show PA files too, and emits the same packet.
 */
export function decodePa(buf: ArrayBuffer): ArrayBuffer {
  if (buf.byteLength === 0) throw new PaReadError("empty", "0 bytes");
  const view = new DataView(buf);
  let at = 0;
  const need = (bytes: number) => {
    if (at + bytes > buf.byteLength) throw new PaReadError("truncated", `needs ${at + bytes} bytes, file has ${buf.byteLength}`);
  };
  const i32 = () => (need(4), (at += 4), view.getInt32(at - 4, true));
  const f64 = () => (need(8), (at += 8), view.getFloat64(at - 8, true));

  const mode = i32();
  if (mode !== -1 && mode !== -2) throw new PaReadError("notPa", `unsupported PA header mode ${mode}`);
  const sym = i32();
  if (sym !== 0 && sym !== 1) throw new PaReadError("badHeader", `invalid PA symmetry ${sym}`);
  const maxVoltage = f64();
  const nx = i32();
  const ny = i32();
  const nz = i32();
  const mirror = i32() >>> 0;
  const [dx, dy, dz] = mode === -2 ? [f64(), f64(), f64()] : [1, 1, 1];

  const cylindrical = sym === 0;
  const mirrors = { x: (mirror & 1) !== 0, y: (mirror & 2) !== 0, z: (mirror & 4) !== 0 };
  if (!(nx >= 3 && nx <= 90000 && ny >= 3 && ny <= 90000 && nz >= 1 && nz <= 90000))
    throw new PaReadError("badHeader", "PA dimensions require nx,ny in 3..=90000 and nz in 1..=90000");
  if (cylindrical && (nz !== 1 || !mirrors.y)) throw new PaReadError("badHeader", "cylindrical PA requires nz=1 and y mirroring");
  if (nz === 1 && mirrors.z) throw new PaReadError("badHeader", "z mirroring requires a 3D PA");
  for (const s of [dx, dy, dz])
    if (!Number.isFinite(s) || s < 1e-6 || s > 900) throw new PaReadError("badHeader", "PA grid spacing must be finite and in 1e-6..=900 mm");
  if (!Number.isFinite(maxVoltage) || maxVoltage <= 0 || !Number.isFinite(2 * maxVoltage))
    throw new PaReadError("badHeader", "PA max_voltage must be positive, finite, and safe to double");

  const n = nx * ny * nz;
  if (n > PA_MAX_POINTS) throw new PaReadError("tooLarge", String(n));
  need(n * 8);

  const potentials = new Float32Array(n);
  const electrodes = new Uint8Array(n);
  let pMin = Infinity, pMax = -Infinity, eMin = Infinity, eMax = -Infinity, ePoints = 0;
  const distinct = new Set<number>();
  // Both header sizes (32, 56) keep the data 8-byte aligned; WebView platforms are little-endian.
  const raw = at % 8 === 0 ? new Float64Array(buf, at, n) : null;
  for (let i = 0; i < n; i++) {
    const r = raw ? raw[i]! : view.getFloat64(at + i * 8, true);
    if (!Number.isFinite(r)) throw new PaReadError("badHeader", "non-finite PA potential");
    const e = r > maxVoltage;
    const p = e ? r - 2 * maxVoltage : r;
    potentials[i] = p;
    if (p < pMin) pMin = p;
    if (p > pMax) pMax = p;
    if (e) {
      electrodes[i] = 1;
      ePoints++;
      if (p < eMin) eMin = p;
      if (p > eMax) eMax = p;
      if (distinct.size < 1001) distinct.add(p + 0);
    }
  }
  at += n * 8;

  let pa0: PaInfo["pa0"] = null;
  if (at < buf.byteLength) {
    const adjustable = i32();
    if (adjustable < 1 || adjustable > 1001) throw new PaReadError("badPa0", "PA0 adjustable-electrode count must be in 1..=1001");
    const referenceVoltage = f64();
    const numbers: number[] = [];
    let entries = 0;
    while (entries < 32 || numbers.length < adjustable) {
      if (entries >= 1001) throw new PaReadError("badPa0", "PA0 electrode table exceeds 1001 entries");
      const index = n >= 2 ** 31 ? Number(view.getBigInt64((need(8), (at += 8), at - 8), true)) : i32();
      if (index !== -1) numbers.push(entries);
      entries++;
    }
    let fastScalable = false;
    if (at < buf.byteLength) {
      const flag = i32();
      if (flag === 1) {
        need(entries * 8);
        at += entries * 8;
        fastScalable = true;
      } else if (flag !== 0) throw new PaReadError("badPa0", `unsupported PA0 scalable flag ${flag}`);
    }
    if (at < buf.byteLength) throw new PaReadError("badPa0", "unexpected data after PA0 trailer");
    pa0 = { adjustable, electrodeNumbers: numbers, referenceVoltage, fastScalable };
  }

  const info: PaInfo = {
    mode: mode as -1 | -2,
    symmetry: cylindrical ? "cylindrical" : "planar",
    nx,
    ny,
    nz,
    dx,
    dy,
    dz,
    mirror: mirrors,
    antiMirror: { x: (mirror & (1 << 29)) !== 0, y: (mirror & (1 << 30)) !== 0, z: (mirror & (1 << 31)) !== 0 },
    field: mirror & 8 ? "magnetic" : "electrostatic",
    refinable: (mirror & (1 << 28)) === 0,
    maxVoltage,
    ng: (mirror >>> 4) & ((1 << 17) - 1),
    points: n,
    electrodePoints: ePoints,
    potentialMin: pMin,
    potentialMax: pMax,
    electrodeMin: ePoints ? eMin : null,
    electrodeMax: ePoints ? eMax : null,
    electrodeValues: [...distinct].sort((a, b) => a - b).slice(0, 64),
    distinctElectrodeValues: distinct.size,
    pa0,
  };
  return encodePacket(info, potentials, electrodes);
}
