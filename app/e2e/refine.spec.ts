import { expect, test } from "@playwright/test";
import { answered, convergenceChart, demoLog, startEinzel, step } from "./demo";

// The refine channel contract the UI relies on (src/scene/simulation.ts onRefineMessage): every job
// ends with exactly one terminal message, `done` or `cancelled`, in channel order.

test("refine: a job runs to completion and ends with done", async ({ page }) => {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  const job = await startEinzel(page);

  const sent: string[] = [];
  for (let type = await step(page, job); type !== null; type = await step(page, job)) sent.push(type);
  expect(sent).toHaveLength(52);
  expect(sent[0]).toBe("start");
  expect(sent.at(-1)).toBe("done");
  expect(sent.filter((t) => t === "done" || t === "cancelled")).toEqual(["done"]);
  expect((await demoLog(page)).refine[job]).toEqual(sent);

  await expect(convergenceChart(page)).toBeHidden();
  await answered(page, "fly", 1);
  expect(errors).toEqual([]);
});

test("refine: cancelling after the first messages ends the job with one cancelled message", async ({ page }) => {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  const job = await startEinzel(page);

  expect(await step(page, job)).toBe("start");
  expect(await step(page, job)).toBe("progress");
  const chart = convergenceChart(page);
  await expect(chart).toBeVisible();

  // The template's refine runs behind the blocking busy card, so call what the Cancel buttons call.
  await page.evaluate(async () => {
    // The app's own modules from the Vite dev server (same instances the UI uses).
    const { useApp } = (await import("/src/state/store.ts" as string)) as typeof import("../src/state/store");
    const { cancelRefine } = (await import("/src/scene/simulation.ts" as string)) as typeof import("../src/scene/simulation");
    cancelRefine(useApp.getState().objects.find((o) => o.kind === "pa")!.id);
  });
  await answered(page, "refine_cancel");
  expect(await step(page, job)).toBe("cancelled");
  expect(await step(page, job)).toBeNull();
  expect((await demoLog(page)).refine[job]).toEqual(["start", "progress", "cancelled"]);

  // The UI finished the cancellation: the busy card and the progress are gone, nothing was flown.
  await expect(page.locator("[data-busy]")).toHaveCount(0);
  await expect(chart).toBeHidden();
  expect((await demoLog(page)).calls).not.toContain("fly");
  expect(errors).toEqual([]);
});
