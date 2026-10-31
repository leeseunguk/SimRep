import { create } from "zustand";

/**
 * Responsive layout of the workspace, by window width (CSS px):
 * - desktop (>= 1024): the three-column layout (scene | 3D view + results | properties), unchanged.
 * - tablet (600-1023): the 3D view takes the full width; the scene and properties panels open as
 *   overlay drawers from the top bar, one at a time.
 * - phone (< 600): one column, the 3D view on top and the panels as tabs below it
 *   (Scene / Simulation / Results).
 * The mode is mirrored to `<html data-layout>`, which the stylesheets key on.
 */
export type LayoutMode = "desktop" | "tablet" | "phone";

/** Smallest width of the tablet layout (narrower is phone). */
export const TABLET_MIN_W = 600;
/** Smallest width of the desktop layout (narrower is tablet). */
export const DESKTOP_MIN_W = 1024;

export function layoutFor(width: number): LayoutMode {
  return width >= DESKTOP_MIN_W ? "desktop" : width >= TABLET_MIN_W ? "tablet" : "phone";
}

/** A side panel: the scene tree (left column) or properties / wizard / assistant (right column). */
export type SidePanel = "scene" | "simulation";
/** A phone tab under the 3D view. */
export type PhoneTab = SidePanel | "results";

/** The phone tab to show: Results falls back to Simulation while there is nothing flown. */
export function shownTab(tab: PhoneTab, hasResults: boolean): PhoneTab {
  return tab === "results" && !hasResults ? "simulation" : tab;
}

/** The phone tabs in visual order. */
export const PHONE_TABS: readonly PhoneTab[] = ["scene", "simulation", "results"];

/**
 * Phone tab keyboard navigation (one tab stop, arrows move and select): the tab that Left/Right
 * (wrapping), Home or End selects from `current`; null for any other key. Results is skipped while
 * it is disabled (nothing flown).
 */
export function tabForKey(current: PhoneTab, key: string, hasResults: boolean): PhoneTab | null {
  const tabs = PHONE_TABS.filter((t) => t !== "results" || hasResults);
  const i = Math.max(0, tabs.indexOf(current));
  const at =
    key === "ArrowRight" ? (i + 1) % tabs.length : key === "ArrowLeft" ? (i - 1 + tabs.length) % tabs.length : key === "Home" ? 0 : key === "End" ? tabs.length - 1 : -1;
  return tabs[at] ?? null;
}

interface Store {
  mode: LayoutMode;
  /** Tablet: the open drawer, if any. */
  drawer: SidePanel | null;
  /** Phone: the selected tab. */
  tab: PhoneTab;
  setMode(mode: LayoutMode): void;
  toggleDrawer(panel: SidePanel): void;
  setTab(tab: PhoneTab): void;
  /** Brings a panel into view: the tablet drawer or the phone tab (desktop shows every panel). */
  show(panel: PhoneTab): void;
}

const initialWidth = typeof window === "undefined" ? DESKTOP_MIN_W : window.innerWidth;

export const useLayout = create<Store>()((set, get) => ({
  mode: layoutFor(initialWidth),
  drawer: null,
  tab: "simulation",
  setMode: (mode) => set(mode === get().mode ? {} : { mode, drawer: null }),
  toggleDrawer: (panel) => set({ drawer: get().drawer === panel ? null : panel }),
  setTab: (tab) => set({ tab }),
  show: (panel) => {
    const { mode } = get();
    if (mode === "phone") set({ tab: panel });
    else if (mode === "tablet" && panel !== "results") set({ drawer: panel });
  },
}));
