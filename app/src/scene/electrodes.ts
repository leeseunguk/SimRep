import type { Message, MessageParams } from "../i18n";
import type { MessageKey } from "../i18n/en";
import type { PaObject } from "../state/store";
import { electrodeLabels } from "./backend";
import type { PaFormatId, PaGrid, PaInfo } from "./pa";
import { templateById } from "./templates";

/**
 * The electrodes of a potential array, as the scene tree lists them.
 *
 * Every grid point gets a label (u16):
 * - fast-adjust .pa0: the adjustable electrode it belongs to (from its solution, native core),
 *   0 for fixed electrodes;
 * - .pa#: the electrode number stored in the point (0 for points that are not numbered);
 * - anything else: electrodes are grouped by potential, label k = k-th distinct electrode value.
 * The surface carries the label per face, so one electrode can be highlighted or hidden.
 */
export interface ElectrodeSet {
  labels: Uint16Array<ArrayBuffer>;
  /** Electrode points per label. */
  counts: Record<number, number>;
  /** Grouped by potential: the value of label k is values[k - 1]. */
  values: number[] | null;
}

export interface PaElectrode {
  label: number;
  /** adjustable: fast-adjust voltage; numbered: .pa# number (no field yet); potential: fixed value. */
  kind: "adjustable" | "numbered" | "potential" | "fixed";
  /** SIMION electrode number (0 = the scalable fixed electrodes of a .pa0). */
  number: number | null;
  /** Current voltage (adjustable / potential). */
  voltage: number | null;
  /** Voltage stored in the file (adjustable). */
  baseline: number | null;
  points: number | null;
}

const MAX_LABEL = 1000;

function count(labels: Uint16Array, electrodes: Uint8Array): Record<number, number> {
  const counts: Record<number, number> = {};
  for (let i = 0; i < labels.length; i++) if (electrodes[i]) counts[labels[i]!] = (counts[labels[i]!] ?? 0) + 1;
  return counts;
}

/** Labels from the stored values (.pa# numbers, or groups of equal potential). */
export function valueLabels(info: PaInfo, grid: PaGrid, formatId: PaFormatId): ElectrodeSet {
  const { potentials, electrodes } = grid;
  const labels = new Uint16Array(potentials.length);
  let values: number[] | null = null;
  if (formatId === "paHash") {
    for (let i = 0; i < labels.length; i++) {
      if (!electrodes[i]) continue;
      const v = potentials[i]!;
      labels[i] = Number.isInteger(v) && v >= 1 && v <= MAX_LABEL ? v : 0;
    }
  } else {
    values = info.electrodeValues;
    const byValue = new Map(values.map((v, k) => [Math.fround(v), k + 1]));
    for (let i = 0; i < labels.length; i++) if (electrodes[i]) labels[i] = byValue.get(potentials[i]!) ?? 0;
  }
  return { labels, counts: count(labels, electrodes), values };
}

/** Labels of an array: from the native core for a fast-adjust .pa0, else from its values. */
export async function computeLabels(o: Pick<PaObject, "id" | "info" | "grid" | "formatId" | "fast">): Promise<ElectrodeSet> {
  if (o.fast?.state === "ready") {
    const buf = await electrodeLabels(o.id);
    if (buf.byteLength === o.info.points * 2) {
      const labels = new Uint16Array(buf);
      return { labels, counts: count(labels, o.grid.electrodes), values: null };
    }
  }
  return valueLabels(o.info, o.grid, o.formatId);
}

export function paElectrodes(o: PaObject): PaElectrode[] {
  const counts = o.electrodeSet?.counts ?? null;
  const points = (label: number) => (counts ? (counts[label] ?? 0) : null);
  if (o.fast?.state === "ready") {
    const list: PaElectrode[] = o.fast.electrodes.map((e) => ({
      label: e.number,
      kind: "adjustable",
      number: e.number,
      voltage: o.voltages[e.number] ?? e.voltage,
      baseline: e.voltage,
      points: points(e.number),
    }));
    if (counts?.[0] && !o.fast.electrodes.some((e) => e.number === 0))
      list.push({ label: 0, kind: "fixed", number: null, voltage: null, baseline: null, points: counts[0] });
    return list.sort((a, b) => (a.label || Infinity) - (b.label || Infinity));
  }
  if (!counts || !o.electrodeSet) return [];
  const values = o.electrodeSet.values;
  return Object.keys(counts)
    .map(Number)
    .sort((a, b) => (a || Infinity) - (b || Infinity))
    .map((label): PaElectrode => {
      if (label === 0) return { label, kind: "fixed", number: null, voltage: null, baseline: null, points: counts[0]! };
      if (values) return { label, kind: "potential", number: null, voltage: values[label - 1]!, baseline: null, points: counts[label]! };
      return { label, kind: "numbered", number: label, voltage: null, baseline: null, points: counts[label]! };
    });
}

type T = (key: MessageKey, params?: MessageParams) => string;

/** Display name: the user's, else the template role, else a generic one. */
export function electrodeName(o: PaObject, e: PaElectrode, t: T, num: (v: number) => string): string {
  const custom = o.electrodeNames[e.label];
  if (custom) return custom;
  const role: Message | undefined = o.template && e.number ? templateById(o.template).roles[e.number] : undefined;
  if (role) return t(role.key, role.params);
  if (e.kind === "fixed") return t("electrode.fixed");
  if (e.number === 0) return t("volts.fixed");
  if (e.kind === "potential") return t("electrode.atVoltage", { v: num(e.voltage!) });
  return t("volts.electrode", { n: e.number! });
}

/** Largest |V| among an array's electrodes (colour scale of the tree swatches). */
export function electrodeScale(list: readonly PaElectrode[]): number {
  return Math.max(1, ...list.map((e) => Math.abs(e.voltage ?? 0)));
}
