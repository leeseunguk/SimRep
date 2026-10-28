import { describe, expect, it } from "vitest";
import type { FlightHeader } from "../scene/flight";
import { parseProject } from "./projectFormat";
import {
  canonicalJson,
  decodeResults,
  decodeTrajectories,
  encodeResults,
  encodeTrajectories,
  flightFromStored,
  newRunId,
  planStorage,
  pruneRuns,
  resultsBytes,
  RUN_ID,
  runMatches,
  settingsDiff,
  settingsHash,
  trajectoryBytes,
  FLIGHT_TOLERANCES,
  type RunInputs,
  type RunSettings,
} from "./runFormat";

const inputs = (patch: Partial<RunInputs> = {}): RunInputs => ({
  pa: { path: "/p/einzel.pa0", sha256: "a".repeat(64) },
  voltages: { "1": 0, "2": 80, "3": 0 },
  source: { kind: "beam", count: 40, massAmu: 100, chargeE: 1, energyEv: 100, positionMm: [1, 0, 0], direction: [1, 0, 0], coneDeg: 0, shape: "line", widthMm: 14 },
  bounds: null,
  placement: null,
  durationUs: 100,
  relativistic: true,
  precision: "live",
  collision: "allCorners",
  draw: 300,
  surfaces: true,
  tolerances: { ...FLIGHT_TOLERANCES },
  ...patch,
});

describe("project format", () => {
  const v1 = {
    format: "simrep-project",
    version: 1,
    savedAt: "2026-01-01T00:00:00.000Z",
    arrays: [{ file: { path: "einzel.pa0", absolute: "/p/einzel.pa0" }, name: null, results: { ions: 40 } }],
    meshes: [],
    view: { camera: null, projection: "perspective", grid: true },
  };

  it("opens version 1 files as version 2 without runs (they fly again)", () => {
    const doc = parseProject(JSON.stringify(v1));
    expect(doc.version).toBe(2);
    expect(doc.arrays[0]!.run).toBeNull();
    expect(doc.runs).toEqual({ keep: 50, capMb: 200 });
    expect((doc.arrays[0] as { results?: unknown }).results).toEqual({ ions: 40 });
  });

  it("keeps the run reference and run settings of version 2 files", () => {
    const v2 = { ...v1, version: 2, runs: { keep: 7, capMb: 20 }, arrays: [{ ...v1.arrays[0], run: "20260101-000000000-abcd" }] };
    const doc = parseProject(JSON.stringify(v2));
    expect(doc.arrays[0]!.run).toBe("20260101-000000000-abcd");
    expect(doc.runs).toEqual({ keep: 7, capMb: 20 });
  });

  it("ignores a run field in a version 1 file and refuses newer formats", () => {
    expect(parseProject(JSON.stringify({ ...v1, arrays: [{ ...v1.arrays[0], run: "x" }] })).arrays[0]!.run).toBeNull();
    expect(() => parseProject(JSON.stringify({ ...v1, version: 3 }))).toThrow(/newer/);
    expect(() => parseProject(JSON.stringify({ format: "other", version: 1 }))).toThrow(/not a SimRep project/);
  });
});

describe("matching stored runs", () => {
  it("hashes settings independently of key order and array path", async () => {
    const a = inputs();
    const b = { ...inputs(), voltages: { "3": 0, "2": 80, "1": 0 }, pa: { path: "/elsewhere/einzel.pa0", sha256: "a".repeat(64) } };
    expect(await settingsHash(a)).toBe(await settingsHash(b));
    expect(canonicalJson({ b: 1, a: [2, { d: 1, c: 2 }] })).toBe('{"a":[2,{"c":2,"d":1}],"b":1}');
  });

  it("is a match only for the same PA contents and settings", async () => {
    const hash = await settingsHash(inputs());
    const record = { settings: { inputs: inputs() } as RunSettings, settingsHash: hash, stored: { results: true } as never };
    expect(runMatches(record, "a".repeat(64), hash)).toBe("match");
    expect(runMatches(record, "b".repeat(64), hash)).toBe("stale");
    const changed = await settingsHash(inputs({ voltages: { "1": 0, "2": 81, "3": 0 } }));
    expect(changed).not.toBe(hash);
    expect(runMatches(record, "a".repeat(64), changed)).toBe("stale");
    expect(await settingsHash(inputs({ precision: "precise" }))).not.toBe(hash);
    expect(runMatches({ ...record, stored: { results: false } as never }, "a".repeat(64), hash)).toBe("noResults");
  });
});

describe("history pruning", () => {
  const run = (i: number, pinned = false) => ({ id: `r${i}`, pinned, createdAt: `2026-01-01T00:00:${String(i).padStart(2, "0")}.000Z` });

  it("drops the oldest unpinned runs beyond the bound", () => {
    const runs = [5, 4, 3, 2, 1].map((i) => run(i));
    expect(pruneRuns(runs, 3)).toEqual(["r1", "r2"]);
    expect(pruneRuns(runs, 5)).toEqual([]);
  });

  it("never prunes pinned or protected runs, even past the bound", () => {
    const runs = [run(4), run(3, true), run(2), run(1, true)];
    expect(pruneRuns(runs, 2)).toEqual(["r2", "r4"]);
    expect(pruneRuns(runs, 2, new Set(["r2"]))).toEqual(["r4"]);
    expect(pruneRuns([run(1, true), run(2, true)], 1)).toEqual([]);
  });
});

describe("stored run files", () => {
  const header = { ions: 3, drawn: 2, points: 3 } as FlightHeader;
  const flight = {
    ends: Float64Array.from([1, 2, 3, 10.000000001, 4, 5, 6, 11, 7, 8, 9, 12]),
    ke: Float64Array.from([100, 101, 102]),
    surface: Uint32Array.from([0, 1, 0]),
    offsets: Uint32Array.from([0, 2, 2, 3]),
    points: Float32Array.from([0, 0, 0, 1, 1, 1, 2, 2, 2]),
  };

  it("round-trips results (f64 exact) and trajectories", () => {
    const v = Float64Array.from([0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9]);
    const bytes = encodeResults(flight, v);
    expect(bytes.length).toBe(resultsBytes(3, true));
    const r = decodeResults(bytes);
    expect(Array.from(r.ends)).toEqual(Array.from(flight.ends));
    expect(Array.from(r.velocities!)).toEqual(Array.from(v));
    expect(Array.from(r.surface)).toEqual([0, 1, 0]);
    const t = decodeTrajectories(encodeTrajectories(flight));
    expect(encodeTrajectories(flight).length).toBe(trajectoryBytes(3, 3));
    expect(Array.from(t.offsets)).toEqual([0, 2, 2, 3]);
    expect(Array.from(t.points)).toEqual(Array.from(flight.points));
    const f = flightFromStored(header, r, null);
    expect(f.header.drawn).toBe(0);
    expect(f.offsets.length).toBe(4);
    expect(decodeResults(encodeResults(flight, null)).velocities).toBeNull();
    expect(() => decodeResults(encodeResults(flight, null).subarray(0, 20))).toThrow(/truncated/);
  });

  it("drops trajectories first, then results, under the size cap", () => {
    const all = resultsBytes(1000, true) + trajectoryBytes(1000, 50_000);
    expect(planStorage(1000, 50_000, true, all)).toMatchObject({ results: true, trajectories: true, dropped: null });
    expect(planStorage(1000, 50_000, true, all - 1)).toMatchObject({ results: true, trajectories: false, dropped: "trajectories" });
    expect(planStorage(1000, 50_000, true, resultsBytes(1000, true) - 1)).toMatchObject({ results: false, trajectories: false, dropped: "results", bytes: 0 });
  });
});

describe("settings diff and ids", () => {
  it("lists differing settings first", () => {
    const s = (i: RunInputs): RunSettings => ({ inputs: i, executed: { device: "cpu", precision: "f64", surfaces: "grid", deviceName: "CPU", fallback: null }, simrepVersion: "0.1.0", computeMs: 1 });
    const rows = settingsDiff(s(inputs()), s(inputs({ voltages: { "1": 0, "2": 90, "3": 0 } })));
    expect(rows[0]).toEqual({ key: "inputs.voltages.2", a: "80", b: "90", same: false });
    expect(rows.filter((r) => !r.same)).toHaveLength(1);
  });

  it("makes sortable, folder-safe run ids", () => {
    const id = newRunId(new Date(Date.UTC(2026, 9, 28, 1, 2, 3, 4)), () => 0.5);
    expect(id).toBe("20261028-010203004-8000");
    expect(RUN_ID.test(id)).toBe(true);
  });
});
