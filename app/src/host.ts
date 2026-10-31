import type { ComponentType } from "react";
import type { Refusal } from "./refusals";

/**
 * Host injection point. The desktop app and demo mode register nothing and behave as before; another
 * build of this UI (the SimRep-Web browser build) registers a host before the app modules load, so it
 * can plug in its own backend start-up, switch features off with an explanation, and add a few
 * pieces of UI. Everything here is optional: no host means every feature is on.
 *
 * The host must be registered before `main.tsx` is imported (scene/simulation.ts and others decide at
 * load time whether a backend is present).
 */

/** Features a host may lack. Each one gates the controls that would otherwise fail. */
export type HostFeature =
  /** Gallery templates (`create_template`). */
  | "templates"
  /** Field solves (`refine_start`). */
  | "refine"
  /** Fast adjust of electrode voltages (`fast_adjust`). */
  | "voltages"
  /** Ion flight (`fly`). */
  | "fly"
  /** GPU solves and flights (the GPU device choice). */
  | "gpu"
  /** The AI assistant. */
  | "ai"
  /** STEP/IGES import (OpenCascade). */
  | "cad"
  /** Geometry import wizard (GEM, meshes to a potential array). */
  | "wizard"
  /** SIMION workbenches (`.iob`). */
  | "iob"
  /** Data export dialog (`export_array`, `export_trajectories`). */
  | "exportData"
  /** Choosing an output folder on disk. */
  | "folders"
  /** The 5-minute tour (needs templates, refine and fly). */
  | "tour";

export interface Host {
  /** Why `feature` is off on this host, with what to do instead; null when it is available. */
  unavailable(feature: HostFeature): Refusal | null;
  /** Replaces the native start-up in runtime.ts (backend info, compute device, status, logs). */
  connect?(): Promise<void>;
  /** Shown in the status bar instead of the desktop runtime / compute items. */
  StatusItems?: ComponentType;
  /** Shown at the top of the start screen's side column. */
  StartPanel?: ComponentType;
  /** Extra entries of the top bar's Export menu; `close` closes the menu. */
  ExportMenuItems?: ComponentType<{ close(): void }>;
}

let current: Host | null = null;

/** Installs the host (once, before the app modules load). */
export function registerHost(host: Host | null): void {
  current = host;
}

/** The registered host, or null (desktop app, demo mode, plain browser preview). */
export function host(): Host | null {
  return current;
}

/** Why `feature` is off on the registered host; null when available (always, without a host). */
export function unavailable(feature: HostFeature): Refusal | null {
  return current?.unavailable(feature) ?? null;
}

/** True unless the registered host switches `feature` off. */
export function allows(feature: HostFeature): boolean {
  return unavailable(feature) === null;
}
