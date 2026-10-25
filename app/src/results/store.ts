import { create } from "zustand";
import type { Message } from "../i18n";
import type { Flight } from "../scene/flight";
import { useApp, type PaObject } from "../state/store";
import type { MapPlane, Quantity, RunData } from "./analysis";
import type { Trace, TraceQuantity } from "./trace";

/**
 * Results panel state: view settings, the selected ion (linked across the 3D view, plots and the
 * table) and kept runs for comparison. Separate from the scene store so none of it is an undo step
 * or part of the project file.
 */

export type ResultsTab = "hist" | "map" | "ion" | "table" | "compare";
export type TableColumn = "index" | "surface" | Quantity;

export interface SelectedIon {
  objectId: string;
  index: number;
}

/** A kept run (Compare tab). `run` shares the flight's arrays (flights are never mutated). */
export interface Snapshot {
  id: number;
  label: string;
  /** Electrode voltages, ions, precision when kept. */
  detail: string;
  time: number;
  run: RunData;
  /** Series colour slot (--plot-series-N). */
  color: number;
  visible: boolean;
}

export interface TraceState {
  objectId: string;
  index: number;
  /** Flight the trace was taken after (the field it flew through). */
  flight: Flight;
  data: Trace;
}

interface Views {
  hist: { quantity: Quantity; bins: number; log: boolean; filter: string; zoom: [number, number] | null };
  map: { surface: string | null; plane: MapPlane; mode: "density" | "points"; colorBy: Quantity; log: boolean; bins: number };
  ionView: { quantities: TraceQuantity[]; xAxis: "t" | "x" };
  table: { sort: TableColumn; desc: boolean; filter: string };
  compare: { quantity: Quantity; bins: number; log: boolean; filter: string; normalize: boolean; zoom: [number, number] | null };
}

interface ResultsState extends Views {
  open: boolean;
  /** Panel height (CSS px) set by dragging; null = --results-h. */
  height: number | null;
  tab: ResultsTab;
  ion: SelectedIon | null;
  trace: TraceState | null;
  traceStatus: "idle" | "loading" | "error";
  traceError: Message | null;
  snapshots: Snapshot[];

  setOpen(open: boolean): void;
  setHeight(height: number | null): void;
  setTab(tab: ResultsTab): void;
  selectIon(ion: SelectedIon | null): void;
  view<K extends keyof Views>(key: K, patch: Partial<Views[K]>): void;
  keep(o: PaObject, run: RunData, detail: string): void;
  patchSnapshot(id: number, patch: Partial<Pick<Snapshot, "visible" | "label">>): void;
  removeSnapshot(id: number): void;
}

const STORAGE = "simrep.results";
/** Kept runs (each holds its flight's buffers). */
export const MAX_SNAPSHOTS = 5;

function saved(): { open: boolean; height: number | null; tab: ResultsTab } {
  try {
    const s = JSON.parse(localStorage.getItem(STORAGE) ?? "{}") as Partial<{ open: boolean; height: number | null; tab: ResultsTab }>;
    return { open: s.open ?? true, height: typeof s.height === "number" ? s.height : null, tab: s.tab ?? "hist" };
  } catch {
    return { open: true, height: null, tab: "hist" };
  }
}

let nextSnapshot = 1;

export const useResults = create<ResultsState>()((set, get) => ({
  ...saved(),
  ion: null,
  trace: null,
  traceStatus: "idle",
  traceError: null,
  snapshots: [],
  hist: { quantity: "tof", bins: 0, log: false, filter: "landed", zoom: null },
  map: { surface: null, plane: "auto", mode: "density", colorBy: "tof", log: true, bins: 80 },
  ionView: { quantities: ["x", "y", "speed", "ke", "potential"], xAxis: "t" },
  table: { sort: "index", desc: false, filter: "all" },
  compare: { quantity: "tof", bins: 0, log: false, filter: "landed", normalize: false, zoom: null },

  setOpen: (open) => set({ open }),
  setHeight: (height) => set({ height }),
  setTab: (tab) => set({ tab, open: true }),
  selectIon: (ion) => {
    const cur = get().ion;
    if (cur?.objectId === ion?.objectId && cur?.index === ion?.index) return;
    set({ ion });
  },
  view: (key, patch) => set((s) => ({ [key]: { ...s[key], ...patch } }) as Partial<ResultsState>),
  keep: (o, run, detail) =>
    set((s) => {
      // Drop the oldest first, then take a colour none of the remaining runs uses.
      const kept = s.snapshots.slice(-(MAX_SNAPSHOTS - 1));
      const used = new Set(kept.map((x) => x.color));
      const color = [0, 1, 2, 3, 4].find((c) => !used.has(c)) ?? 0;
      if (s.snapshots.length === 0) nextSnapshot = 1;
      const id = nextSnapshot++;
      const snap: Snapshot = { id, label: `${o.fileName} · #${id}`, detail, time: Date.now(), run, color, visible: true };
      return { snapshots: [...kept, snap] };
    }),
  patchSnapshot: (id, patch) => set((s) => ({ snapshots: s.snapshots.map((x) => (x.id === id ? { ...x, ...patch } : x)) })),
  removeSnapshot: (id) => set((s) => ({ snapshots: s.snapshots.filter((x) => x.id !== id) })),
}));

useResults.subscribe((s, prev) => {
  if (s.open !== prev.open || s.height !== prev.height || s.tab !== prev.tab) {
    localStorage.setItem(STORAGE, JSON.stringify({ open: s.open, height: s.height, tab: s.tab }));
  }
});

/** The array whose results the panel shows: the selected one if it has a flight, else the first flown. */
export function activeArray(objects: ReturnType<typeof useApp.getState>["objects"], selectedId: string | null): PaObject | null {
  const flown = objects.filter((o): o is PaObject => o.kind === "pa" && o.flight.result !== null);
  return flown.find((o) => o.id === selectedId) ?? flown[0] ?? null;
}

export function useActiveArray(): PaObject | null {
  return useApp((s) => activeArray(s.objects, s.selectedId));
}

// A re-fly with fewer ions (or a removed array) drops a selection that no longer exists.
useApp.subscribe((s, prev) => {
  if (s.objects === prev.objects) return;
  const ion = useResults.getState().ion;
  if (!ion) return;
  const o = s.objects.find((x) => x.id === ion.objectId);
  const f = o?.kind === "pa" ? o.flight.result : null;
  if (!o || (f && ion.index >= f.header.ions)) useResults.getState().selectIon(null);
});
