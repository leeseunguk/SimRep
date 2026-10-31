import { beforeEach, describe, expect, it } from "vitest";
import { layoutFor, shownTab, useLayout } from "./layout";

describe("layoutFor", () => {
  it("maps widths to the three layouts at 600 and 1024 px", () => {
    expect(layoutFor(390)).toBe("phone");
    expect(layoutFor(599)).toBe("phone");
    expect(layoutFor(600)).toBe("tablet");
    expect(layoutFor(768)).toBe("tablet");
    expect(layoutFor(1023)).toBe("tablet");
    expect(layoutFor(1024)).toBe("desktop");
    expect(layoutFor(1280)).toBe("desktop");
  });
});

describe("shownTab", () => {
  it("shows Simulation instead of Results until something has flown", () => {
    expect(shownTab("results", false)).toBe("simulation");
    expect(shownTab("results", true)).toBe("results");
    expect(shownTab("scene", false)).toBe("scene");
  });
});

describe("useLayout", () => {
  beforeEach(() => useLayout.setState({ mode: "tablet", drawer: null, tab: "simulation" }));

  it("toggles one tablet drawer at a time", () => {
    const s = useLayout.getState();
    s.toggleDrawer("scene");
    expect(useLayout.getState().drawer).toBe("scene");
    s.toggleDrawer("simulation");
    expect(useLayout.getState().drawer).toBe("simulation");
    s.toggleDrawer("simulation");
    expect(useLayout.getState().drawer).toBeNull();
  });

  it("show opens the drawer on a tablet, the tab on a phone, nothing on desktop", () => {
    useLayout.getState().show("simulation");
    expect(useLayout.getState().drawer).toBe("simulation");
    useLayout.getState().show("results");
    expect(useLayout.getState().drawer).toBe("simulation");

    useLayout.getState().setMode("phone");
    expect(useLayout.getState().drawer).toBeNull();
    useLayout.getState().show("results");
    expect(useLayout.getState().tab).toBe("results");

    useLayout.getState().setMode("desktop");
    useLayout.getState().show("scene");
    expect(useLayout.getState()).toMatchObject({ drawer: null, tab: "results" });
  });
});
