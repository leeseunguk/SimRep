import { beforeEach, describe, expect, it } from "vitest";
import { useResults } from "../results/store";
import { ownEscape, workspaceEscape } from "./escape";
import { useLayout } from "./layout";
import { useApp } from "./store";

/** The part of a keydown event the Escape owners use; one event passes through both, as in the page. */
function escape(key = "Escape") {
  return {
    key,
    defaultPrevented: false,
    preventDefault() {
      this.defaultPrevented = true;
    },
  };
}

describe("Escape ownership", () => {
  beforeEach(() => {
    useApp.setState({ selectedId: "pa-1", selectedElectrode: 2, helpOpen: false, home: false, objects: [] });
    useResults.setState({ ion: null });
    useLayout.setState({ mode: "phone", drawer: null, tab: "simulation" });
  });

  it("a menu that takes Escape closes and the selection survives", () => {
    let closed = 0;
    const e = escape();
    expect(ownEscape(e, () => closed++)).toBe(true);
    workspaceEscape(e);
    expect(closed).toBe(1);
    expect(useApp.getState()).toMatchObject({ selectedId: "pa-1", selectedElectrode: 2 });
  });

  it("only Escape is taken, and only once", () => {
    let closed = 0;
    expect(ownEscape(escape("Enter"), () => closed++)).toBe(false);
    const e = escape();
    ownEscape(e, () => closed++);
    expect(ownEscape(e, () => closed++)).toBe(false);
    expect(closed).toBe(1);
  });

  it("without a popup, Escape clears the selection", () => {
    workspaceEscape(escape());
    expect(useApp.getState()).toMatchObject({ selectedId: null, selectedElectrode: null });
  });

  it("on a tablet, Escape closes the open drawer before touching the selection", () => {
    useLayout.setState({ mode: "tablet", drawer: "simulation" });
    workspaceEscape(escape());
    expect(useLayout.getState().drawer).toBeNull();
    expect(useApp.getState().selectedId).toBe("pa-1");
    workspaceEscape(escape());
    expect(useApp.getState().selectedId).toBeNull();
  });
});
