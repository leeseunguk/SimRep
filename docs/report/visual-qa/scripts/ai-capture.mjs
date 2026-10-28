// AI accounts page and assistant panel captures against demo mode (src/demo/ai.ts), with the DOM audit
// (audit.js) per state. Every state is reached by manual replay (no timers), so captures are repeatable.
//
//   cd app && npx vite --mode demo --port 1431 --strictPort &
//   node ../docs/report/visual-qa/scripts/ai-capture.mjs <before|after> [base-url]
//
// Writes docs/report/visual-qa/<phase>/ai-<state>-<w>x<h>-<theme>-<lang>-ai-polish.png and
// ai-polish-<phase>.json (audit results per capture).
import { readFileSync, writeFileSync, mkdirSync } from "node:fs";
import { createRequire } from "node:module";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
// Playwright comes from the app's dev dependencies.
const { chromium } = createRequire(join(here, "../../../../app/package.json"))("@playwright/test");
const phase = process.argv[2] ?? "after";
const base = process.argv[3] ?? "http://127.0.0.1:1431";
// Optional "<width>-<theme>" (e.g. 1280-dark) to capture one slice; slices can run in parallel.
const only = process.argv[4] ?? null;
const outDir = join(here, "..", phase);
mkdirSync(outDir, { recursive: true });
const audit = readFileSync(join(here, "audit.js"), "utf8");

const sizes = [
  [1280, 800],
  [1920, 1080],
];
const themes = ["light", "dark"];
const langs = ["en", "ko"];
const results = {};

const demo = (page, fn, arg) => page.evaluate(fn, arg);
const aiStep = (page) => demo(page, () => window.__simrepDemoAi.step());
const answered = (page, cmd, n) => demo(page, ([c, k]) => window.__simrepDemo.answered(c, k), [cmd, n]);

async function shot(page, name, w, h, theme, lang) {
  await page.waitForTimeout(250); // fonts and the last React commit; captures only, never a test
  const file = `ai-${name}-${w}x${h}-${theme}-${lang}-ai-polish.png`;
  await page.screenshot({ path: join(outDir, file) });
  results[file] = await page.evaluate(`(async () => { ${audit} })()`);
  const r = results[file];
  console.log(file, `small ${r.small.n} mono ${r.monoHangul.n} contrast ${r.lowContrast.n} unnamed ${r.unnamed.n} targets ${r.smallTargets.n}`);
}

async function open(page, query, theme, lang) {
  await page.addInitScript(
    ([t, l]) => {
      localStorage.setItem("simrep.tour", "seen");
      localStorage.setItem("simrep.theme", t);
      localStorage.setItem("simrep.lang", l);
    },
    [theme, lang],
  );
  await page.goto(`${base}/${query}`);
}

const browser = await chromium.launch();
for (const [w, h] of sizes)
  for (const theme of themes)
    for (const lang of langs) {
      if (only && only !== `${w}-${theme}`) continue;
      const ctx = await browser.newContext({ viewport: { width: w, height: h }, colorScheme: theme, locale: lang === "ko" ? "ko-KR" : "en-US" });
      const page = await ctx.newPage();
      page.on("pageerror", (e) => console.error("pageerror", e.message));

      // Accounts: first run, signing in, signed in (+ welcome).
      await open(page, "?replay=manual", theme, lang);
      await page.locator('[data-tour="template-einzel"]').click();
      const job = await answered(page, "refine_start", 1);
      while ((await demo(page, (j) => window.__simrepDemo.step(j), job)) !== null);
      await answered(page, "fly", 1);
      await page.getByTestId("ai-accounts-open").click();
      await page.getByTestId("ai-accounts").waitFor();
      await page.locator('[data-testid="ai-signin-chatgpt"]').waitFor();
      await shot(page, "accounts-empty", w, h, theme, lang);
      await page.getByTestId("ai-signin-chatgpt").click();
      await shot(page, "accounts-signing", w, h, theme, lang);
      await aiStep(page);
      await page.getByTestId("ai-welcome").waitFor();
      await shot(page, "accounts-signed", w, h, theme, lang);
      await page.getByTestId("ai-welcome").getByRole("button").first().click();
      await page.keyboard.press("Escape");

      // Assistant panel: empty, streaming with a tool card, approval pending.
      await page.getByTestId("ai-toggle").click();
      await page.getByTestId("ai-panel").waitFor();
      await shot(page, "panel-empty", w, h, theme, lang);
      await page.getByTestId("ai-panel").getByRole("textbox").fill(lang === "ko" ? "einzel 렌즈 이온을 날리고 TOF를 알려줘" : "Fly the einzel ions and report the TOF");
      await page.getByTestId("ai-send").click();
      await answered(page, "ai_request", 4);
      let type;
      while ((type = await aiStep(page)) === "delta");
      await aiStep(page); // tool_result of pa_info
      await aiStep(page); // tool_call fly (running)
      await shot(page, "panel-streaming", w, h, theme, lang);
      await aiStep(page); // approval
      await page.getByTestId("ai-approval").waitFor();
      await shot(page, "panel-approval", w, h, theme, lang);
      await page.getByTestId("ai-approval").getByRole("button").last().click();
      while ((await aiStep(page)) !== null);
      await shot(page, "panel-done", w, h, theme, lang);

      // Bridge-described providers (demo `?ai=device`): a sign-in-only card, then a device-code sign-in.
      await page.goto(`${base}/?replay=manual&ai=device`);
      await page.getByTestId("ai-accounts-open").click();
      const signInOnly = page.getByTestId("ai-sign-in-only").first();
      if ((await signInOnly.count()) > 0) {
        await signInOnly.scrollIntoViewIfNeeded();
        await shot(page, "accounts-signin-only", w, h, theme, lang);
      }
      const device = page.locator('[data-provider="github-copilot"] [data-testid^="ai-signin-"]').first();
      if ((await device.count()) > 0) {
        await device.scrollIntoViewIfNeeded();
        await device.click();
        await aiStep(page); // device_code
        await page.getByTestId("ai-device-code").waitFor();
        await page.getByTestId("ai-device-code").scrollIntoViewIfNeeded();
        await shot(page, "accounts-device", w, h, theme, lang);
      }
      await ctx.close();
    }
await browser.close();
writeFileSync(join(outDir, only ? `ai-polish-${phase}-${only}.json` : `ai-polish-${phase}.json`), JSON.stringify(results, null, 1));
