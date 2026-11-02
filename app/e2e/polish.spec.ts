import { mkdirSync } from "node:fs";
import { expect, test, type Page } from "@playwright/test";
import { answered, startEinzel, step } from "./demo";

// Browser-build polish: the language switch stays on one line, the phone status bar shows only
// whole items, phone notifications and viewport overlays leave the 3D view visible, the first
// flight frames the scene on a phone, and the AI accounts dialog of a host without the assistant
// offers the desktop app instead of token wording. Screenshots go to SHOTS.

const SHOTS = process.env.SIMREP_POLISH_SHOTS ?? "/tmp/simrep-polish-shots";
mkdirSync(SHOTS, { recursive: true });

/** Starts the app under a browser host (e2e/webHost.tsx) instead of plain demo mode. */
const useWebHost = (page: Page) =>
  page.route(
    (url) => url.pathname === "/",
    async (route) => {
      const response = await route.fetch();
      const body = (await response.text()).replace('src="/src/main.tsx"', 'src="/e2e/webHost.tsx"');
      expect(body, "index.html loads the web host").toContain("/e2e/webHost.tsx");
      await route.fulfill({ response, body });
    },
  );

const setup = (page: Page, lang: "en" | "ko") =>
  page.addInitScript((l) => {
    localStorage.setItem("simrep.tour", "seen");
    localStorage.setItem("simrep.lang", l);
  }, lang);

/** Refine the einzel template to the end; the template then flies once. */
async function einzel(page: Page) {
  const job = await startEinzel(page);
  while ((await step(page, job)) !== null);
  await answered(page, "fly", 1);
}

/** Resolves after two animation frames (work scheduled for the next frame has run). */
const frames = (page: Page) => page.evaluate(() => new Promise<void>((r) => requestAnimationFrame(() => requestAnimationFrame(() => r()))));

/** Line boxes of an element's text (1 = no wrap) and its height. */
const lines = (page: Page, sel: string) =>
  page.locator(sel).evaluateAll((els) =>
    els.map((el) => {
      const range = document.createRange();
      range.selectNodeContents(el);
      const tops = new Set([...range.getClientRects()].filter((r) => r.width > 0).map((r) => Math.round(r.top)));
      return { text: el.textContent ?? "", lines: tops.size, height: el.getBoundingClientRect().height };
    }),
  );

const LANG_RADIOS = 'header [role="radiogroup"][aria-label="Language"] [role="radio"], header [role="radiogroup"][aria-label="언어"] [role="radio"]';

test.describe("top bar language switch", () => {
  for (const lang of ["en", "ko"] as const) {
    test(`one line, no shift at 1024 / 1100 / 1280 / 1920, ${lang}`, async ({ page }) => {
      await setup(page, lang);
      const sizes = [1024, 1100, 1280, 1920];
      const heights: number[] = [];
      for (const width of sizes) {
        await page.setViewportSize({ width, height: 800 });
        if (width === sizes[0]) await einzel(page);
        const radios = await lines(page, LANG_RADIOS);
        expect(radios.map((r) => r.text)).toEqual(["한국어", "English"]);
        // Same height as the icon-only theme segments: a single line.
        const theme = await page.locator('header [role="radiogroup"]').nth(1).locator('[role="radio"]').first().boundingBox();
        for (const r of radios) {
          expect(r.lines, `${width}: ${r.text} line count`).toBe(1);
          expect(r.height, `${width}: ${r.text} height`).toBeCloseTo(theme!.height, 0);
        }
        const bar = await page.locator("header").first().evaluate((h) => {
          const box = h.getBoundingClientRect();
          const out = [...h.querySelectorAll<HTMLElement>("button, [role=radio]")]
            .map((el) => el.getBoundingClientRect())
            .filter((r) => r.width > 0 && (r.left < box.left - 0.5 || r.right > box.right + 0.5 || r.top < box.top - 0.5 || r.bottom > box.bottom + 0.5)).length;
          return { height: box.height, scroll: h.scrollWidth, client: h.clientWidth, out };
        });
        expect(bar.scroll, `${width}: header overflows`).toBeLessThanOrEqual(bar.client);
        expect(bar.out, `${width}: controls outside the header`).toBe(0);
        heights.push(bar.height);
        if (width === 1280 && lang === "en") await page.locator("header").first().screenshot({ path: `${SHOTS}/topbar-1280-en.png` });
      }
      expect(new Set(heights).size, `header heights ${heights.join(", ")}`).toBe(1);
    });
  }
});

// The project name in the desktop top bar: under 1366 px the tagline goes, so typical names show
// in full; a name that still does not fit ends in an ellipsis and keeps its full text as the tooltip.
test.describe("top bar project name", () => {
  /** Renames the open project in the store (name: a file stem; nameKey: a template title; neither: untitled). */
  const rename = (page: Page, meta: { name: string | null; nameKey: string | null }) =>
    page.evaluate(async (m) => {
      const url = "/src/state/store.ts";
      const { useApp } = (await import(/* @vite-ignore */ url)) as typeof import("../src/state/store");
      const project = useApp.getState().project;
      useApp.setState({ project: { ...project, name: m.name, nameKey: m.nameKey as never } });
    }, meta);
  const measure = (page: Page) =>
    page.getByTestId("project-name").evaluate((el) => ({ text: el.textContent ?? "", scroll: el.scrollWidth, client: el.clientWidth, title: el.getAttribute("title") }));
  const header = (page: Page) =>
    page.locator("header").first().evaluate((h) => {
      const box = h.getBoundingClientRect();
      const out = [...h.querySelectorAll<HTMLElement>("button, [role=radio]")]
        .map((el) => el.getBoundingClientRect())
        .filter((r) => r.width > 0 && (r.left < box.left - 0.5 || r.right > box.right + 0.5 || r.top < box.top - 0.5 || r.bottom > box.bottom + 0.5)).length;
      return { scroll: h.scrollWidth, client: h.clientWidth, out };
    });

  for (const lang of ["en", "ko"] as const) {
    test(`typical names in full at 1024 / 1100 / 1280 / 1920, ${lang}`, async ({ page }) => {
      await setup(page, lang);
      const names = [
        { meta: { name: null, nameKey: "template.einzel.title" }, text: lang === "en" ? "Einzel lens" : "아인젤 렌즈" },
        { meta: { name: "Quadrupole", nameKey: null }, text: "Quadrupole" },
        { meta: { name: null, nameKey: null }, text: lang === "en" ? "Untitled project" : "제목 없는 프로젝트" },
      ];
      for (const width of [1024, 1100, 1280, 1920]) {
        await page.setViewportSize({ width, height: 800 });
        if (width === 1024) await einzel(page);
        // The tagline only from 1366 px; the logo and the name always.
        await expect(page.getByTestId("topbar-tagline")).toBeVisible({ visible: width >= 1366 });
        await expect(page.locator("header").first().getByText("SimRep", { exact: true })).toBeVisible();
        // Under 1280 px the labelled actions show their icon only, keeping their names; nothing is squeezed.
        for (const name of lang === "en" ? ["Open", "Save", "Export", "Assistant"] : ["열기", "저장", "내보내기", "어시스턴트"])
          await expect(page.locator("header").first().getByRole("button", { name, exact: true })).toBeVisible();
        const home = (await page.locator('header [data-tour="home"]').boundingBox())!;
        expect(home.width, `${width}: start screen button squeezed`).toBeCloseTo(home.height, 0);
        for (const { meta, text } of names) {
          await rename(page, meta);
          await expect(page.getByTestId("project-name")).toHaveText(text);
          const m = await measure(page);
          expect(m.scroll, `${width}: "${text}" truncated (${m.scroll} > ${m.client})`).toBeLessThanOrEqual(m.client);
          for (const r of await lines(page, LANG_RADIOS)) expect(r.lines, `${width}: ${r.text} line count`).toBe(1);
          const bar = await header(page);
          expect(bar.scroll, `${width}: header overflows`).toBeLessThanOrEqual(bar.client);
          expect(bar.out, `${width}: controls outside the header`).toBe(0);
        }
        await rename(page, names[0]!.meta);
        if (width === 1100 || width === 1280) await page.locator("header").first().screenshot({ path: `${SHOTS}/topbar-${width}-${lang}.png` });
      }
    });

    test(`a name too long to fit ends in an ellipsis with the full name as tooltip, 1024, ${lang}`, async ({ page }) => {
      await setup(page, lang);
      await page.setViewportSize({ width: 1024, height: 800 });
      await einzel(page);
      const long = "Einzel lens with three cylinders and a very long descriptive project name";
      await rename(page, { name: long, nameKey: null });
      await expect(page.getByTestId("project-name")).toHaveText(long);
      const m = await measure(page);
      expect(m.scroll, "long name is clipped").toBeGreaterThan(m.client);
      expect(m.title).toBe(long);
      expect(await page.getByTestId("project-name").evaluate((el) => getComputedStyle(el).textOverflow)).toBe("ellipsis");
      for (const r of await lines(page, LANG_RADIOS)) expect(r.lines, `${r.text} line count`).toBe(1);
      const bar = await header(page);
      expect(bar.scroll, "header overflows").toBeLessThanOrEqual(bar.client);
      expect(bar.out, "controls outside the header").toBe(0);
    });
  }
});

test.describe("phone 390x844", () => {
  test.use({ viewport: { width: 390, height: 844 }, hasTouch: true, reducedMotion: "reduce" });

  for (const host of ["web host", "desktop"] as const)
    for (const lang of ["en", "ko"] as const)
      test(`status bar shows whole items only, ${host}, ${lang}`, async ({ page }) => {
        await setup(page, lang);
        if (host === "web host") await useWebHost(page);
        await einzel(page);
        const footer = page.locator("footer");
        await expect(footer.getByRole("button", { name: lang === "en" ? /Results/ : /결과/ })).toBeVisible();
        const items = await footer.evaluate((f) =>
          [...f.querySelectorAll<HTMLElement>(":scope > *, :scope > * > *")]
            .filter((el) => {
              const r = el.getBoundingClientRect();
              return r.width > 1 && r.height > 1 && getComputedStyle(el).display !== "contents";
            })
            .map((el) => ({ text: el.textContent ?? "", scroll: el.scrollWidth, client: el.clientWidth, tag: el.tagName, h: el.getBoundingClientRect().height })),
        );
        expect(items.length).toBeGreaterThan(0);
        expect(items.filter((i) => i.scroll > i.client), "cut-off status items").toEqual([]);
        expect(items.filter((i) => i.tag === "BUTTON" && i.h < 43.5), "status buttons under 44 px").toEqual([]);
        const bar = await footer.evaluate((f) => ({ scroll: f.scrollWidth, client: f.clientWidth }));
        expect(bar.scroll).toBeLessThanOrEqual(bar.client);
        // The renderer in its compact form: the dot and the API name.
        await expect(page.getByTestId("status-renderer")).toHaveText(/WebGL2/);
        if (host === "web host") await expect(footer.getByText("CPU fallback · 4 threads")).toBeHidden();
      });

  for (const lang of ["en", "ko"] as const)
    test(`einzel: toast off the 3D view, compact overlays, fitted after the first fly, ${lang}`, async ({ page }) => {
      await setup(page, lang);
      await useWebHost(page);
      await einzel(page);
      const view = page.locator("main section").first();
      await expect(page.getByTestId("phone-tab-results")).toBeEnabled();
      await frames(page);
      const camera = () =>
        page.evaluate(async () => {
          const url = "/src/viewport/viewState.ts";
          const { cameraState } = (await import(/* @vite-ignore */ url)) as typeof import("../src/viewport/viewState");
          return cameraState()!;
        });
      const afterFly = await camera();

      // The template notification: under the 3D view, nowhere over the canvas.
      await page.evaluate(async () => {
        const url = "/src/state/store.ts";
        const { useApp } = (await import(/* @vite-ignore */ url)) as typeof import("../src/state/store");
        // One notification, shown now (the template's own one may already have timed out).
        for (const t of useApp.getState().toasts) useApp.getState().dismissToast(t.id);
        useApp.getState().toast("success", { key: "toast.templateReady.title", params: { name: "Einzel lens" } }, { key: "toast.templateReady.body", params: { ms: 1200 } });
      });
      const toast = page.locator('[aria-live="polite"] > [role="status"]');
      await expect(toast).toHaveCount(1);
      await expect(toast).toBeVisible();
      const t = (await toast.boundingBox())!;
      const v = (await view.boundingBox())!;
      const center = { x: v.x + v.width / 2, y: v.y + v.height / 2 };
      expect(center.x >= t.x && center.x <= t.x + t.width && center.y >= t.y && center.y <= t.y + t.height, "toast covers the canvas center").toBe(false);
      expect(t.y, "toast top below the 3D view").toBeGreaterThanOrEqual(v.y + v.height);
      const tabs = (await page.getByTestId("phone-tabs").boundingBox())!;
      const status = (await page.locator("footer").boundingBox())!;
      expect(t.y).toBeGreaterThanOrEqual(tabs.y + tabs.height);
      expect(t.y + t.height).toBeLessThanOrEqual(status.y);
      await page.screenshot({ path: `${SHOTS}/phone-einzel-390-${lang}.png` });

      // Overlay controls: one row of toolbar, a collapsed legend and the axes, at most 25% of the view.
      const overlays = await view.evaluate((section) => {
        const box = section.getBoundingClientRect();
        const els = [...section.querySelectorAll<Element>('[role="toolbar"], [aria-label="Potential"], [aria-label="전위"], svg[role="group"]')];
        const rects = els.map((el) => el.getBoundingClientRect()).filter((r) => r.width > 0 && r.height > 0);
        const area = rects.reduce((sum, r) => {
          const w = Math.max(0, Math.min(r.right, box.right) - Math.max(r.left, box.left));
          const h = Math.max(0, Math.min(r.bottom, box.bottom) - Math.max(r.top, box.top));
          return sum + w * h;
        }, 0);
        const toolbar = section.querySelector('[role="toolbar"]')!.getBoundingClientRect();
        return { count: rects.length, share: area / (box.width * box.height), toolbarH: toolbar.height };
      });
      expect(overlays.count, "toolbar, legend and axes measured").toBe(3);
      expect(overlays.share, "overlay share of the 3D view").toBeLessThanOrEqual(0.25);
      expect(overlays.toolbarH, "toolbar is one row").toBeLessThanOrEqual(56);
      const toolbarButtons = await page.getByTestId("viewport-toolbar").locator("button, select").evaluateAll((els) => els.map((el) => el.getBoundingClientRect().height));
      expect(toolbarButtons.every((h) => h >= 43.5), `toolbar targets ${toolbarButtons.join(",")}`).toBe(true);

      // The legend opens on demand and closes again.
      const legend = page.getByTestId("legend-toggle");
      await expect(legend).toHaveAttribute("aria-expanded", "false");
      await legend.click();
      await expect(legend).toHaveAttribute("aria-expanded", "true");
      await expect(view.getByText(lang === "en" ? "Electrode" : "전극")).toBeVisible();
      await legend.click();
      await expect(legend).toHaveAttribute("aria-expanded", "false");
      await frames(page);

      // The first flight framed the scene: Fit all changes nothing.
      await page.getByTestId("viewport-toolbar").getByRole("button", { name: lang === "en" ? "Fit all" : "전체 맞추기" }).click();
      await frames(page);
      const fitted = await camera();
      expect(afterFly.distance).toBeCloseTo(fitted.distance, 3);
      for (let i = 0; i < 3; i++) expect(afterFly.target[i]).toBeCloseTo(fitted.target[i]!, 3);
    });

  test("view menu selects a preset", async ({ page }) => {
    await setup(page, "en");
    await einzel(page);
    const select = page.getByTestId("viewport-toolbar").getByRole("combobox", { name: "View" });
    await select.selectOption("top");
    await expect(select).toHaveValue("top");
    const pitch = await page.evaluate(async () => {
      const url = "/src/viewport/viewState.ts";
      const { cameraState } = (await import(/* @vite-ignore */ url)) as typeof import("../src/viewport/viewState");
      return cameraState()!.pitch;
    });
    expect(Math.abs(pitch)).toBeGreaterThan(1.4);
  });
});

test.describe("AI accounts in a host without the assistant", () => {
  for (const lang of ["en", "ko"] as const)
    test(`desktop app explanation and download, no token wording, ${lang}`, async ({ page }) => {
      await setup(page, lang);
      await useWebHost(page);
      await page.goto("/");
      await page.getByTestId("ai-accounts-open").click();
      const dialog = page.getByTestId("ai-accounts");
      await expect(dialog).toBeVisible();
      const text = (await dialog.textContent()) ?? "";
      if (lang === "en") {
        expect(text).toContain("The assistant needs the desktop app");
        expect(text).not.toMatch(/token|Sign-in happens/i);
        await expect(dialog).toHaveAccessibleDescription(/needs the desktop app/);
      } else {
        expect(text).toContain("어시스턴트는 데스크톱 앱에서만 됩니다");
        expect(text).not.toMatch(/토큰|로그인은 브라우저에서/);
      }
      const link = page.getByTestId("ai-desktop-download");
      await expect(link).toHaveText(lang === "en" ? "Download the desktop app" : "데스크톱 앱 내려받기");
      await expect(link).toHaveAttribute("href", "https://github.com/leeseunguk/SimRep/releases/latest");
      await expect(link).toHaveAttribute("target", "_blank");
      if (lang === "en") await page.screenshot({ path: `${SHOTS}/ai-gated-1280.png` });
    });

  test("desktop (demo) keeps the sign-in intro and privacy line", async ({ page }) => {
    await setup(page, "en");
    await page.goto("/");
    await page.getByTestId("ai-accounts-open").click();
    const dialog = page.getByTestId("ai-accounts");
    await expect(dialog).toContainText("Sign-in happens in your browser");
    await expect(dialog).toContainText("Tokens stay on this computer");
    await expect(page.getByTestId("ai-desktop-download")).toHaveCount(0);
  });
});
