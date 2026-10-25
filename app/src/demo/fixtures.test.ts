import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { runOf, select } from "../results/analysis";
import { endStatesCsv } from "../results/csv";
import { parseFlight } from "../scene/flight";
import { parsePacket, parsePotentialsPacket } from "../scene/pa";

// The demo fixtures are packets recorded from the real native core. Decoding them here keeps the
// demo (and the e2e test that runs on it) in step with the packet formats.
const fx = JSON.parse(readFileSync(new URL("./fixtures/einzel.json", import.meta.url), "utf8"));
const bytes = (b64: string) => {
  const raw = Buffer.from(b64, "base64");
  return raw.buffer.slice(raw.byteOffset, raw.byteOffset + raw.byteLength) as ArrayBuffer;
};

describe("demo fixtures decode with the app's decoders", () => {
  it("opens the einzel .pa# and its refined .pa0", () => {
    const opened = parsePacket(bytes(fx.opened));
    expect(opened.info.points).toBe(opened.grid.potentials.length);
    const result = fx.messages.find((m: { packet?: string }) => m.packet && Buffer.from(m.packet, "base64").subarray(0, 4).toString() === "SPA1");
    const refined = parsePacket(bytes(result.packet));
    expect(refined.info.points).toBe(opened.info.points);
    const live = parsePotentialsPacket(bytes(fx.fast), refined.grid.electrodes);
    expect(live.grid.potentials.length).toBe(opened.info.points);
  });

  it("replays one progress message per V-cycle, as refine.rs streams them", () => {
    const progress = fx.messages.filter((m: { json?: { type: string } }) => m.json?.type === "progress").map((m: { json: { stage: number; cycle: number } }) => m.json);
    const stages = new Set(progress.map((p: { stage: number }) => p.stage));
    for (const s of stages) {
      const cycles = progress.filter((p: { stage: number }) => p.stage === s).map((p: { cycle: number }) => p.cycle);
      expect(cycles).toEqual(cycles.map((_: number, i: number) => i));
    }
    const start = fx.messages[0].json;
    expect(start).toMatchObject({ type: "start", tolerance: 1e-10 });
    expect(fx.messages.at(-1).json.type).toBe("done");
  });

  for (const precision of ["live", "precise"] as const) {
    it(`decodes the ${precision} flight and exports its end states`, () => {
      const flight = parseFlight(bytes(fx.flights[precision].packet), 0);
      expect(flight.header.precision).toBe(precision);
      const run = runOf(flight);
      expect(select(run, "tof", "all").length).toBe(flight.header.ions);
      const velocities = new Float64Array(bytes(fx.flights[precision].velocities));
      expect(velocities.length).toBe(3 * flight.header.ions);
      expect(endStatesCsv(run, velocities).trim().split("\r\n")).toHaveLength(flight.header.ions + 1);
    });
  }
});
