import { create } from "zustand";

export type ExportKind = "geometry" | "grid" | "isosurface" | "trajectories" | "pa";

/** The export dialog (components/ExportDialog.tsx): which array, which tab. */
interface ExportStore {
  objectId: string | null;
  kind: ExportKind;
  open(objectId: string, kind?: ExportKind): void;
  close(): void;
}

export const useExport = create<ExportStore>()((set) => ({
  objectId: null,
  kind: "geometry",
  open: (objectId, kind) => set((s) => ({ objectId, kind: kind ?? s.kind })),
  close: () => set({ objectId: null }),
}));
