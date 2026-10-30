import { beforeEach, describe, expect, it, vi } from "vitest";
import type { PaInfo, FastStatus } from "../scene/pa";
import type { RunInputs, RunSettings } from "./runFormat";
import { fieldFingerprint } from "./fieldFingerprint";
import { runMatches, settingsHash, sha256Hex, FLIGHT_TOLERANCES } from "./runFormat";

const { contents } = vi.hoisted(() => ({ contents: new Map<string, string>() }));
vi.mock("./projectDir", () => ({
  fileSha256: async (path: string) => {
    const data = contents.get(path);
    if (data === undefined) throw new Error(`missing ${path}`);
    return sha256Hex(data);
  },
}));
const info = { cuts: "lens.cuts.json" } as PaInfo;
const fast: FastStatus = { state: "ready", electrodes: [{ number: 2, voltage: 60 }, { number: 0, voltage: 0 }, { number: 1, voltage: 0 }] };
const path = "/project/lens.PA0";
const fingerprint = () => fieldFingerprint(path, info, fast, true);
const inputs = (sha256: string): RunInputs => ({
  pa: { path, sha256 }, voltages: { "2": 60 }, source: { kind: "file", path: "/project/ions.ion", sha256: "ions" },
  bounds: null, placement: null, durationUs: 1, relativistic: true, precision: "precise", collision: "allCorners",
  draw: 1, surfaces: true, tolerances: FLIGHT_TOLERANCES,
});

describe("stored field dependencies", () => {
  beforeEach(() => {
    contents.clear();
    for (const [name, value] of Object.entries({ "lens.PA0": "pa0", "lens.PA1": "basis1", "lens.PA2": "basis2", "lens.PA_": "scalable", "lens.cuts.json": "cuts" })) contents.set(`/project/${name}`, value);
  });

  it.each(["lens.PA1", "lens.PA2", "lens.PA_", "lens.cuts.json"])("marks a stored run stale when only %s changes", async name => {
    const saved = await fingerprint();
    const savedInputs = inputs(saved);
    const record = { settings: { inputs: savedInputs } as RunSettings, settingsHash: await settingsHash(savedInputs), stored: { results: true, trajectories: false, velocities: false, bytes: 0, dropped: null } };
    expect(runMatches(record, await fingerprint(), await settingsHash(inputs(await fingerprint())))).toBe("match");
    contents.set(`/project/${name}`, "changed in place");
    const reopened = await fingerprint();
    expect(runMatches(record, reopened, await settingsHash(inputs(reopened)))).toBe("stale");
    expect(contents.get(path)).toBe("pa0");
  });

  it("rejects missing loaded dependencies instead of reusing a cached hash", async () => {
    await fingerprint();
    contents.delete("/project/lens.PA2");
    await expect(fingerprint()).rejects.toThrow();
  });

  it("hashes only active surfaces and bases actually loaded by the session", async () => {
    const saved = await fieldFingerprint(path, info, null, false);
    contents.set("/project/lens.cuts.json", "different inactive surface");
    contents.set("/project/lens.PA1", "different unloaded basis");
    expect(await fieldFingerprint(path, info, null, false)).toBe(saved);
    contents.set(path, "changed map");
    expect(await fieldFingerprint(path, info, null, false)).not.toBe(saved);
  });

  it("does not depend on the directory or metadata electrode order", async () => {
    const saved = await fingerprint();
    for (const [file, data] of [...contents]) contents.set(file.replace("/project/", "/moved/"), data);
    const reordered: FastStatus = { state: "ready", electrodes: [...fast.electrodes].reverse() };
    expect(await fieldFingerprint("/moved/lens.PA0", info, reordered, true)).toBe(saved);
  });
});
