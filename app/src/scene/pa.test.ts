import { describe, expect, it } from "vitest";
import { encodePacket, parsePacket, parsePotentialsPacket, type PaInfo } from "./pa";

const info = { points: 3, nx: 3, ny: 1, nz: 1, fileName: "t.pa" } as unknown as PaInfo;

describe("PA packets", () => {
  it("round-trips an SPA1 packet", () => {
    const buf = encodePacket(info, new Float32Array([0, 50, 100]), new Uint8Array([1, 0, 1]));
    const { info: back, grid } = parsePacket(buf);
    expect(back).toEqual(info);
    expect(Array.from(grid.potentials)).toEqual([0, 50, 100]);
    expect(Array.from(grid.electrodes)).toEqual([1, 0, 1]);
    expect(() => parsePacket(buf.slice(0, -1))).toThrow();
  });

  it("dequantizes an SPQ1 live packet onto the array's electrode flags", () => {
    const json = new TextEncoder().encode(JSON.stringify(info));
    const len = Math.ceil(json.length / 4) * 4;
    const buf = new ArrayBuffer(8 + len + 16 + 3 * 2);
    const bytes = new Uint8Array(buf);
    bytes.set(new TextEncoder().encode("SPQ1"));
    new DataView(buf).setUint32(4, len, true);
    bytes.fill(32, 8, 8 + len);
    bytes.set(json, 8);
    new DataView(buf).setFloat64(8 + len, -10, true);
    new DataView(buf).setFloat64(8 + len + 8, 0.5, true);
    new Uint16Array(buf, 8 + len + 16, 3).set([0, 20, 65535]);
    const flags = new Uint8Array([0, 1, 0]);
    const { grid } = parsePotentialsPacket(buf, flags);
    expect(Array.from(grid.potentials)).toEqual([-10, 0, -10 + 65535 * 0.5]);
    expect(grid.electrodes).toBe(flags);
    expect(() => parsePotentialsPacket(buf, new Uint8Array(2))).toThrow(/does not match/);
  });
});
