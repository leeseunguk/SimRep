import { expect, test, type Page } from "@playwright/test";
import type { DemoAi } from "../src/demo/ai";
import { answered, startEinzel, step } from "./demo";

// The AI accounts page and assistant panel against the demo bridge (src/demo/ai.ts). With
// ?replay=manual every `ai-event` waits for aiStep, so each state is seen without timing.

type AiWindow = { __simrepDemoAi: DemoAi };
const aiStep = (page: Page) => page.evaluate(() => (window as unknown as AiWindow).__simrepDemoAi.step());
// The first-run tour offer floats over the bottom of the right column; a returning user has seen it.
test.beforeEach(({ page }) => page.addInitScript(() => localStorage.setItem("simrep.tour", "seen")));

const aiQueued = (page: Page) => page.evaluate(() => (window as unknown as AiWindow).__simrepDemoAi.queued());

/** Einzel template refined (it gives the project folder the assistant works in). */
async function workspaceReady(page: Page) {
  const job = await startEinzel(page);
  while ((await step(page, job)) !== null);
  await answered(page, "fly", 1);
}

test("AI: sign in with ChatGPT, chat, a tool card and approval appear, cancel ends the turn", async ({ page }) => {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  await workspaceReady(page);

  // Settings > AI accounts: both providers, nobody signed in.
  await page.getByTestId("ai-accounts-open").click();
  const dialog = page.getByRole("dialog", { name: "AI accounts" });
  await expect(dialog).toBeVisible();
  const chatgpt = dialog.getByRole("button", { name: "Continue with ChatGPT" });
  await expect(chatgpt).toBeVisible();
  await expect(dialog.getByRole("button", { name: "Sign in with Claude" })).toBeVisible();

  // Sign in: "finish in your browser" until the auth event, then success + the one-time welcome.
  await chatgpt.click();
  await expect(dialog.getByText("Finish signing in in your browser")).toBeVisible();
  await expect(chatgpt).toBeDisabled();
  expect(await aiStep(page)).toBe("auth");
  await expect(dialog.getByText(/^Signed in: researcher@example\.com/)).toBeVisible();
  await expect(page.getByTestId("ai-welcome")).toContainText("You're using your ChatGPT plan");
  await page.getByTestId("ai-welcome").getByRole("button", { name: "Got it" }).click();
  await expect(page.getByTestId("ai-welcome")).toHaveCount(0);
  const account = dialog.locator('section[data-provider="chatgpt"]').getByTestId("ai-account");
  await expect(account).toContainText("Plan usage granted");
  await dialog.getByRole("button", { name: "Close" }).first().click();
  await expect(dialog).toBeHidden();

  // Assistant panel: provider, model, the template's folder as workspace.
  await page.getByTestId("ai-toggle").click();
  const panel = page.getByTestId("ai-panel");
  await expect(panel).toBeVisible();
  await expect(panel.getByRole("radio", { name: "ChatGPT" })).toHaveAttribute("aria-checked", "true");
  await expect(panel.getByText("C:\\Users\\demo\\Documents\\SimRep\\einzel")).toBeVisible();
  await expect(panel.getByTestId("ai-using")).toContainText("Using ChatGPT plan");

  // Send: deltas stream into one bubble, then the pa_info tool card fills with its summary.
  await panel.getByRole("textbox", { name: "Message the assistant" }).fill("Fly the einzel ions and report the TOF");
  await panel.getByTestId("ai-send").click();
  await answered(page, "ai_request", 4); // auth.status, auth.login, chat.start, chat.send
  await expect(panel.getByTestId("ai-stop")).toBeVisible();
  let type: string | null;
  while ((type = await aiStep(page)) === "delta");
  expect(type).toBe("tool_call");
  await expect(panel.getByText("I'll check the refined array first, then fly the ions.")).toBeVisible();
  const card = panel.locator('[data-testid="ai-tool-card"][data-tool="pa_info"]');
  await expect(card).toContainText("Running");
  await expect(card).toContainText("einzel.pa0");
  expect(await aiStep(page)).toBe("tool_result");
  await expect(card).toContainText("Done");

  // The write tool asks first; cancel the turn while it waits.
  expect(await aiStep(page)).toBe("tool_call");
  expect(await aiStep(page)).toBe("approval");
  const approval = panel.getByTestId("ai-approval");
  await expect(approval).toContainText("Allow fly?");
  await expect(approval.getByRole("button", { name: "Allow" })).toBeVisible();
  await panel.getByTestId("ai-stop").click();
  await expect.poll(() => aiQueued(page)).toBe(1);
  expect(await aiStep(page)).toBe("done");
  expect(await aiStep(page)).toBeNull();
  await expect(panel.getByText("Stopped. The assistant didn't finish this reply.")).toBeVisible();
  await expect(approval).toContainText("Denied");
  await expect(panel.locator('[data-testid="ai-tool-card"][data-tool="fly"]')).toContainText("Didn't run");
  await expect(panel.getByTestId("ai-send")).toBeVisible();

  // A second turn: allow the write tool, its result card shows the flight summary.
  await panel.getByRole("textbox", { name: "Message the assistant" }).fill("Try again and fly");
  await panel.getByTestId("ai-send").click();
  await answered(page, "ai_request", 6); // chat.cancel, chat.send
  while ((type = await aiStep(page)) !== "approval") expect(type).not.toBeNull();
  await panel.getByTestId("ai-approval").last().getByRole("button", { name: "Allow" }).click();
  await expect.poll(() => aiQueued(page)).toBeGreaterThan(0);
  while ((await aiStep(page)) !== null);
  const fly = panel.locator('[data-testid="ai-tool-card"][data-tool="fly"]').last();
  await expect(fly).toContainText("Done");
  await expect(fly.locator('[data-field="executed"]')).toContainText("cpu · f64");
  await expect(fly.locator('[data-field="compute_ms"]')).toContainText("6.84");
  await expect(fly.locator('[data-field="arrivals"]')).toContainText("37");
  await expect(panel.getByText(/37 of 40 ions reached the detector plane/)).toBeVisible();

  expect(errors).toEqual([]);
});

test("AI: denied sign-in, ChatGPT without plan usage, Claude through an API key", async ({ page }) => {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));

  await page.goto("/?replay=manual&ai=denied");
  await page.getByTestId("ai-accounts-open").click();
  const dialog = page.getByRole("dialog", { name: "AI accounts" });
  await dialog.getByRole("button", { name: "Continue with ChatGPT" }).click();
  expect(await aiStep(page)).toBe("error");
  await expect(dialog.locator('[data-notice="AUTH_DENIED"]')).toContainText("access_denied");

  await page.goto("/?replay=manual&ai=noplan");
  await page.getByTestId("ai-accounts-open").click();
  await dialog.getByRole("button", { name: "Continue with ChatGPT" }).click();
  expect(await aiStep(page)).toBe("auth");
  await expect(dialog.getByText("ChatGPT plan usage isn't enabled")).toBeVisible();
  await expect(dialog.locator('section[data-provider="chatgpt"]').getByTestId("ai-account")).toContainText("Plan usage not granted");
  await expect(dialog.getByRole("button", { name: "Reauthorize" })).toBeVisible();
  await expect(page.getByTestId("ai-welcome")).toHaveCount(0);

  await page.goto("/?replay=manual&ai=apikey");
  await page.getByTestId("ai-accounts-open").click();
  const claude = dialog.locator('section[data-provider="claude"]');
  await expect(claude.getByTestId("ai-account")).toContainText("API key (environment)");
  await expect(claude.getByText(/Sign-out can't remove a key from the environment/)).toBeVisible();
  await expect(claude.getByRole("button", { name: "Sign out" })).toHaveCount(0);

  expect(errors).toEqual([]);
});
