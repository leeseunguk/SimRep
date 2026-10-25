import { create } from "zustand";
import type { Flight } from "../scene/flight";
import { useApp, type PaObject } from "./store";

/**
 * The 5-minute guided tour (components/Tour.tsx): einzel template -> focus voltage -> fly -> TOF
 * histogram -> CSV export. Offered once on first launch, re-opened from Help or the start screen.
 * Not a document field: no undo step, not in the project file.
 */

export const TOUR_STEPS = ["template", "voltage", "fly", "histogram", "export", "done"] as const;
export type TourStep = (typeof TOUR_STEPS)[number];

/** Electrode the tour asks to move (the einzel template's "Focus"). */
export const TOUR_ELECTRODE = 2;

/** What the scene looked like when a step began; a step is done when the scene moves past it. */
export interface StepMark {
  /** Focus voltage of the einzel array (voltage step). */
  voltage: number | null;
  /** Flight shown (fly step): a newer flight completes the step. */
  flight: Flight | null;
  /** ms since epoch (export step: only exports after this count). */
  time: number;
}

interface TourState {
  /** "offer": the first-launch invitation; "running": the coach card. */
  mode: "off" | "offer" | "running";
  step: number;
  mark: StepMark;
  start(): void;
  go(step: number): void;
  /** Close the tour (skip, finish, or "not now" on the offer). */
  close(): void;
}

const STORAGE = "simrep.tour";

/** The einzel template array the tour works on (newest first). */
export function tourArray(objects = useApp.getState().objects): PaObject | null {
  for (let i = objects.length - 1; i >= 0; i--) {
    const o = objects[i]!;
    if (o.kind === "pa" && o.template === "einzel") return o;
  }
  return null;
}

function markNow(): StepMark {
  const o = tourArray();
  return { voltage: o ? (o.voltages[TOUR_ELECTRODE] ?? null) : null, flight: o?.flight.result ?? null, time: Date.now() };
}

function remember() {
  localStorage.setItem(STORAGE, "seen");
}

export const useTour = create<TourState>()((set) => ({
  mode: "off",
  step: 0,
  mark: { voltage: null, flight: null, time: 0 },
  start: () => {
    remember();
    // Step 1 picks a template on the start screen.
    useApp.getState().setHelpOpen(false);
    useApp.getState().setHome(true);
    set({ mode: "running", step: 0, mark: markNow() });
  },
  go: (step) => {
    const app = useApp.getState();
    const o = tourArray();
    // The voltage and fly steps work in the array's properties: select it.
    if (o && (TOUR_STEPS[step] === "voltage" || TOUR_STEPS[step] === "fly") && app.selectedId !== o.id) app.select(o.id);
    set({ step: Math.max(0, Math.min(TOUR_STEPS.length - 1, step)), mark: markNow() });
  },
  close: () => {
    remember();
    set({ mode: "off" });
  },
}));

/** First launch: offer the tour (desktop only: the tour needs templates, i.e. the native core). */
export function initTour(desktop: boolean) {
  if (desktop && localStorage.getItem(STORAGE) === null) useTour.setState({ mode: "offer" });
}
