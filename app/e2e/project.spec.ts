import { expect, test, type Page } from "@playwright/test";
import type { DemoLog } from "../src/demo/ipc";
import { answered, demoLog, startEinzel, step } from "./demo";

// Project format 2 against the demo IPC (src/demo/ipc.ts), whose files live in localStorage so a
// reload stands in for reopening the app: stored runs in X.simrep.d, loading them instead of
// flying, the run history and comparing two runs.

type DemoWindow = { __simrepDemo: DemoLog };
const PROJECT = "C:\\Users\\demo\\Documents\\SimRep\\einzel\\Einzel lens.simrep";
const files = (page: Page) => page.evaluate(() => (window as unknown as DemoWindow).__simrepDemo.files());
const fileText = (page: Page, path: string) => page.evaluate((p) => (window as unknown as DemoWindow).__simrepDemo.text(p), path);

/** Pixels of a chart canvas in a saturated colour (the heat map; axes and text are grey). */
const colouredPixels = (box: import("@playwright/test").Locator) =>
  box.locator("canvas").evaluate((c: HTMLCanvasElement) => {
    const d = c.getContext("2d")!.getImageData(0, 0, c.width, c.height).data;
    let n = 0;
    for (let i = 0; i < d.length; i += 4) if (d[i + 3]! > 0 && Math.max(d[i]!, d[i + 1]!, d[i + 2]!) - Math.min(d[i]!, d[i + 1]!, d[i + 2]!) > 60) n++;
    return n;
  });

test.beforeEach(({ page }) => page.addInitScript(() => localStorage.setItem("simrep.tour", "seen")));

/** Einzel template refined and flown once, then saved with Save (the mocked dialog answers PROJECT). */
async function savedProject(page: Page) {
  const job = await startEinzel(page);
  while ((await step(page, job)) !== null);
  await answered(page, "fly", 1);
  await showTab(page, "history");
  await expect(page.getByTestId("history-row")).toHaveCount(1);
  await page.getByRole("button", { name: "Save", exact: true }).click();
  await expect.poll(async () => Object.keys(await files(page)).filter((f) => f.endsWith("run.json")).length).toBe(1);
}

/** Shows results tab `id` (clicking the tab already shown would collapse the panel). */
async function showTab(page: Page, id: string) {
  const tab = page.locator(`[data-tour="results-tab-${id}"]`);
  const panel = page.locator('[data-tour="results"]');
  if ((await tab.getAttribute("aria-selected")) !== "true" || (await panel.getAttribute("data-open")) !== "true") await tab.click();
  await expect(tab).toHaveAttribute("aria-selected", "true");
  await expect(panel).toHaveAttribute("data-open", "true");
}

async function reopen(page: Page) {
  await page.goto("/?replay=manual");
  await page.getByTitle(PROJECT, { exact: true }).click();
  await answered(page, "open_pa", 1);
}

test("save stores the run in X.simrep.d; reopening loads it without flying; Fly again adds a run", async ({ page }) => {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  await savedProject(page);

  const stored = await files(page);
  const runFiles = Object.keys(stored).filter((f) => f.startsWith(`${PROJECT}.d\\runs\\`));
  expect(runFiles.map((f) => f.split("\\").pop()).sort()).toEqual(["results.bin", "run.json", "trajectories.bin"]);
  const doc = JSON.parse((await fileText(page, PROJECT))!) as { version: number; arrays: { run: string }[] };
  expect(doc.version).toBe(2);
  const runId = doc.arrays[0]!.run;
  expect(runId).toMatch(/^\d{8}-\d{9}-[0-9a-f]{4}$/);
  const record = JSON.parse((await fileText(page, `${PROJECT}.d\\runs\\${runId}\\run.json`))!) as Record<string, any>;
  expect(record.settings.inputs.pa.sha256).toHaveLength(64);
  expect(record.settings.inputs.voltages).toEqual({ "1": 0, "2": 80, "3": 0 });
  expect(record.metrics).toMatchObject({ ions: 40, arrivals: 40 });
  expect(record.plots.histogram.counts.reduce((a: number, b: number) => a + b, 0)).toBe(40);
  expect(stored[`${PROJECT}.d\\runs\\${runId}\\results.bin`]).toBe(16 + 40 * (32 + 8 + 24 + 4));

  await reopen(page);
  await expect(page.getByTestId("results-stored")).toHaveText("Loaded saved results");
  await showTab(page, "hist");
  // Shown from the folder: the TOF histogram is there and no ion was flown.
  await expect(page.getByRole("img", { name: /TOF/ }).first()).toBeVisible();
  expect((await demoLog(page)).calls.filter((c) => c === "fly")).toEqual([]);
  await showTab(page, "history");
  await expect(page.getByTestId("history-row")).toHaveCount(1);
  await expect(page.getByTestId("history-row").first()).toContainText("Shown");

  await page.getByTestId("results-fly-again").click();
  await answered(page, "fly", 1);
  await expect(page.getByTestId("results-stored")).toHaveCount(0);
  await expect(page.getByTestId("history-row")).toHaveCount(2);
  // Saved project: the new run is written right away.
  await expect.poll(async () => Object.keys(await files(page)).filter((f) => f.endsWith("run.json")).length).toBe(2);
  expect(errors).toEqual([]);
});

test("a changed array marks the saved run stale and flies again", async ({ page }) => {
  await savedProject(page);
  await page.evaluate((p) => (window as unknown as DemoWindow).__simrepDemo.setSha(p, "e".repeat(64)), PROJECT.replace("Einzel lens.simrep", "einzel.pa0"));
  await reopen(page);
  await answered(page, "fly", 1);
  await expect(page.getByTestId("results-stored")).toHaveCount(0);
  await showTab(page, "history");
  await expect(page.getByTestId("history-row").filter({ hasText: "Out of date" })).toHaveCount(1);
  await expect(page.getByTestId("history-row")).toHaveCount(2);
});

test("history: rename, note, pin, delete, and compare two runs with a settings diff", async ({ page }) => {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  await savedProject(page);
  // A second run with another precision (properties panel): live mode re-flies it, and the run is
  // recorded once the live flights settle.
  await page.getByRole("radio", { name: /Precise/ }).click();
  await answered(page, "fly", 2);
  const rows = page.getByTestId("history-row");
  await expect(rows).toHaveCount(2);
  await expect(rows.first()).toContainText("Run 2");

  // Rename (keyboard: Enter commits), note, pin.
  await rows.first().getByRole("button", { name: "Rename: Run 2" }).click();
  await page.getByRole("textbox", { name: "Rename" }).fill("Precise CPU");
  await page.keyboard.press("Enter");
  await expect(rows.first()).toContainText("Precise CPU");
  await rows.first().getByRole("button", { name: "Note: Precise CPU" }).click();
  await page.getByRole("textbox", { name: "Note" }).fill("f64 reference");
  await page.keyboard.press("Enter");
  await expect(rows.first().getByTestId("history-note")).toHaveText("Note: f64 reference");
  await expect(rows.first().getByTestId("history-subtitle")).toHaveText("CPU · f64 · Precise · 40 ions");
  await expect(rows.nth(1).getByTestId("history-subtitle")).toHaveText("GPU · f32 · Live · 40 ions");
  await rows.first().getByRole("button", { name: "Pin: Precise CPU" }).click();
  await expect(rows.first().getByRole("button", { name: "Unpin: Precise CPU" })).toBeVisible();
  const id = await rows.first().getAttribute("data-run");
  await expect.poll(async () => JSON.parse((await fileText(page, `${PROJECT}.d\\runs\\${id}\\run.json`)) ?? "{}").label).toBe("Precise CPU");

  // Select both, compare: overlaid histograms + detector maps, then the settings diff.
  const compare = page.getByTestId("history-compare");
  await expect(compare).toBeDisabled();
  await rows.nth(0).getByRole("checkbox").check();
  await rows.nth(1).getByRole("checkbox").check();
  await expect(compare).toBeEnabled();
  await compare.click();
  await expect(page.locator('[data-tour="results-tab-compare"]')).toHaveAttribute("aria-selected", "true");
  await expect(page.getByRole("img", { name: /^Compare: / })).toBeVisible();
  await expect(page.getByRole("img", { name: /^Detector map: / })).toHaveCount(2);
  // Both detector maps show their ions: binned into the window, and drawn as coloured pixels.
  const maps = page.getByTestId("compare-map");
  await expect(maps).toHaveCount(2);
  for (const map of await maps.all()) {
    expect(Number(await map.getAttribute("data-ions"))).toBe(40);
    expect(Number(await map.getAttribute("data-cells"))).toBeGreaterThan(0);
    await expect.poll(() => colouredPixels(map)).toBeGreaterThan(20);
  }
  // Run B is dashed; the legend says when both histograms are the same.
  // The demo's live and precise flights land the same ions in the same TOF bins: identical histograms.
  const hist = page.getByTestId("compare-hist");
  await expect(hist).toHaveAttribute("data-dashed", "true");
  await expect(hist).toHaveAttribute("data-identical", "true");
  await expect(page.getByTestId("compare-identical")).toHaveText("Both runs give identical results");
  await page.getByRole("radio", { name: /^Settings/ }).click();
  const diff = page.getByTestId("settings-diff");
  // Both runs side by side, differing rows first, keys in words with the raw key as tooltip.
  await expect(diff.locator("thead th")).toHaveText(["Setting", "Run 1", "Precise CPU", "Same or different"]);
  const precision = diff.locator('tr[data-key="inputs.precision"]');
  await expect(precision).toHaveAttribute("data-differs", "true");
  await expect(precision.getByRole("rowheader")).toHaveText("Requested mode");
  await expect(precision.getByRole("rowheader")).toHaveAttribute("title", "inputs.precision");
  await expect(precision.locator("td")).toHaveText(["live", "precise", "Differs"]);
  await expect(diff.locator('tr[data-key="executed.device"] td')).toHaveText(["gpu", "cpu", "Differs"]);
  await expect(diff.locator('tr[data-key="inputs.durationUs"] th')).toHaveText("Flight time limit (µs)");
  const flags = await diff.locator("tbody tr").evaluateAll((rows) => rows.map((r) => r.hasAttribute("data-differs")));
  expect(flags.indexOf(false)).toBe(flags.filter(Boolean).length);
  await page.getByTestId("compare-pair-close").click();

  // Delete the unpinned run: its folder goes.
  await showTab(page, "history");
  const other = await rows.nth(1).getAttribute("data-run");
  await rows.nth(1).getByRole("button", { name: /^Delete run: / }).click();
  await expect(rows).toHaveCount(1);
  await expect.poll(async () => Object.keys(await files(page)).some((f) => f.includes(other!))).toBe(false);
  expect(errors).toEqual([]);
});
