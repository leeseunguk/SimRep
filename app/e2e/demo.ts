import { expect, type Page } from "@playwright/test";
import type { DemoLog } from "../src/demo/ipc";

// Drives demo mode (src/demo/ipc.ts) with manual refine replay: a recorded refine message is sent
// only when the test steps it, and commands are awaited by count, so no state depends on timing.

type DemoWindow = { __simrepDemo: DemoLog };

/** The calls, writes and refine messages the demo IPC layer recorded. */
export const demoLog = (page: Page) =>
  page.evaluate(() => {
    const { calls, writes, refine } = (window as unknown as DemoWindow).__simrepDemo;
    return { calls, writes, refine };
  });

/** Waits until `cmd` has been answered `count` times; its latest answer. */
export const answered = (page: Page, cmd: string, count = 1) =>
  page.evaluate(([c, n]) => (window as unknown as DemoWindow).__simrepDemo.answered(c, n), [cmd, count] as const);

/** Sends the next recorded message of refine `job`; its type, or null once the job has ended. */
export const step = (page: Page, job: number) => page.evaluate((j) => (window as unknown as DemoWindow).__simrepDemo.step(j), job);

/** The live convergence chart of a refine in progress. */
export const convergenceChart = (page: Page) => page.getByRole("img", { name: /^Convergence: relative residual/ });

/** Opens the einzel template in demo mode with manual replay; the refine job it starts. */
export async function startEinzel(page: Page): Promise<number> {
  await page.goto("/?replay=manual");
  const einzel = page.locator('[data-tour="template-einzel"]');
  await expect(einzel).toBeVisible();
  await einzel.click();
  return (await answered(page, "refine_start")) as number;
}
