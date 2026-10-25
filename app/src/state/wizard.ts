import { create } from "zustand";
import type { Message } from "../i18n";
import type { BuildMethod, BuildResult, CadSurface, RefineDevice } from "../scene/backend";
import type { GeometryInfo } from "../scene/geometry";
import type { MeshData, Vec3 } from "../scene/mesh";

/** One part of the imported model and what it becomes (electrode number, name, voltage). */
export interface WizardPart {
  index: number;
  name: string;
  number: number;
  voltage: number;
}

export type WizardStep = "parts" | "grid" | "refine";

/**
 * The import wizard (components/ImportWizard.tsx, flow in scene/importWizard.ts). Kept out of the
 * scene store: the preview is not a document object (no undo steps, not saved in projects) until
 * the wizard builds and opens the array.
 */
export interface WizardState {
  path: string;
  fileName: string;
  status: "inspecting" | "ready" | "error" | "building" | "refining" | "done";
  error: Message | null;
  /** The native core has no CAD (STEP/IGES) support or cannot find OpenCascade. */
  cadUnavailable: boolean;
  info: GeometryInfo | null;
  meshes: MeshData[];
  /** Source unit override (null = file default). */
  units: string | null;
  repair: boolean;
  parts: WizardPart[];
  method: BuildMethod;
  cadSurface: CadSurface;
  spacing: number;
  /** Grid domain (mm): first node and extent. */
  origin: Vec3;
  size: Vec3;
  outputDir: string;
  stem: string;
  sidecar: boolean;
  device: RefineDevice;
  step: WizardStep;
  /** Part hovered / selected in the table (highlighted in 3D). */
  highlight: number | null;
  build: BuildResult | null;
  /** The array the wizard opened. */
  objectId: string | null;
}

interface Store {
  wizard: WizardState | null;
  set(patch: Partial<WizardState>): void;
  open(w: WizardState): void;
  close(): void;
}

export const useWizard = create<Store>()((set) => ({
  wizard: null,
  set: (patch) => set((s) => (s.wizard ? { wizard: { ...s.wizard, ...patch } } : {})),
  open: (wizard) => set({ wizard }),
  close: () => set({ wizard: null }),
}));

/** Grid points along each axis for the wizard's domain and spacing. */
export function wizardDims(w: Pick<WizardState, "size" | "spacing" | "info">): [number, number, number] {
  const gem = w.info?.gem;
  return [0, 1, 2].map((a) => {
    if (gem && gem.dims[a] === 1) return 1;
    return Math.max(2, Math.round(w.size[a]! / w.spacing) + 1);
  }) as [number, number, number];
}
