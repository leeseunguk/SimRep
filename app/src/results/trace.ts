import { isTauri } from "@tauri-apps/api/core";
import { backendError, errorMessage, ionTrace } from "../scene/backend";
import type { Surface } from "../scene/flight";
import { flyRequest } from "../scene/simulation";
import { useApp, type PaObject } from "../state/store";
import { useResults } from "./store";

/** Per-ion time series (src-tauri/src/fly.rs `ion_trace`, SIT1 packet). */
export type TraceQuantity = "x" | "y" | "z" | "speed" | "ke" | "potential";
export const TRACE_QUANTITIES: readonly TraceQuantity[] = ["x", "y", "z", "speed", "ke", "potential"];
export const TRACE_UNITS: Record<TraceQuantity, string> = { x: "mm", y: "mm", z: "mm", speed: "mm/µs", ke: "eV", potential: "V" };

export interface TraceHeader {
  ion: number;
  massAmu: number;
  chargeE: number;
  samples: number;
  steps: number;
  thinned: boolean;
  surface: Surface;
  tofUs: number;
  field: "resident" | "array" | "exact";
  ms: number;
}

export interface Trace {
  header: TraceHeader;
  t: Float64Array;
  x: Float64Array;
  y: Float64Array;
  z: Float64Array;
  vx: Float64Array;
  vy: Float64Array;
  vz: Float64Array;
  speed: Float64Array;
  ke: Float64Array;
  potential: Float64Array;
}

const FIELDS = 9;

export function parseTrace(buf: ArrayBuffer): Trace {
  const bytes = new Uint8Array(buf);
  if (String.fromCharCode(...bytes.subarray(0, 4)) !== "SIT1") throw new Error("the native core answered with an unknown trace packet");
  const len = new DataView(buf).getUint32(4, true);
  const header = JSON.parse(new TextDecoder().decode(bytes.subarray(8, 8 + len))) as TraceHeader;
  const raw = new Float64Array(buf, 8 + len, header.samples * FIELDS);
  const col = (k: number) => {
    const out = new Float64Array(header.samples);
    for (let i = 0; i < header.samples; i++) out[i] = raw[i * FIELDS + k]!;
    return out;
  };
  const [t, x, y, z, vx, vy, vz, ke, potential] = [0, 1, 2, 3, 4, 5, 6, 7, 8].map(col) as [Float64Array, Float64Array, Float64Array, Float64Array, Float64Array, Float64Array, Float64Array, Float64Array, Float64Array];
  const speed = new Float64Array(header.samples);
  for (let i = 0; i < header.samples; i++) speed[i] = Math.hypot(vx[i]!, vy[i]!, vz[i]!);
  return { header, t, x, y, z, vx, vy, vz, speed, ke, potential };
}

// ------------------------------------------------------------------------------ the selected ion

const paOf = (id: string): PaObject | null => {
  const o = useApp.getState().objects.find((x) => x.id === id);
  return o?.kind === "pa" ? o : null;
};

/**
 * Keeps `useResults.trace` on the selected ion: re-traced after every flight of its array (so the
 * plots follow a live re-fly), one request at a time, latest wins.
 */
let running = false;
let dirty = false;

function schedule() {
  dirty = true;
  if (running) return;
  running = true;
  void (async () => {
    try {
      while (dirty) {
        dirty = false;
        const results = useResults.getState();
        const ion = results.ion;
        const o = ion && paOf(ion.objectId);
        const flight = o?.flight.result;
        if (!ion || !o || !flight || ion.index >= flight.header.ions) {
          if (results.trace || results.traceStatus !== "idle") useResults.setState({ trace: null, traceStatus: "idle", traceError: null });
          continue;
        }
        const t = results.trace;
        if (t && t.objectId === ion.objectId && t.index === ion.index && t.flight === flight) continue;
        useResults.setState({ traceStatus: "loading" });
        try {
          const data = parseTrace(await ionTrace(flyRequest(o), ion.index));
          const now = useResults.getState().ion;
          if (now?.objectId === ion.objectId && now.index === ion.index) {
            useResults.setState({ trace: { objectId: ion.objectId, index: ion.index, flight, data }, traceStatus: "idle", traceError: null });
          }
        } catch (err) {
          useResults.setState({ trace: null, traceStatus: "error", traceError: errorMessage(backendError(err)) });
        }
      }
    } finally {
      running = false;
    }
  })();
}

if (isTauri()) {
  useResults.subscribe((s, prev) => {
    if (s.ion !== prev.ion) schedule();
  });
  useApp.subscribe((s, prev) => {
    if (s.objects === prev.objects) return;
    const ion = useResults.getState().ion;
    if (!ion) return;
    const find = (list: typeof s.objects) => list.find((x) => x.id === ion.objectId);
    const a = find(s.objects);
    const b = find(prev.objects);
    if (a?.kind !== "pa" || b?.kind !== "pa" || a.flight.result !== b.flight.result) schedule();
  });
}
