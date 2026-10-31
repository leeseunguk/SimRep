import { mkdirSync } from "node:fs";
import { expect, test, type Page } from "@playwright/test";
import { answered, startEinzel, step } from "./demo";

// Responsive layout (src/state/layout.ts): phone (< 600), tablet (600-1023) and desktop (>= 1024).
// At each size the einzel template refines and flies, the panels stay reachable, nothing overflows
// the window, and screenshots are written to SHOTS for a visual check.

const SHOTS = process.env.SIMREP_SHOTS ?? "/tmp/simrep-mobile-shots";
mkdirSync(SHOTS, { recursive: true });

/** Everything a user can press or type in that is on screen now. */
const INTERACTIVE = 'button, a[href], input:not([type="hidden"]), select, textarea, [role="tab"], [role="radio"], [role="switch"], [role="menuitem"], [tabindex="0"]';

/** Page-level overflow and interactive elements outside the window (CSS px). */
const overflow = (page: Page) =>
  page.evaluate((sel) => {
    const w = window.innerWidth;
    const h = window.innerHeight;
    const out: string[] = [];
    for (const el of document.querySelectorAll<HTMLElement>(sel)) {
      const r = el.getBoundingClientRect();
      if (r.width === 0 || r.height === 0 || el.closest("[hidden], [inert]")) continue;
      const style = getComputedStyle(el);
      if (style.visibility === "hidden" || style.display === "none") continue;
      // Inside a scroll container, only what the container shows counts (the rest scrolls into view);
      // past the bottom of a vertical scroller is fine, past the window's sides never is.
      let clip = { left: 0, top: 0, right: w, bottom: h };
      let scrollsY = false;
      for (let p = el.parentElement; p; p = p.parentElement) {
        const ps = getComputedStyle(p);
        if (/(auto|scroll|hidden)/.test(ps.overflowX + ps.overflowY)) {
          const pr = p.getBoundingClientRect();
          clip = { left: Math.max(clip.left, pr.left), top: Math.max(clip.top, pr.top), right: Math.min(clip.right, pr.right), bottom: Math.min(clip.bottom, pr.bottom) };
        }
        if (/(auto|scroll)/.test(ps.overflowY) && p.scrollHeight > p.clientHeight) scrollsY = true;
      }
      const visible = r.right > clip.left && r.left < clip.right && r.bottom > clip.top && r.top < clip.bottom;
      if (!visible) continue;
      if (r.left < -0.5 || r.right > w + 0.5 || (!scrollsY && (r.top < -0.5 || r.bottom > h + 0.5))) {
        out.push(`${el.tagName.toLowerCase()}[${el.getAttribute("aria-label") ?? el.textContent?.trim().slice(0, 30)}] ${Math.round(r.left)},${Math.round(r.top)} ${Math.round(r.width)}x${Math.round(r.height)}`);
      }
    }
    return { scrollWidth: document.documentElement.scrollWidth, bodyScrollWidth: document.body.scrollWidth, innerWidth: w, outside: out };
  }, INTERACTIVE);

/** Visible buttons and tabs smaller than 44 px or whose label is cut off (Korean and English). */
const smallOrClipped = (page: Page, root: string) =>
  page.evaluate(
    ([rootSel, sel]) => {
      const small: string[] = [];
      const clipped: string[] = [];
      for (const scope of document.querySelectorAll(rootSel))
        for (const el of scope.querySelectorAll<HTMLElement>(sel)) {
          const r = el.getBoundingClientRect();
          if (r.width === 0 || r.height === 0) continue;
          const name = `${el.getAttribute("aria-label") ?? el.textContent?.trim().slice(0, 30)}`;
          if (r.height < 43.5 || r.width < 43.5) small.push(`${name} ${Math.round(r.width)}x${Math.round(r.height)}`);
          if (el.scrollWidth > el.clientWidth + 1) clipped.push(`${name} ${el.scrollWidth}>${el.clientWidth}`);
        }
      return { small, clipped };
    },
    [root, 'button, [role="tab"], [role="radio"]'] as const,
  );

async function expectNoOverflow(page: Page, what: string) {
  const o = await overflow(page);
  expect(o.scrollWidth, `${what}: document.scrollWidth`).toBeLessThanOrEqual(o.innerWidth);
  expect(o.bodyScrollWidth, `${what}: body.scrollWidth`).toBeLessThanOrEqual(o.innerWidth);
  expect(o.outside, `${what}: interactive elements outside the window`).toEqual([]);
}

const canvasWidth = (page: Page) => page.locator('section[aria-label="3D view"] canvas').evaluate((c) => c.getBoundingClientRect().width);

/** Refine the einzel template to the end; the template then flies once. */
async function einzel(page: Page) {
  const job = await startEinzel(page);
  while ((await step(page, job)) !== null);
  await answered(page, "fly", 1);
}


/** The app store (demo dev server module), read in the page. */
const appState = (page: Page) =>
  page.evaluate(async () => {
    const url = "/src/state/store.ts";
    const { useApp } = (await import(/* @vite-ignore */ url)) as typeof import("../src/state/store");
    const s = useApp.getState();
    return { selectedId: s.selectedId, home: s.home };
  });

/** Shows an error notification (the longest-lived tone) through the store. */
const notify = (page: Page) =>
  page.evaluate(async () => {
    const url = "/src/state/store.ts";
    const { useApp } = (await import(/* @vite-ignore */ url)) as typeof import("../src/state/store");
    useApp.getState().toast("error", { key: "toast.dismiss" });
  });

/** Width x height of every visible element matching `sel` (CSS px), with its accessible name. */
const boxes = (page: Page, sel: string) =>
  page.evaluate(
    (s) =>
      [...document.querySelectorAll<HTMLElement>(s)]
        .map((el) => ({ name: el.getAttribute("aria-label") ?? "", r: el.getBoundingClientRect() }))
        .filter(({ r }) => r.width > 0 && r.height > 0)
        .map(({ name, r }) => ({ name, w: r.width, h: r.height })),
    sel,
  );

/** The 44 px rule: switches and notification dismiss buttons. */
async function expectTouchTargets(page: Page, what: string) {
  const switches = await boxes(page, '[role="switch"]');
  expect(switches.length, `${what}: switches on screen`).toBeGreaterThan(0);
  await notify(page);
  const dismiss = page.getByRole("button", { name: "Dismiss" }).first();
  await expect(dismiss).toBeVisible();
  const all = [...switches, ...(await boxes(page, 'button[aria-label="Dismiss"]'))];
  expect(all.filter((b) => b.w < 43.5 || b.h < 43.5), `${what}: switches and dismiss buttons under 44 px`).toEqual([]);
  await dismiss.click();
}

/** Where keyboard focus is: the id of the focused element and of its nearest tab panel or drawer. */
const focused = (page: Page) =>
  page.evaluate(() => {
    const a = document.activeElement as HTMLElement | null;
    return { id: a?.id ?? "", testId: a?.dataset.testid ?? "", panel: a?.closest('[id^="panel-"]')?.id ?? null, inert: !!a?.closest("[inert]") };
  });

const SIZES = [
  { name: "390", width: 390, height: 844, minCanvas: 0.9 },
  { name: "768", width: 768, height: 1024, minCanvas: 0.5 },
  { name: "1280", width: 1280, height: 800, minCanvas: 0.3 },
] as const;

for (const size of SIZES) {
  test.describe(`${size.width}x${size.height}`, () => {
    test.use({ viewport: { width: size.width, height: size.height }, hasTouch: size.width < 1024 });

    for (const theme of ["light", "dark"] as const) {
      test(`start screen and einzel after fly, ${theme}`, async ({ page }) => {
        const errors: string[] = [];
        page.on("pageerror", (e) => errors.push(e.message));
        await page.addInitScript((th) => {
          localStorage.setItem("simrep.tour", "seen");
          localStorage.setItem("simrep.theme", th);
        }, theme);

        await page.goto("/?replay=manual");
        await expect(page.locator('[data-tour="template-einzel"]')).toBeVisible();
        await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
        await expectNoOverflow(page, "start");
        await page.screenshot({ path: `${SHOTS}/start-${size.name}-${theme}.png` });

        await einzel(page);
        await expect(page.getByText("40 ions flown").or(page.locator('[data-tour="results"]')).first()).toBeAttached();
        const width = await canvasWidth(page);
        expect(width / size.width, "canvas share of the window width").toBeGreaterThanOrEqual(size.minCanvas);
        await expectNoOverflow(page, "einzel");
        // Results visible next to the trajectories: the phone shows its Results tab, the others the panel.
        if (size.width < 600) await page.getByTestId("phone-tab-results").click();
        await expect(page.locator('[data-tour="results"]')).toBeVisible();
        expect(await canvasWidth(page)).toBeGreaterThanOrEqual(width - 1);
        await expectNoOverflow(page, "einzel results");
        await page.screenshot({ path: `${SHOTS}/einzel-${size.name}-${theme}.png` });
        expect(errors).toEqual([]);
      });
    }
  });
}

test.describe("phone 390x844", () => {
  test.use({ viewport: { width: 390, height: 844 }, hasTouch: true });
  test.beforeEach(({ page }) => page.addInitScript(() => localStorage.setItem("simrep.tour", "seen")));

  test("tabs reach every panel, fly from the Simulation tab, 44 px targets, Korean and English fit", async ({ page }) => {
    await einzel(page);
    const view = page.locator('section[aria-label="3D view"]');
    const box = (await view.boundingBox())!;
    expect(box.width).toBeGreaterThanOrEqual(390 * 0.9);
    expect(box.height).toBeGreaterThanOrEqual(844 * 0.45 - 1);

    // Scene tab: the scene tree; Simulation: the properties panel with Fly; Results: the plots.
    await page.getByTestId("phone-tab-scene").click();
    await expect(page.getByRole("tree", { name: "Scene" })).toBeVisible();
    await expectNoOverflow(page, "scene tab");
    await page.getByTestId("phone-tab-simulation").click();
    const fly = page.locator('[data-tour="fly"]');
    await fly.scrollIntoViewIfNeeded();
    await expect(fly).toBeEnabled();
    await fly.click();
    await answered(page, "fly", 2);
    await expectNoOverflow(page, "simulation tab");
    await page.getByTestId("phone-tab-results").click();
    await expect(page.getByRole("img", { name: /TOF/ })).toBeVisible();
    await expectNoOverflow(page, "results tab");

    for (const lang of ["en", "ko"] as const) {
      await page.getByTestId("topbar-more").click();
      await page.getByTestId("topbar-more-menu").getByRole("radio", { name: lang === "ko" ? "한국어" : "English" }).click();
      await expect(page.locator("html")).toHaveAttribute("lang", lang);
      await page.keyboard.press("Escape");
      await expect(page.getByTestId("topbar-more-menu")).toHaveCount(0);
      const chrome = await smallOrClipped(page, 'header, footer, [data-testid="phone-tabs"]');
      expect(chrome.small, `${lang}: top bar, tabs and status bar targets under 44 px`).toEqual([]);
      expect(chrome.clipped, `${lang}: clipped labels`).toEqual([]);
      await page.getByTestId("topbar-more").click();
      const menu = await smallOrClipped(page, '[data-testid="topbar-more-menu"]');
      expect(menu.small, `${lang}: More menu targets under 44 px`).toEqual([]);
      expect(menu.clipped, `${lang}: More menu clipped labels`).toEqual([]);
      await expectNoOverflow(page, `${lang} more menu`);
      await page.keyboard.press("Escape");
    }
  });

  test("tabs: tablist semantics, one tab stop, arrows and Home/End, Tab enters the shown panel", async ({ page }) => {
    await einzel(page);
    const tabs = page.getByRole("tablist", { name: "Panels" }).getByRole("tab");
    await expect(tabs).toHaveCount(3);
    for (const id of ["scene", "simulation", "results"] as const) {
      const tab = page.getByTestId(`phone-tab-${id}`);
      await expect(tab).toHaveAttribute("id", `phone-tab-${id}`);
      await expect(tab).toHaveAttribute("aria-controls", `panel-${id}`);
      const panel = page.locator(`#panel-${id}`);
      await expect(panel).toHaveAttribute("role", "tabpanel");
      await expect(panel).toHaveAttribute("aria-labelledby", `phone-tab-${id}`);
    }
    const selected = async (id: string) => {
      await expect(page.getByTestId(`phone-tab-${id}`)).toHaveAttribute("aria-selected", "true");
      await expect(page.getByTestId(`phone-tab-${id}`)).toBeFocused();
      await expect(page.locator('[role="tab"][tabindex="0"]')).toHaveCount(1);
      await expect(page.locator('[role="tab"][tabindex="0"]')).toHaveAttribute("id", `phone-tab-${id}`);
      await expect(page.locator(`#panel-${id}`)).toBeVisible();
    };
    await expect(page.locator('[role="tab"][tabindex="0"]')).toHaveAttribute("id", "phone-tab-simulation");
    await page.getByTestId("phone-tab-simulation").focus();
    await page.keyboard.press("ArrowRight");
    await selected("results");
    await page.keyboard.press("ArrowRight");
    await selected("scene");
    await page.keyboard.press("ArrowLeft");
    await selected("results");
    await page.keyboard.press("Home");
    await selected("scene");
    await page.keyboard.press("End");
    await selected("results");
    await page.keyboard.press("ArrowLeft");
    await selected("simulation");

    // Tab from the selected tab goes into its panel (the panel is next in the DOM, as on screen).
    for (const id of ["scene", "simulation", "results"] as const) {
      await page.getByTestId(`phone-tab-${id}`).click();
      await page.getByTestId(`phone-tab-${id}`).focus();
      await page.keyboard.press("Tab");
      expect((await focused(page)).panel, `Tab from the ${id} tab`).toBe(`panel-${id}`);
    }
  });

  test("start screen over the Results tab: results, tabs and panels are inert", async ({ page }) => {
    await einzel(page);
    await page.getByTestId("phone-tab-results").click();
    await expect(page.locator('[data-tour="results"]')).toBeVisible();
    await page.locator('[data-tour="home"]').click();
    await expect(page.locator('[data-tour="template-einzel"]')).toBeVisible();
    expect((await appState(page)).home).toBe(true);
    // Every results control sits under an inert boundary ...
    const exposed = await page.evaluate(() =>
      [...document.querySelectorAll<HTMLElement>('#panel-results button, #panel-results [tabindex], [data-testid="phone-tabs"] button')].filter((el) => !el.closest("[inert]")).length,
    );
    expect(exposed, "results and tab controls outside an inert boundary").toBe(0);
    expect(await page.locator('[data-tour="results-tab-hist"]').count()).toBeGreaterThan(0);
    // ... and keyboard traversal never reaches the workspace behind the start screen.
    await page.locator('[data-tour="template-einzel"]').focus();
    for (let i = 0; i < 60; i++) {
      await page.keyboard.press("Tab");
      const f = await focused(page);
      expect(f.panel, `Tab ${i + 1} reached a covered panel`).toBeNull();
      expect(f.inert).toBe(false);
    }
  });

  test("Escape on the More menu closes it, keeps the selection, focus back on More", async ({ page }) => {
    await einzel(page);
    const before = (await appState(page)).selectedId;
    expect(before, "an object is selected after the einzel flight").not.toBeNull();
    await page.getByTestId("topbar-more").click();
    await expect(page.getByTestId("topbar-more-menu")).toBeVisible();
    await page.keyboard.press("Escape");
    await expect(page.getByTestId("topbar-more-menu")).toHaveCount(0);
    await expect(page.getByTestId("topbar-more")).toBeFocused();
    expect((await appState(page)).selectedId).toBe(before);
    // With no menu open, Escape is still the workspace's: it clears the selection.
    await page.keyboard.press("Escape");
    expect((await appState(page)).selectedId).toBeNull();
  });

  test("44 px targets: switches in the Simulation tab and notification dismiss", async ({ page }) => {
    await einzel(page);
    await page.getByTestId("phone-tab-simulation").click();
    await expectTouchTargets(page, "phone");
  });

  test("dialogs fit: AI accounts, report, shortcuts, export and the import wizard", async ({ page }) => {
    await einzel(page);
    const more = async (name: string | RegExp) => {
      await page.getByTestId("topbar-more").click();
      await page.getByTestId("topbar-more-menu").getByRole("button", { name }).click();
    };
    const fits = async (name: string) => {
      const d = page.getByRole("dialog", { name });
      await expect(d).toBeVisible();
      const b = (await d.boundingBox())!;
      expect(b.x).toBeGreaterThanOrEqual(0);
      expect(b.x + b.width).toBeLessThanOrEqual(390);
      await expectNoOverflow(page, name);
      await page.screenshot({ path: `${SHOTS}/dialog-${name.replace(/\W+/g, "-").toLowerCase()}-390.png` });
      await page.keyboard.press("Escape");
      await expect(d).toHaveCount(0);
    };
    await more("AI accounts");
    await fits("AI accounts");
    await more(/^Report/);
    await fits("Export report");
    await more("Keyboard shortcuts");
    await fits("Keyboard shortcuts");
    await more(/^Data files/);
    await fits("Export");

    // The import wizard opens in the right column, i.e. the Simulation tab (the demo cannot read
    // geometry, so it shows its error state; the layout is what is checked).
    await page.evaluate(async () => {
      const url = "/src/scene/importWizard.ts";
      const { openWizard } = (await import(/* @vite-ignore */ url)) as typeof import("../src/scene/importWizard");
      void openWizard({ path: "C:\\demo\\lens.gem", name: "lens.gem", size: 1000 });
    });
    const wizard = page.getByRole("complementary", { name: "Import geometry" });
    await expect(wizard).toBeVisible();
    await expect(page.getByTestId("phone-tab-simulation")).toHaveAttribute("aria-selected", "true");
    await expectNoOverflow(page, "import wizard");
    await page.screenshot({ path: `${SHOTS}/wizard-390.png` });
  });

  test("touch: one finger orbits, two fingers pinch-zoom", async ({ page }) => {
    await einzel(page);
    const canvas = page.locator('section[aria-label="3D view"] canvas');
    const b = (await canvas.boundingBox())!;
    const cx = b.x + b.width / 2;
    const cy = b.y + b.height / 2;
    const camera = () =>
      page.evaluate(async () => {
        const url = "/src/viewport/viewState.ts";
        const { cameraState } = (await import(/* @vite-ignore */ url)) as typeof import("../src/viewport/viewState");
        return cameraState()!;
      });
    // Touch pointer events through CDP (Input.dispatchTouchEvent): real touch input to the canvas.
    const cdp = await page.context().newCDPSession(page);
    const touch = (type: "touchStart" | "touchMove" | "touchEnd", points: { x: number; y: number }[]) =>
      cdp.send("Input.dispatchTouchEvent", { type, touchPoints: points.map((p, id) => ({ ...p, id })) });

    const before = await camera();
    await touch("touchStart", [{ x: cx, y: cy }]);
    for (let i = 1; i <= 5; i++) await touch("touchMove", [{ x: cx + i * 20, y: cy }]);
    await touch("touchEnd", []);
    await expect.poll(async () => (await camera()).yaw).not.toBeCloseTo(before.yaw, 3);
    const orbited = await camera();
    expect(orbited.distance).toBeCloseTo(before.distance, 6);

    await touch("touchStart", [{ x: cx - 30, y: cy }, { x: cx + 30, y: cy }]);
    for (let i = 1; i <= 5; i++) await touch("touchMove", [{ x: cx - 30 - i * 15, y: cy }, { x: cx + 30 + i * 15, y: cy }]);
    await touch("touchEnd", []);
    await expect.poll(async () => (await camera()).distance).toBeLessThan(orbited.distance * 0.6);
    expect((await camera()).yaw).toBeCloseTo(orbited.yaw, 6);
  });
});

test.describe("tablet 768x1024", () => {
  test.use({ viewport: { width: 768, height: 1024 }, hasTouch: true });
  test.beforeEach(({ page }) => page.addInitScript(() => localStorage.setItem("simrep.tour", "seen")));

  test("drawers open over the 3D view, which keeps at least half of the width", async ({ page }) => {
    await einzel(page);
    const full = await canvasWidth(page);
    expect(full).toBeGreaterThanOrEqual(768 * 0.9);
    for (const id of ["scene", "simulation"] as const) {
      await page.getByTestId(`drawer-${id}`).click();
      const panel = id === "scene" ? page.getByRole("tree", { name: "Scene" }) : page.locator('[data-tour="fly"]');
      await expect(panel).toBeVisible();
      // The drawer overlays the view (the canvas keeps its size) and covers at most half of it.
      expect(await canvasWidth(page)).toBeCloseTo(full, 0);
      const drawer = (await panel.evaluate((el) => el.closest("aside")!.getBoundingClientRect().width))!;
      expect(drawer).toBeLessThanOrEqual(768 / 2);
      await expectNoOverflow(page, `${id} drawer`);
      const chrome = await smallOrClipped(page, "header");
      expect(chrome.small).toEqual([]);
      expect(chrome.clipped).toEqual([]);
      await page.screenshot({ path: `${SHOTS}/drawer-${id}-768.png` });
    }
    // Fly from the drawer.
    await page.locator('[data-tour="fly"]').click();
    await answered(page, "fly", 2);
    await page.getByTestId("drawer-simulation").click();
    await expect(page.locator('[data-tour="fly"]')).toBeHidden();
  });
  test("drawer toggles: expanded state and controlled panel, Escape closes and focus returns", async ({ page }) => {
    await einzel(page);
    for (const id of ["scene", "simulation"] as const) {
      const toggle = page.getByTestId(`drawer-${id}`);
      await expect(toggle).toHaveAttribute("aria-controls", `panel-${id}`);
      await expect(toggle).toHaveAttribute("aria-expanded", "false");
      await toggle.click();
      await expect(toggle).toHaveAttribute("aria-expanded", "true");
      await expect(page.locator(`#panel-${id}`)).toBeVisible();
      // Into the drawer, then Escape: closed, focus on its toggle, selection untouched.
      const before = (await appState(page)).selectedId;
      await page.locator(`#panel-${id}`).locator("button:visible").first().focus();
      expect((await focused(page)).panel).toBe(`panel-${id}`);
      await page.keyboard.press("Escape");
      await expect(toggle).toHaveAttribute("aria-expanded", "false");
      await expect(page.locator(`#panel-${id}`)).toBeHidden();
      await expect(toggle).toBeFocused();
      expect((await appState(page)).selectedId).toBe(before);
    }
  });

  test("Escape in a drawer field: an unchanged field closes the drawer, an edit is cancelled first", async ({ page }) => {
    await einzel(page);
    const toggle = page.getByTestId("drawer-simulation");
    const name = page.locator("#panel-simulation #prop-name");
    const before = (await appState(page)).selectedId;
    // Unchanged: nothing to cancel, so the first Escape closes the drawer and focus returns to its toggle.
    await toggle.click();
    await expect(toggle).toHaveAttribute("aria-expanded", "true");
    await name.focus();
    await page.keyboard.press("Escape");
    await expect(toggle).toHaveAttribute("aria-expanded", "false");
    await expect(toggle).toBeFocused();
    // Edited: the first Escape restores the text and keeps the drawer, the second closes it.
    await toggle.click();
    const original = await name.inputValue();
    await name.fill(`${original} edited`);
    await page.keyboard.press("Escape");
    await expect(name).toHaveValue(original);
    await expect(name).toBeFocused();
    await expect(toggle).toHaveAttribute("aria-expanded", "true");
    await page.keyboard.press("Escape");
    await expect(toggle).toHaveAttribute("aria-expanded", "false");
    await expect(toggle).toBeFocused();
    expect((await appState(page)).selectedId).toBe(before);
  });

  test("Escape on a closed select in a drawer closes the drawer", async ({ page }) => {
    await einzel(page);
    const toggle = page.getByTestId("drawer-simulation");
    // The assistant (in the More menu at this width) opens in the Simulation drawer; its approval mode is a native select.
    await page.getByTestId("topbar-more").click();
    await page.getByTestId("topbar-more-menu").getByTestId("ai-toggle").click();
    await expect(toggle).toHaveAttribute("aria-expanded", "true");
    const select = page.locator('#panel-simulation [data-testid="ai-approval-mode"]');
    await select.focus();
    await page.keyboard.press("Escape");
    await expect(toggle).toHaveAttribute("aria-expanded", "false");
    await expect(toggle).toBeFocused();
    await expect(select).toHaveValue("ask");
  });

  test("44 px targets: switches in the properties drawer and notification dismiss", async ({ page }) => {
    await einzel(page);
    await page.getByTestId("drawer-simulation").click();
    await expectTouchTargets(page, "tablet");
  });
});
