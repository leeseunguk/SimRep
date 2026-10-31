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
 * A text field's or select's own Escape. `cancel` always runs (it restores the text); the event is
 * marked handled only when there was something to cancel (`changed`), so an Escape that cancelled
 * nothing is still free to close the drawer (`fieldEscape`). Returns whether it was handled.
 */
export function cancelEscape(e: Pick<KeyboardEvent, "key" | "defaultPrevented" | "preventDefault">, changed: boolean, cancel: () => void): boolean {
  if (e.key !== "Escape" || e.defaultPrevented) return false;
  cancel();
  if (!changed) return false;
  e.preventDefault();
  return true;
}

/** Close the open tablet drawer and put focus back on its toggle; false when no drawer is open. */
function closeDrawer(): boolean {
  const layout = useLayout.getState();
  if (layout.mode !== "tablet" || !layout.drawer) return false;
  const panel = layout.drawer;
  layout.toggleDrawer(panel);
  globalThis.document?.getElementById(drawerToggleId(panel))?.focus();
  return true;
}

/**
 * Escape from a text field, textarea or select (App). The control's own Escape comes first: when it
 * cancelled something (an edit, an open popup) it marked the event handled and nothing else happens.
 * Otherwise, inside the open tablet drawer (`inPanel(id)`: the event target lies in the element with
 * that id), it closes the drawer. The selection and the rest of the workspace Escape stay out of
 * reach while typing. Returns whether it closed the drawer.
 */
export function fieldEscape(e: Pick<KeyboardEvent, "defaultPrevented" | "isComposing">, inPanel: (id: string) => boolean): boolean {
  if (e.defaultPrevented || e.isComposing) return false;
  const drawer = useLayout.getState().drawer;
  return drawer !== null && inPanel(`panel-${drawer}`) && closeDrawer();
}

/**
 * The workspace Escape, one step at a time: close help, leave the start screen, close the tablet
 * drawer (focus back on its toggle), clear the selected ion, then the selection. Nothing when a
 * popup already took this Escape.
 */
export function workspaceEscape(e: Pick<KeyboardEvent, "defaultPrevented">): void {
  if (e.defaultPrevented) return;
  const app = useApp.getState();
  if (app.helpOpen) app.setHelpOpen(false);
  else if (app.home && app.objects.length) app.setHome(false);
  else if (closeDrawer()) return;
  else if (useResults.getState().ion) useResults.getState().selectIon(null);
  else app.select(null);
}
