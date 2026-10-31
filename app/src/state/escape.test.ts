import { beforeEach, describe, expect, it } from "vitest";
import { useResults } from "../results/store";
import { cancelEscape, fieldEscape, ownEscape, workspaceEscape } from "./escape";
import { useLayout } from "./layout";
import { useApp } from "./store";

/** The part of a keydown event the Escape owners use; one event passes through both, as in the page. */
function escape(key = "Escape") {
  return {
    key,
    defaultPrevented: false,
    isComposing: false,
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

  it("a field's Escape is handled only when it cancelled an edit; the text is restored either way", () => {
    let restored = 0;
    const edited = escape();
    expect(cancelEscape(edited, true, () => restored++)).toBe(true);
    expect(edited.defaultPrevented).toBe(true);
    const unchanged = escape();
    expect(cancelEscape(unchanged, false, () => restored++)).toBe(false);
    expect(unchanged.defaultPrevented).toBe(false);
    expect(restored).toBe(2);
    expect(cancelEscape(escape("Enter"), true, () => restored++)).toBe(false);
    expect(cancelEscape(edited, true, () => restored++)).toBe(false);
    expect(restored).toBe(2);
  });

  describe("Escape from a field in a tablet drawer", () => {
    const inSimulation = (id: string) => id === "panel-simulation";
    beforeEach(() => useLayout.setState({ mode: "tablet", drawer: "simulation" }));

    it.each([
      ["nothing cancelled, field in the open drawer", {}, true],
      ["the field cancelled an edit", { handled: true }, false],
      ["IME composition", { composing: true }, false],
      ["field outside the open drawer", { inPanel: () => false }, false],
      ["no drawer open", { drawer: null }, false],
      ["desktop (>= 1024 px)", { mode: "desktop" }, false],
      ["phone", { mode: "phone" }, false],
    ] as const)("%s", (_, c: { handled?: boolean; composing?: boolean; inPanel?: (id: string) => boolean; drawer?: null; mode?: "desktop" | "phone" }, closes) => {
      if (c.drawer !== undefined || c.mode) useLayout.setState({ ...(c.drawer !== undefined && { drawer: c.drawer }), ...(c.mode && { mode: c.mode }) });
      const before = useLayout.getState().drawer;
      const e = escape();
      if (c.handled) e.preventDefault();
      e.isComposing = c.composing ?? false;
      expect(fieldEscape(e, c.inPanel ?? inSimulation)).toBe(closes);
      expect(useLayout.getState().drawer).toBe(closes ? null : before);
      expect(useApp.getState()).toMatchObject({ selectedId: "pa-1", selectedElectrode: 2 });
    });

    it("an edited field: the first Escape restores the text, the second closes the drawer", () => {
      let text = "edited";
      const name = "Einzel";
      const press = () => {
        const e = escape();
        cancelEscape(e, text !== name, () => (text = name));
        return fieldEscape(e, inSimulation);
      };
      expect(press()).toBe(false);
      expect(text).toBe(name);
      expect(useLayout.getState().drawer).toBe("simulation");
      expect(press()).toBe(true);
      expect(useLayout.getState().drawer).toBeNull();
    });
  });
});
