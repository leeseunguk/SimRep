import { expect, test, type Page } from "@playwright/test";
import { answered, startEinzel, step } from "./demo";

// WebGL2 fallback of the 3D view: navigator.gpu is removed before the app loads, so the renderer
// must select WebGL2 and draw the einzel template (electrodes, section) and, after the flight, its
// trajectories. Pixels are read back through the app's capture path.

test.beforeEach(({ page }) =>
  page.addInitScript(() => {
    localStorage.setItem("simrep.tour", "seen");
    delete (Navigator.prototype as unknown as { gpu?: unknown }).gpu;
  }),
);

type Frames = Record<string, { width: number; height: number; data: Uint8ClampedArray; bg: Uint8ClampedArray }>;

/**
 * Captures the 3D view through the app's own capture path (results export / report use it) into
 * `window.__frames[name]`, with the bare viewport background for counting drawn pixels.
 */
const capture = (page: Page, name: string) =>
  page.evaluate(async (name) => {
    // The dev server's module (same instance as the app's), not a bundled copy.
    const url = "/src/viewport/viewState.ts";
    const { captureViewport } = (await import(/* @vite-ignore */ url)) as typeof import("../src/viewport/viewState");
    const blob = await captureViewport();
    if (!blob) throw new Error("the 3D view is not mounted");
    const bmp = await createImageBitmap(blob);
    const { width, height } = bmp;
    const c = document.createElement("canvas");
    c.width = width;
    c.height = height;
    const ctx = c.getContext("2d")!;
    ctx.drawImage(bmp, 0, 0);
    const data = ctx.getImageData(0, 0, width, height).data;
    // Same background as Renderer.capture (radial gradient of the viewport tokens).
    const css = getComputedStyle(document.documentElement);
    const rx = 1.2 * width;
    const ry = 0.9 * height;
    ctx.clearRect(0, 0, width, height);
    ctx.save();
    ctx.translate(width / 2, height * 0.4);
    ctx.scale(1, ry / rx);
    const g = ctx.createRadialGradient(0, 0, 0, 0, 0, rx);
    g.addColorStop(0, css.getPropertyValue("--vp-bg-center").trim());
    g.addColorStop(1, css.getPropertyValue("--vp-bg-edge").trim());
    ctx.fillStyle = g;
    ctx.fillRect(-width, -height * (rx / ry), width * 2, height * 2 * (rx / ry));
    ctx.restore();
    const w = window as unknown as { __frames?: Frames };
    w.__frames ??= {};
    w.__frames[name] = { width, height, data, bg: ctx.getImageData(0, 0, width, height).data };
  }, name);

/** Share of pixels of a captured frame that differ from the background. */
const drawnShare = (page: Page, name: string) =>
  page.evaluate((name) => {
    const f = (window as unknown as { __frames: Frames }).__frames[name]!;
    let n = 0;
    for (let i = 0; i < f.data.length; i += 4)
      if (Math.abs(f.data[i]! - f.bg[i]!) + Math.abs(f.data[i + 1]! - f.bg[i + 1]!) + Math.abs(f.data[i + 2]! - f.bg[i + 2]!) > 12) n++;
    return n / (f.width * f.height);
  }, name);

/** Pixels (index y * width + x) where two captured frames differ, or null if their sizes differ. */
const changedPixels = (page: Page, a: string, b: string) =>
  page.evaluate(
    ([a, b]) => {
      const { __frames } = window as unknown as { __frames: Frames };
      const fa = __frames[a]!;
      const fb = __frames[b]!;
      if (fa.width !== fb.width || fa.height !== fb.height) return null;
      const out: number[] = [];
      for (let i = 0; i < fa.data.length; i += 4)
        if (Math.abs(fa.data[i]! - fb.data[i]!) + Math.abs(fa.data[i + 1]! - fb.data[i + 1]!) + Math.abs(fa.data[i + 2]! - fb.data[i + 2]!) > 24) out.push(i / 4);
      return { width: fa.width, pixels: out };
    },
    [a, b] as const,
  );

/** Captures `name` once the view has settled (camera fit, panel layout): two identical frames in a row. */
async function settled(page: Page, name: string) {
  await capture(page, name);
  await expect
    .poll(async () => {
      await capture(page, `${name}.next`);
      const d = await changedPixels(page, name, `${name}.next`);
      await capture(page, name);
      return d?.pixels.length ?? -1;
    })
    .toBe(0);
}

test("no WebGPU: the 3D view falls back to WebGL2 and draws the einzel lens and its trajectories", async ({ page }) => {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  page.on("console", (m) => {
    if (m.type() === "error") errors.push(m.text());
  });

  const job = await startEinzel(page);
  const status = page.getByTestId("status-renderer");
  await expect(status).toHaveText("Renderer: WebGL2");
  await expect(status).toHaveAttribute("data-renderer", "webgl2");
  await expect(page.getByText("The 3D view needs WebGPU or WebGL2")).toHaveCount(0);
  expect(await page.evaluate(() => "gpu" in navigator)).toBe(false);

  // The unrefined array: electrodes and the section are drawn.
  await settled(page, "before");
  const before = await drawnShare(page, "before");
  expect(before).toBeGreaterThan(0.05);

  while ((await step(page, job)) !== null);
  await answered(page, "fly", 1);
  await expect(page.getByText("40 ions flown")).toBeVisible();

  // Trajectories on vs off: the frames differ by the trajectory pixels.
  await settled(page, "on");
  const toggle = page.getByRole("switch", { name: "Show trajectories" });
  await expect(toggle).toHaveAttribute("aria-checked", "true");
  await toggle.click();
  await expect(toggle).toHaveAttribute("aria-checked", "false");
  await settled(page, "off");
  await toggle.click();
  await expect(toggle).toHaveAttribute("aria-checked", "true");
  await settled(page, "again");
  expect((await changedPixels(page, "on", "again"))?.pixels.length).toBe(0);
  const traj = (await changedPixels(page, "on", "off"))!;
  // 40 trajectories across the lens: thousands of pixels, not a blank or unchanged view.
  expect(traj.pixels.length).toBeGreaterThan(2000);
  // The refined lens is still drawn under the trajectories (the results panel shrinks the view).
  const after = await drawnShare(page, "on");
  expect(after).toBeGreaterThan(0.02);
  test.info().annotations.push({ type: "pixels", description: JSON.stringify({ before, after, trajectoryPixels: traj.pixels.length }) });

  // Ion picking on the fallback: a click on a drawn trajectory pixel (not under an overlay) opens that ion.
  const canvas = page.locator('section[aria-label="3D view"] canvas');
  const box = (await canvas.boundingBox())!;
  const target = await canvas.evaluate(
    (c, { pixels, width }) => {
      const rect = c.getBoundingClientRect();
      const scale = width / rect.width;
      for (let k = Math.floor(pixels.length / 2); k < pixels.length; k += 97) {
        const x = rect.left + ((pixels[k]! % width) + 0.5) / scale;
        const y = rect.top + (Math.floor(pixels[k]! / width) + 0.5) / scale;
        if (document.elementFromPoint(x, y) === c) return { x, y };
      }
      return null;
    },
    traj,
  );
  expect(box.width).toBeGreaterThan(0);
  expect(target).not.toBeNull();
  await page.mouse.click(target!.x, target!.y);
  await expect(page.locator('[data-tour="results-tab-ion"]')).toHaveAttribute("aria-selected", "true");

  expect(errors).toEqual([]);
});

test("neither WebGPU nor WebGL2: the fallback card names both reasons", async ({ page }) => {
  await page.addInitScript(() => {
    const get = HTMLCanvasElement.prototype.getContext;
    HTMLCanvasElement.prototype.getContext = function (this: HTMLCanvasElement, id: string, ...rest: unknown[]) {
      return id === "webgl2" ? null : (get as (...a: unknown[]) => unknown).call(this, id, ...rest);
    } as typeof get;
  });
  await startEinzel(page);
  await expect(page.getByRole("alert").getByText("The 3D view needs WebGPU or WebGL2")).toBeVisible();
  const status = page.getByTestId("status-renderer");
  await expect(status).toHaveText("No WebGPU or WebGL2");
  await expect(status).toHaveAttribute("title", /navigator\.gpu is undefined.*getContext\('webgl2'\) returned null/);
});
