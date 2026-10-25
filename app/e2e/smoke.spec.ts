import { expect, test } from "@playwright/test";
import { answered, convergenceChart, demoLog, startEinzel, step } from "./demo";

test("start screen -> einzel template -> refine -> fly -> TOF histogram -> CSV export, light and dark", async ({ page }) => {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));

  // Start screen: the template gallery; the einzel template starts a refine.
  const job = await startEinzel(page);

  // Refine: the replayed channel drives the live convergence chart, one point per V-cycle.
  expect(await step(page, job)).toBe("start");
  expect(await step(page, job)).toBe("progress");
  const chart = convergenceChart(page);
  await expect(chart).toHaveAttribute("aria-label", /after 0 V-cycles, target 1e-10/);
  while ((await step(page, job)) !== null);
  expect((await demoLog(page)).refine[job]!.at(-1)).toBe("done");
  await expect(chart).toBeHidden();

  // The template flies once refined; fly again from the properties panel.
  await answered(page, "fly", 1);
  const fly = page.locator('[data-tour="fly"]');
  await expect(fly).toBeEnabled();
  await fly.click();
  await answered(page, "fly", 2);

  // TOF histogram in the results panel (clicking the tab that is already shown collapses the panel).
  const panel = page.locator('[data-tour="results"]');
  const hist = page.locator('[data-tour="results-tab-hist"]');
  if ((await panel.getAttribute("data-open")) !== "true" || (await hist.getAttribute("aria-selected")) !== "true") await hist.click();
  await expect(panel).toHaveAttribute("data-open", "true");
  await expect(hist).toHaveAttribute("aria-selected", "true");
  await expect(page.getByRole("img", { name: /TOF/ })).toBeVisible();

  // CSV export from the results export menu (the mocked save dialog answers a path).
  await page.locator('[data-tour="results-export"]').getByRole("button", { name: "Export" }).click();
  const menu = page.getByRole("menu");
  await expect(menu).toBeVisible();
  await menu.getByRole("menuitem", { name: /End states of all ions/ }).click();
  await answered(page, "write_bytes", 1);
  const [csv] = (await demoLog(page)).writes;
  expect(csv!.path).toMatch(/\.csv$/);
  const rows = csv!.text.trim().split("\r\n");
  expect(rows[0]).toBe("ion,surface,x_mm,y_mm,z_mm,r_mm,tof_us,ke_ev,vx_mm_per_us,vy_mm_per_us,vz_mm_per_us");
  expect(rows).toHaveLength(41);

  // Light / dark theme.
  const theme = page.getByRole("radiogroup", { name: "Theme" });
  await theme.getByRole("radio", { name: "Dark" }).click();
  await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");
  await theme.getByRole("radio", { name: "Light" }).click();
  await expect(page.locator("html")).toHaveAttribute("data-theme", "light");

  expect(errors).toEqual([]);
});
