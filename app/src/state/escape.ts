import { useResults } from "../results/store";
import { useLayout } from "./layout";
import { useApp } from "./store";

/**
 * Escape ownership. A popup (menu, drawer-level popover) that closes on Escape claims the event
 * with `ownEscape`, which marks it handled; the workspace Escape (`workspaceEscape`, App) then
 * leaves the workspace alone, so dismissing a popup never clears the selection.
 */
export function ownEscape(e: Pick<KeyboardEvent, "key" | "defaultPrevented" | "preventDefault">, close: () => void): boolean {
  if (e.key !== "Escape" || e.defaultPrevented) return false;
  e.preventDefault();
  close();
  return true;
}

/** The id of a tablet drawer's toggle button in the top bar (focus returns there on close). */
export const drawerToggleId = (panel: "scene" | "simulation") => `drawer-toggle-${panel}`;

/**
 * The workspace Escape, one step at a time: close help, leave the start screen, close the tablet
 * drawer (focus back on its toggle), clear the selected ion, then the selection. Nothing when a
 * popup already took this Escape.
 */
export function workspaceEscape(e: Pick<KeyboardEvent, "defaultPrevented">): void {
  if (e.defaultPrevented) return;
  const app = useApp.getState();
  const layout = useLayout.getState();
  if (app.helpOpen) app.setHelpOpen(false);
  else if (app.home && app.objects.length) app.setHome(false);
  else if (layout.mode === "tablet" && layout.drawer) {
    const panel = layout.drawer;
    layout.toggleDrawer(panel);
    globalThis.document?.getElementById(drawerToggleId(panel))?.focus();
  } else if (useResults.getState().ion) useResults.getState().selectIon(null);
  else app.select(null);
}
