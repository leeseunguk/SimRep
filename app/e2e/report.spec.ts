import { expect, test, type Page } from "@playwright/test";
import { answered, demoLog, startEinzel, step } from "./demo";

// Report export in demo mode: top bar Export menu and command palette -> report dialog -> the
// mocked save dialog answers a path -> the written HTML is checked (self-contained, language,
// metrics of the recorded flights, run comparison with a settings diff).

async function flownEinzel(page: Page) {
  const job = await startEinzel(page);
  while ((await step(page, job)) !== null);
  await answered(page, "fly", 1);
}

/** The newest write_bytes body (the report). */
async function lastWrite(page: Page, count: number) {
  await answered(page, "write_bytes", count);
  const { writes } = await demoLog(page);
  return writes[count - 1]!;
}

function expectSelfContained(html: string) {
  expect(html.startsWith("<!DOCTYPE html>")).toBe(true);
  expect(html).not.toMatch(/https?:\/\//i);
  expect(html).not.toMatch(/<script|<link\b|@import/i);
  const imgs = [...html.matchAll(/<img\b[^>]*\bsrc="([^"]*)"/g)].map((m) => m[1]!);
  expect(imgs.length).toBeGreaterThanOrEqual(3);
  for (const src of imgs) expect(src).toMatch(/^data:image\/(png|svg\+xml);base64,/);
}

test("top bar Export -> Report: self-contained HTML with the current flight, then two runs compared", async ({ page }) => {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  await flownEinzel(page);

  // The first flight is in the run history (in memory: the project is untitled).
  await page.getByTestId("topbar-export").getByRole("button", { name: "Export" }).click();
  await page.getByRole("menu").getByRole("menuitem", { name: /Report/ }).click();
  const dialog = page.getByTestId("report-dialog");
  await expect(dialog).toBeVisible();
  await expect(dialog.getByRole("radio", { name: "PDF" })).toBeVisible();
  await dialog.getByTestId("report-run").click();
  const first = await lastWrite(page, 1);
  expect(first.path).toMatch(/einzel-report\.html$/);
  const html = first.text;
  expectSelfContained(html);
  expect(html).toContain('<html lang="en">');
  expect(html).toContain("<h1>Simulation report</h1>");
  // Geometry of the einzel template and its electrode names.
  expect(html).toContain('<td class="num">201 × 41 × 1</td>');
  expect(html).toMatch(/<td>Focus<\/td><td class="num">2<\/td>/);
  // Metrics of the recorded live flight (40 ions, all on the +X edge, GPU compute 37.96 ms).
  expect(html).toContain('<tr><td>Ions flown</td><td class="num">40</td></tr>');
  expect(html).toContain('<tr><td>End states: +X edge</td><td class="num">40 (100.0%)</td></tr>');
  expect(html).toContain('<tr><td>TOF mean, µs</td><td class="num">8.263052</td></tr>');
  expect(html).toContain('<tr><td>Compute time, ms (compute_ms)</td><td class="num">38.0</td></tr>');
  expect(html).toContain("Compute only");
  // Headless Chromium has no WebGPU adapter: the WebGL2 fallback draws the view, so its snapshot is in.
  expect(html).not.toContain("The 3D view snapshot is not included");
  expect(html).toMatch(/<img src="data:image\/png;base64,[^"]+" alt="3D view snapshot">/);
  await expect(dialog.getByText("Exported")).toBeVisible();
  await dialog.getByRole("button", { name: "Close" }).first().click();
  await expect(dialog).toBeHidden();

  // A second (precise) flight, then compare both runs from the command palette.
  await page.getByRole("radio", { name: /Precise/ }).first().click();
  await page.locator('[data-tour="fly"]').click();
  await answered(page, "fly", 2);
  await page.keyboard.press("Control+k");
  const palette = page.getByRole("combobox", { name: "Command palette" });
  await expect(palette).toBeFocused();
  await palette.fill("report");
  await page.keyboard.press("Enter");
  await expect(dialog).toBeVisible();
  const runs = dialog.getByRole("checkbox");
  await expect(runs).toHaveCount(2);
  await runs.nth(0).check();
  await runs.nth(1).check();
  await dialog.getByTestId("report-run").click();
  const second = (await lastWrite(page, 2)).text;
  expectSelfContained(second);
  expect(second).toContain("Run comparison");
  expect(second).toMatch(/<th class="num">Run 2<\/th><th class="num">Run 1<\/th>/);
  expect(second).toMatch(/<td><code>inputs\.precision<\/code><\/td><td>precise<\/td><td>live<\/td>/);
  expect(second).toContain("TOF histograms of the compared runs");

  expect(errors).toEqual([]);
});

test("report follows the UI language (Korean)", async ({ page }) => {
  await flownEinzel(page);
  await page.getByRole("radiogroup", { name: "Language" }).getByRole("radio", { name: "한국어" }).click();
  await page.getByTestId("topbar-export").getByRole("button", { name: "내보내기" }).click();
  await page.getByRole("menu").getByRole("menuitem", { name: /보고서/ }).click();
  await page.getByTestId("report-run").click();
  const html = (await lastWrite(page, 1)).text;
  expectSelfContained(html);
  expect(html).toContain('<html lang="ko">');
  expect(html).toContain("<h1>시뮬레이션 보고서</h1>");
  expect(html).toContain("주요 지표");
  expect(html).toContain("계산만 포함");
});
