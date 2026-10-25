import { beforeEach, describe, expect, it } from "vitest";
import { MAX_SNAPSHOTS, useResults } from "../results/store";
import type { RunData } from "../results/analysis";
import { useApp, type PaObject } from "./store";

const pa = (id: string) => ({ id, kind: "pa", fileName: `${id}.pa0`, voltages: { 1: 0 }, visible: true }) as unknown as PaObject;

describe("app store", () => {
  beforeEach(() => useApp.setState({ objects: [], selectedId: null, logs: [], toasts: [] }));

  it("persists language and theme", () => {
    useApp.getState().setLang("en");
    useApp.getState().setThemePref("dark");
    expect(localStorage.getItem("simrep.lang")).toBe("en");
    expect(useApp.getState()).toMatchObject({ lang: "en", themePref: "dark" });
  });

  it("patches one array and leaves the others", () => {
    useApp.getState().addObject(pa("a"));
    useApp.getState().addObject(pa("b"));
    const before = useApp.getState().objects[1];
    useApp.getState().updatePa("a", (o) => ({ voltages: { ...o.voltages, 2: 80 } }));
    const [a, b] = useApp.getState().objects as PaObject[];
    expect(a!.voltages).toEqual({ 1: 0, 2: 80 });
    expect(b).toBe(before);
  });

  it("drops the selection with the removed object", () => {
    useApp.getState().addObject(pa("a"));
    useApp.getState().select("a", 2);
    expect(useApp.getState().selectedElectrode).toBe(2);
    useApp.getState().removeObject("a");
    expect(useApp.getState()).toMatchObject({ selectedId: null, selectedElectrode: null, objects: [] });
  });

  it("numbers log entries and toasts", () => {
    useApp.getState().log("info", { key: "refine.starting" });
    useApp.getState().log("warning", { key: "refine.cancelled" });
    const [first, second] = useApp.getState().logs;
    expect(second!.id).toBeGreaterThan(first!.id);
    useApp.getState().toast("error", { key: "refine.failed" });
    const toast = useApp.getState().toasts[0]!;
    useApp.getState().dismissToast(toast.id);
    expect(useApp.getState().toasts).toEqual([]);
  });
});

describe("results store", () => {
  beforeEach(() => useResults.setState({ snapshots: [] }));

  it(`gives each kept run its own colour, up to ${MAX_SNAPSHOTS}`, () => {
    const run = { n: 0 } as RunData;
    for (let i = 0; i < MAX_SNAPSHOTS; i++) useResults.getState().keep(pa("a"), run, `run ${i}`);
    expect(new Set(useResults.getState().snapshots.map((s) => s.color)).size).toBe(MAX_SNAPSHOTS);
    useResults.getState().removeSnapshot(useResults.getState().snapshots[1]!.id);
    useResults.getState().keep(pa("a"), run, "again");
    expect(useResults.getState().snapshots.at(-1)!.color).toBe(1);
  });

  it(`drops the oldest run beyond ${MAX_SNAPSHOTS}`, () => {
    const run = { n: 0 } as RunData;
    for (let i = 0; i < MAX_SNAPSHOTS + 2; i++) useResults.getState().keep(pa("a"), run, `run ${i}`);
    expect(useResults.getState().snapshots.map((s) => s.detail).slice(-3)).toEqual(["run 4", "run 5", "run 6"]);
    expect(useResults.getState().snapshots).toHaveLength(MAX_SNAPSHOTS);
  });

  it("never gives two kept runs the same colour once the oldest are dropped", () => {
    const run = { n: 0 } as RunData;
    for (let i = 0; i < MAX_SNAPSHOTS * 2 + 1; i++) {
      useResults.getState().keep(pa("a"), run, `run ${i}`);
      const colors = useResults.getState().snapshots.map((s) => s.color);
      expect(new Set(colors).size).toBe(colors.length);
    }
  });

  it("opens the panel when a tab is chosen and remembers it", () => {
    useResults.getState().setOpen(false);
    useResults.getState().setTab("table");
    expect(useResults.getState()).toMatchObject({ open: true, tab: "table" });
    expect(JSON.parse(localStorage.getItem("simrep.results")!)).toMatchObject({ open: true, tab: "table" });
  });
});
