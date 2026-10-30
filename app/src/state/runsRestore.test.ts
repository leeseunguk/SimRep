import { beforeEach, expect, it, vi } from "vitest";
import { defaultSlice, type PaInfo } from "../scene/pa";
import { defaultSource, type Flight } from "../scene/flight";
import type { FlyRequest } from "../scene/backend";
import { flyRequest } from "../scene/simulation";
import { useApp, type PaObject } from "./store";
import { DEFAULT_RUNS_CONFIG, openRuns, recordFlight, resetRuns, restoreRun, saveRuns, useRuns } from "./runs";
import { sha256Hex } from "./runFormat";

const { fieldFiles, projectFiles } = vi.hoisted(() => ({ fieldFiles: new Map<string, string>(), projectFiles: new Map<string, Uint8Array<ArrayBuffer>>() }));
vi.mock("../scene/simulation", () => ({
  flyRequest: (o: PaObject): FlyRequest => ({
    id: o.id, voltages: Object.entries(o.voltages).map(([n, v]) => [Number(n), v]),
    source: { kind: "beam", count: 1, massAmu: 1, chargeE: 1, energyEv: 1, positionMm: [0, 0, 0], direction: [1, 0, 0], coneDeg: 0, shape: "line", widthMm: 0 },
    bounds: null, placement: null, durationUs: 1, relativistic: true, precision: "live", collision: "allCorners", draw: 0, surfaces: true,
  }),
  setFlightListener: vi.fn(),
}));
vi.mock("./projectDir", () => ({
  fileSha256: async (path: string) => {
    const bytes = fieldFiles.get(path);
    if (bytes === undefined) throw new Error(`missing ${path}`);
    return sha256Hex(bytes);
  },
  listProjectFiles: async () => [...projectFiles.keys()].map(path => ({ path })),
  readProjectFile: async (path: string) => projectFiles.get(path) ?? null,
  readProjectText: async (path: string) => projectFiles.has(path) ? new TextDecoder().decode(projectFiles.get(path)) : null,
  writeProjectFile: async (path: string, bytes: string | Uint8Array<ArrayBuffer>) => { projectFiles.set(path, typeof bytes === "string" ? new TextEncoder().encode(bytes) : bytes); },
  deleteProjectFile: async () => { throw new Error("unexpected pruning"); },
}));

const project = "/p/lens.simrep";
const info: PaInfo = {
  mode: -2, symmetry: "cylindrical", nx: 3, ny: 3, nz: 1, dx: 1, dy: 1, dz: 1,
  mirror: { x: false, y: true, z: false }, antiMirror: { x: false, y: false, z: false },
  field: "electrostatic", refinable: true, maxVoltage: 1000, ng: 100, points: 9, electrodePoints: 0,
  potentialMin: 0, potentialMax: 0, electrodeMin: null, electrodeMax: null, electrodeValues: [], distinctElectrodeValues: 0,
  pa0: { adjustable: 3, electrodeNumbers: [0, 1, 2], referenceVoltage: 1, fastScalable: true }, cuts: "lens.cuts.json",
};
function array(): PaObject {
  return {
    id: "lens", kind: "pa", fileName: "lens.pa0", fileSize: 128, visible: true, formatId: "pa0", info: { ...info },
    path: "/p/lens.pa0", fast: { state: "ready", electrodes: [{ number: 0, voltage: 0 }, { number: 1, voltage: 0 }, { number: 2, voltage: 60 }] },
    voltages: { 0: 0, 1: 0, 2: 60 }, grid: { potentials: new Float32Array(9), electrodes: new Uint8Array(9) },
    surface: { quads: new Float32Array(), count: 0, step: 1 }, showMirrors: true, slice: defaultSlice(info),
    electrodeSet: null, electrodeNames: {}, hiddenElectrodes: [], template: null, source: defaultSource(info),
    liveFly: false, showTrajectories: true, outputDir: null, refine: null, flight: { status: "idle", result: null, error: null },
  };
}
const flight: Flight = {
  header: {
    ions: 1, drawn: 0, points: 0, surfaces: [{ surface: { kind: "wall", face: "+x" }, count: 1, tofMean: 1 }],
    tof: { n: 1, min: 1, max: 1, mean: 1, std: 0 }, keMean: 1, device: { kind: "cpu", name: "CPU" },
    precision: "live", executed: { device: "cpu", precision: "f64", surfaces: "exact" }, fallback: null,
    surface: { file: "lens.cuts.json", accuracy: "source" }, fieldMs: 0, prepareMs: 0, flyMs: 1, transferMs: 0, pathsMs: 0,
    pathsGpu: false, totalMs: 1, thinned: false, gpuBytes: 0, dispatchIons: 0, surfaceBytes: 1, token: 1,
  },
  offsets: new Uint32Array(2), points: new Float32Array(), ends: new Float64Array([1, 0, 0, 1]),
  surface: new Uint32Array(1), ke: new Float64Array([1]), velocities: new Float64Array([1, 0, 0]), roundTripMs: 1,
};

beforeEach(() => {
  fieldFiles.clear(); projectFiles.clear();
  for (const file of ["lens.pa0", "lens.pa1", "lens.pa2", "lens.pa_", "lens.cuts.json"]) fieldFiles.set(`/p/${file}`, file);
  resetRuns(null);
  useApp.setState({ objects: [array()] });
});

for (const changed of ["lens.pa1", "lens.pa_", "lens.cuts.json"]) {
  it(`reopens saved results unchanged, but reopens stale after only ${changed} changes`, async () => {
    const original = array();
    const record = await recordFlight(original.id, flyRequest(original), flight);
    expect(record).not.toBeNull();
    await saveRuns(project);
    // Reopen from serialized run.json and binary results, after clearing held flights.
    await openRuns(project, DEFAULT_RUNS_CONFIG);
    const reopened = array();
    expect(await restoreRun(reopened, record!.id)).toBe("loaded");
    expect(useApp.getState().objects.find(o => o.id === reopened.id)).toMatchObject({ flight: { stored: { runId: record!.id } } });
    fieldFiles.set(`/p/${changed}`, "edited dependency bytes");
    await openRuns(project, DEFAULT_RUNS_CONFIG);
    expect(await restoreRun(array(), record!.id)).toBe("stale");
    expect(useRuns.getState().stale[record!.id]).toBe(true);
    expect(fieldFiles.get("/p/lens.pa0")).toBe("lens.pa0");
  });
}
