import { isTauri } from "@tauri-apps/api/core";
import type { MessageKey } from "../i18n/en";
import { closeSession } from "../scene/backend";
import { useApp, type SceneObject } from "./store";

/**
 * Undo / redo for every edit of the project document.
 *
 * The document is the editable part of the scene objects (DOC_FIELDS: names, visibility, voltages,
 * ion source, section plane, ...) plus which objects exist (imports and removals). Computed state
 * (grids, surfaces, refine progress, flights) is never rolled back: undo merges the document
 * fields of a snapshot into the live objects, and only objects that no longer exist come back
 * whole. Edits are recorded automatically by watching the store, so every control is covered;
 * system updates (refine results, template setup, project loading) run inside `quietly`.
 *
 * Consecutive edits of the same thing (one slider, one field) coalesce into one step while the
 * pointer stays down (a drag, live re-fly included) or within COALESCE_MS without a new click.
 *
 * Native array sessions stay open while any snapshot still references their object, so undoing a
 * removal brings the array back without reading the file again.
 */

const LIMIT = 200;
const COALESCE_MS = 1000;

const DOC_FIELDS: Record<SceneObject["kind"], readonly string[]> = {
  pa: ["customName", "visible", "voltages", "electrodeNames", "hiddenElectrodes", "source", "liveFly", "showTrajectories", "showMirrors", "slice", "outputDir"],
  mesh: ["customName", "visible", "units"],
  pending: ["customName"],
  loading: [],
};

const LABELS: Record<string, MessageKey> = {
  customName: "history.rename",
  electrodeNames: "history.rename",
  visible: "history.visibility",
  hiddenElectrodes: "history.visibility",
  voltages: "history.voltage",
  source: "history.source",
  slice: "history.slice",
  units: "history.units",
  outputDir: "history.folder",
};

interface Step {
  objects: SceneObject[];
  label: MessageKey;
}

let past: Step[] = [];
let future: Step[] = [];
/** Objects after the last recorded change: what an undo step stores. */
let baseline: SceneObject[] = useApp.getState().objects;
/** Document at the last save / load / new project, for the unsaved marker. */
let saved: SceneObject[] = baseline;
let lastKey: string | null = null;
let lastTime = 0;
let lastGesture = -1;
let gesture = 0;
let pointerDown = false;
let applying = false;
let quiet = 0;
let structural = 0;

window.addEventListener("pointerdown", () => {
  gesture++;
  pointerDown = true;
}, true);
window.addEventListener("pointerup", () => (pointerDown = false), true);
window.addEventListener("pointercancel", () => (pointerDown = false), true);

const docObjects = (objects: readonly SceneObject[]) => objects.filter((o) => o.kind !== "loading");
const field = (o: SceneObject, f: string) => (o as unknown as Record<string, unknown>)[f];
const same = (a: unknown, b: unknown) => a === b || (typeof a === "object" && typeof b === "object" && JSON.stringify(a) === JSON.stringify(b));

/** Sub-keys of an object-valued field that differ (which voltage, which source parameter, ...). */
function subKeys(a: unknown, b: unknown): string {
  if (typeof a !== "object" || typeof b !== "object" || !a || !b) return "";
  const ra = a as Record<string, unknown>;
  const rb = b as Record<string, unknown>;
  const keys = new Set([...Object.keys(ra), ...Object.keys(rb)]);
  return [...keys].filter((k) => !same(ra[k], rb[k])).sort().join(",");
}

/** What changed in the document between two object lists (null = nothing). */
function diff(prev: readonly SceneObject[], next: readonly SceneObject[]): { key: string; label: MessageKey } | null {
  const a = docObjects(prev);
  const b = docObjects(next);
  const ids = (l: SceneObject[]) => l.map((o) => o.id).join("|");
  if (ids(a) !== ids(b)) {
    const before = new Set(a.map((o) => o.id));
    const added = b.some((o) => !before.has(o.id));
    return { key: `structure:${++structural}`, label: added ? "history.import" : "history.remove" };
  }
  const changes: { id: string; f: string; sub: string }[] = [];
  b.forEach((o, i) => {
    const p = a[i]!;
    if (o === p) return;
    for (const f of DOC_FIELDS[o.kind]) if (!same(field(o, f), field(p, f))) changes.push({ id: o.id, f, sub: subKeys(field(p, f), field(o, f)) });
  });
  if (changes.length === 0) return null;
  const first = changes[0]!;
  const label = LABELS[first.f] ?? "history.display";
  if (changes.length > 1) return { key: `multi:${++structural}`, label };
  return { key: `${first.id}:${first.f}:${first.sub}`, label };
}

function publish() {
  useApp.setState({
    history: { undo: past.at(-1)?.label ?? null, redo: future.at(-1)?.label ?? null },
    dirty: diff(saved, useApp.getState().objects) !== null,
  });
}

/** Arrays opened natively and not closed yet. */
const sessions = new Set<string>();

function collectSessions() {
  if (!isTauri()) return;
  const alive = new Set<string>();
  for (const o of useApp.getState().objects) {
    alive.add(o.id);
    if (o.kind === "pa") sessions.add(o.id);
  }
  for (const step of [...past, ...future]) for (const o of step.objects) alive.add(o.id);
  for (const id of sessions)
    if (!alive.has(id)) {
      closeSession(id);
      sessions.delete(id);
    }
}

useApp.subscribe((s, prev) => {
  if (s.objects === prev.objects) return;
  if (applying) return;
  const change = diff(baseline, s.objects);
  if (change && quiet === 0) {
    const now = performance.now();
    const coalesce = change.key === lastKey && gesture === lastGesture && (pointerDown || now - lastTime < COALESCE_MS);
    if (!coalesce) {
      past.push({ objects: baseline, label: change.label });
      if (past.length > LIMIT) past.shift();
      future = [];
    }
    lastKey = change.key;
    lastTime = now;
    lastGesture = gesture;
  }
  baseline = s.objects;
  if (change) publish();
  collectSessions();
});

/** Merges the document fields of `target` into the live objects (computed state stays). */
function restore(current: readonly SceneObject[], target: readonly SceneObject[]): SceneObject[] {
  const byId = new Map(current.map((o) => [o.id, o]));
  const targetIds = new Set(target.map((o) => o.id));
  const out: SceneObject[] = [];
  for (const t of target) {
    const c = byId.get(t.id);
    if (t.kind === "loading") {
      // A placeholder in the step is not part of the document; the array it became is.
      if (c?.kind === "loading") out.push(c);
      continue;
    }
    if (!c) {
      out.push(t);
      continue;
    }
    if (c.kind !== t.kind) {
      out.push(c);
      continue;
    }
    const patch: Record<string, unknown> = {};
    for (const f of DOC_FIELDS[t.kind]) if (!same(field(c, f), field(t, f))) patch[f] = field(t, f);
    out.push(Object.keys(patch).length ? ({ ...c, ...patch } as SceneObject) : c);
  }
  // Imports still reading are not part of any step.
  for (const c of current) if (c.kind === "loading" && !targetIds.has(c.id)) out.push(c);
  return out;
}

function move(from: Step[], to: Step[]) {
  const step = from.pop();
  if (!step) return;
  const s = useApp.getState();
  to.push({ objects: s.objects, label: step.label });
  const objects = restore(s.objects, step.objects);
  const before = new Set(s.objects.map((o) => o.id));
  const added = objects.find((o) => !before.has(o.id));
  const selectionGone = s.selectedId !== null && !objects.some((o) => o.id === s.selectedId);
  applying = true;
  try {
    useApp.setState({
      objects,
      ...(added ? { selectedId: added.id, selectedElectrode: null } : selectionGone ? { selectedId: null, selectedElectrode: null } : {}),
    });
  } finally {
    applying = false;
  }
  baseline = objects;
  lastKey = null;
  publish();
  collectSessions();
}

export const undo = () => move(past, future);
export const redo = () => move(future, past);

/** Runs a system update (not a user edit): no undo step, still marks the project changed. */
export function quietly<T>(fn: () => T): T {
  quiet++;
  try {
    return fn();
  } finally {
    quiet--;
  }
}

/** Holds recording during an async system task (project loading, template setup). */
export async function quietlyAsync<T>(fn: () => Promise<T>): Promise<T> {
  quiet++;
  try {
    return await fn();
  } finally {
    quiet--;
  }
}

/** The current document is the saved one (after save). */
export function markSaved() {
  saved = useApp.getState().objects;
  publish();
  collectSessions();
}

/** The document has never been saved in this form (restored from the autosave). */
export function markUnsaved() {
  saved = [];
  publish();
}

/** Fresh history for a new or freshly opened project. */
export function resetHistory() {
  past = [];
  future = [];
  baseline = useApp.getState().objects;
  lastKey = null;
  markSaved();
}
