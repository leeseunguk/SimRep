import type { Message } from "../i18n";
import type { FlightHeader, IonSource } from "../scene/flight";
import type { Flags3, PaFormatId, PaKind } from "../scene/pa";
import type { Lang } from "../state/store";
import type { ResultSummary } from "./summary";

/**
 * Everything one report shows, gathered from the app (report/collect.ts) and rendered to HTML by
 * report/html.ts. Plain data: the renderer is pure and unit tested on hand-made models.
 */

/** One entry of the optional AI chat appendix (supplied through report/transcript.ts). */
export interface TranscriptItem {
  role: string;
  text: string;
  /** Tool name, for tool calls / results. */
  tool?: string | null;
}

export interface ElectrodeRow {
  name: string;
  /** SIMION electrode number (null: grouped by potential, or the fixed electrodes). */
  number: number | null;
  /** Current voltage, V (null: a .pa# number without a field yet). */
  voltage: number | null;
}

export interface ArraySummary {
  name: string;
  fileName: string;
  path: string | null;
  format: PaFormatId;
  kind: PaKind;
  dims: [number, number, number];
  spacing: [number, number, number];
  sizeMm: [number, number, number];
  mirror: Flags3;
  antiMirror: Flags3;
  field: "electrostatic" | "magnetic";
  points: number;
  electrodePoints: number;
  /** Sibling .cuts.json file name, or null. */
  cuts: string | null;
  electrodes: ElectrodeRow[];
}

/** Integration settings of every flight (state/runFormat.ts FLIGHT_TOLERANCES). */
export interface FlightTolerances {
  relativeTolerance: number;
  positionToleranceMm: number;
  velocityToleranceMmPerUs: number;
  initialStepUs: number;
  maxStepUs: number;
}

export interface SettingsSummary {
  refine: { device: string | null; tolerance: number; cycles: number | null; residual: number | null } | null;
  /** Requested flight mode. */
  precision: IonSource["precision"];
  /** What actually ran (null: nothing flown). */
  executed: FlightHeader["executed"] | null;
  deviceName: string | null;
  collision: NonNullable<IonSource["collision"]>;
  exactSurfaces: boolean;
  relativistic: boolean;
  durationUs: number;
  drawn: number;
  tolerances: FlightTolerances;
}

/** A selected stored run in the comparison. */
export interface RunColumn {
  label: string;
  createdAt: string;
  summary: ResultSummary;
  /** Its settings flattened to dotted keys (state/runFormat.ts `flatten`). */
  settings: Record<string, string>;
}

export interface ReportModel {
  lang: Lang;
  generatedAt: Date;
  simrepVersion: string;
  projectName: string;
  projectPath: string | null;
  array: ArraySummary | null;
  source: IonSource | null;
  settings: SettingsSummary | null;
  results: ResultSummary | null;
  /** PNG data URI of the 3D view, or why there is none. */
  view3d: { dataUri: string } | { unavailable: Message } | null;
  runs: RunColumn[];
  warnings: Message[];
  transcript: TranscriptItem[] | null;
}
