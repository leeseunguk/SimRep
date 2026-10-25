import { describe, expect, it } from "vitest";
import { extentAt, indicesOf, runOf, select, stats, statsAt } from "../results/analysis";
import { endStatesCsv } from "../results/csv";
import { parseFlight, type FlightHeader } from "./flight";

// A TOF pair distinct in f64 but identical in f32 must survive STR4 -> parseFlight -> analysis -> CSV
// (also run alone by `npm run check:flight`).
const tof = [100, 100.000001];
const ends = [1.0000000001, 2.0000000002, 3.0000000003, tof[0]!, 1.0000000002, 2.0000000003, 3.0000000004, tof[1]!];
const ke = [10.0000000001, 10.0000000002];
const velocity = new Float64Array([0.10000000001, 0.20000000002, 0.30000000003, 0.40000000004, 0.50000000005, 0.60000000006]);

function header(drawn: boolean, surfaceBytes: FlightHeader["surfaceBytes"]): FlightHeader {
  return {
    ions: 2, drawn: drawn ? 1 : 0, points: drawn ? 1 : 0,
    surfaces: [
      { surface: { kind: "electrode", number: 2, voltage: null }, count: 1, tofMean: tof[0]! },
      { surface: { kind: "wall", face: "+x" }, count: 1, tofMean: tof[1]! },
    ],
    tof: { n: 2, min: tof[0]!, max: tof[1]!, mean: (tof[0]! + tof[1]!) / 2, std: (tof[1]! - tof[0]!) / 2 },
    keMean: (ke[0]! + ke[1]!) / 2, device: { kind: "cpu", name: "test" },
    precision: "live", executed: { device: "cpu", precision: "f64", surfaces: "exact" },
    fallback: { code: "exactSurfaces" }, surface: { file: "test.cuts.json", accuracy: "source" },
    fieldMs: 0, prepareMs: 0, flyMs: 0, transferMs: 0, pathsMs: 0, pathsGpu: false,
    totalMs: 0, thinned: false, gpuBytes: 0, dispatchIons: 0, surfaceBytes, token: 1,
  };
}

/** An STR4 packet laid out as src-tauri/src/fly.rs `packet` writes it. */
function packet(h: FlightHeader): ArrayBuffer {
  const json = new TextEncoder().encode(JSON.stringify(h));
  const headerLen = Math.ceil(json.length / 8) * 8;
  const endAt = Math.ceil((8 + headerLen + 4 * (2 * h.drawn + 1 + 3 * h.points)) / 8) * 8;
  const buf = new ArrayBuffer(endAt + 40 * h.ions + h.surfaceBytes * h.ions);
  const bytes = new Uint8Array(buf);
  bytes.set(new TextEncoder().encode("STR4"));
  new DataView(buf).setUint32(4, headerLen, true);
  bytes.fill(32, 8, 8 + headerLen);
  bytes.set(json, 8);
  const u32 = h.drawn ? [1, 0, 1] : [0];
  new Uint32Array(buf, 8 + headerLen, u32.length).set(u32);
  if (h.drawn) new Float32Array(buf, 8 + headerLen + u32.length * 4, 3).set([1, 2, 3]);
  new Float64Array(buf, endAt, ends.length).set(ends);
  new Float64Array(buf, endAt + ends.length * 8, ke.length).set(ke);
  new DataView(buf).setUint8(endAt + 40 * h.ions + h.surfaceBytes, 1);
  return buf;
}

describe("STR4 flight packets keep f64 end states", () => {
  for (const drawn of [false, true]) {
    for (const w of [1, 2, 4] as const) {
      it(`decodes, analyses and exports (${drawn ? "1 drawn" : "no paths"}, ${w} B surfaces)`, () => {
        // The fixture only tests anything if f32 would merge the pair.
        expect(Math.fround(tof[0]!)).toBe(Math.fround(tof[1]!));
        const h = header(drawn, w);
        const buf = packet(h);
        const flight = parseFlight(buf, 0);
        expect(flight.points).toBeInstanceOf(Float32Array);
        expect(flight.ends).toBeInstanceOf(Float64Array);
        expect(Array.from(flight.ends)).toEqual(ends);
        expect(Array.from(flight.ke)).toEqual(ke);
        expect(Array.from(flight.surface)).toEqual([0, 1]);
        expect(Array.from(flight.offsets)).toEqual(drawn ? [0, 0, 1] : [0, 0, 0]);
        expect(flight.header.executed).toEqual(h.executed);
        expect(() => parseFlight(buf.slice(0, -1), 0)).toThrow(/short flight packet/);

        const run = runOf(flight);
        const landed = select(run, "tof", "landed");
        expect(Array.from(landed)).toEqual(tof);
        const summary = stats(landed)!;
        expect([summary.min, summary.max]).toEqual(tof);
        expect(summary.std).toBeGreaterThan(0);
        expect(statsAt(run.cols.tof, indicesOf(run, "landed"))).toEqual(summary);
        expect(extentAt(run.cols.tof, indicesOf(run, "landed"))).toEqual(tof);
        expect(Array.from(select(run, "tof", "all", [100.0000005, 101]))).toEqual([tof[1]]);

        const csv = endStatesCsv(run, velocity).trim().split("\r\n");
        expect(csv[0]).toBe("ion,surface,x_mm,y_mm,z_mm,r_mm,tof_us,ke_ev,vx_mm_per_us,vy_mm_per_us,vz_mm_per_us");
        const rows = csv.slice(1).map((r) => r.split(","));
        for (let i = 0; i < run.n; i++) {
          expect(rows[i]!.slice(2).map(Number)).toEqual([
            ends[i * 4], ends[i * 4 + 1], ends[i * 4 + 2], Math.hypot(ends[i * 4 + 1]!, ends[i * 4 + 2]!),
            tof[i], ke[i], ...Array.from(velocity.subarray(i * 3, i * 3 + 3)),
          ]);
        }
        expect(rows[0]![6]).not.toBe(rows[1]![6]);
        expect(endStatesCsv(run, velocity, [1]).trim().split("\r\n")[1]).toBe(rows[1]!.join(","));
      });
    }
  }
});
