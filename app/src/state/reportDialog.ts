import { create } from "zustand";

/** The report export dialog (components/ReportDialog.tsx) and the command palette. */
interface ReportDialogStore {
  open: boolean;
  paletteOpen: boolean;
  show(): void;
  close(): void;
  setPalette(open: boolean): void;
}

export const useReportDialog = create<ReportDialogStore>()((set) => ({
  open: false,
  paletteOpen: false,
  show: () => set({ open: true, paletteOpen: false }),
  close: () => set({ open: false }),
  setPalette: (paletteOpen) => set({ paletteOpen }),
}));
