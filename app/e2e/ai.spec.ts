import { expect, test, type Page } from "@playwright/test";
import type { DemoAi } from "../src/demo/ai";
import type { DemoLog } from "../src/demo/ipc";
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
  await expect(dialog.getByText("Tokens stay on this computer. The app window never sees them.")).toBeVisible();
  await expect(dialog.locator('[data-provider="chatgpt"] [data-testid="ai-status"]')).toHaveText("Not signed in");

  // Sign in: "finish in your browser" until the auth event, then success + the one-time welcome.
  await chatgpt.click();
  await expect(dialog.getByText("Finish signing in in your browser")).toBeVisible();
  await expect(chatgpt).toBeDisabled();
  expect(await aiStep(page)).toBe("auth");
  await expect(dialog.getByText(/^Signed in: researcher@example\.com/)).toBeVisible();
  await expect(page.getByTestId("ai-welcome")).toContainText("You're using your ChatGPT plan");
  await page.getByTestId("ai-welcome").getByRole("button", { name: "Got it" }).click();
  await expect(page.getByTestId("ai-welcome")).toHaveCount(0);
  await expect(dialog.locator('[data-provider="chatgpt"] [data-testid="ai-status"]')).toHaveText("Plan usage granted");
  const account = dialog.locator('li[data-provider="chatgpt"]').getByTestId("ai-account");
  await expect(account).toContainText("researcher@example.com");
  // Sign-out sits in the account's menu; Escape closes the menu first, not the dialog.
  await account.getByRole("button", { name: /^More for researcher@example\.com/ }).click();
  await expect(account.getByRole("menuitem", { name: "Sign out" })).toBeFocused();
  await page.keyboard.press("Escape");
  await expect(account.getByRole("menu")).toHaveCount(0);
  await expect(dialog).toBeVisible();
  await dialog.getByRole("button", { name: "Close" }).first().click();
  await expect(dialog).toBeHidden();

  // Assistant panel: provider, model, the template's folder as workspace.
  await page.getByTestId("ai-toggle").click();
  const panel = page.getByTestId("ai-panel");
  await expect(panel).toBeVisible();
  await expect(panel.getByRole("radio", { name: "ChatGPT" })).toHaveAttribute("aria-checked", "true");
  await expect(panel.getByText("C:\\Users\\demo\\Documents\\SimRep\\einzel")).toBeVisible();
  await expect(panel.getByTestId("ai-using")).toContainText("Using ChatGPT plan");

  // Empty chat: an example request fills the composer.
  await expect(panel.getByTestId("ai-example")).toHaveCount(3);
  await panel.getByTestId("ai-example").first().click();
  await expect(panel.getByRole("textbox", { name: "Message the assistant" })).toHaveValue("Refine the einzel lens potentials and fly the ions");
  await expect(panel.getByRole("textbox", { name: "Message the assistant" })).toBeFocused();

  // Send: deltas stream into one bubble, then the pa_info tool card fills with its summary.
  await panel.getByRole("textbox", { name: "Message the assistant" }).fill("Fly the einzel ions and report the TOF");
  await panel.getByTestId("ai-send").click();
  await answered(page, "ai_request", 5); // auth.providers, auth.status, auth.login, chat.start, chat.send
  await expect(panel.getByTestId("ai-stop")).toBeVisible();
  let type: string | null;
  // Text still arriving shows the caret on the streaming bubble; it goes once a tool card follows.
  expect(await aiStep(page)).toBe("delta");
  await expect(panel.locator("[data-streaming]")).toHaveCount(1);
  while ((type = await aiStep(page)) === "delta");
  expect(type).toBe("tool_call");
  await expect(panel.getByTestId("ai-active-skill")).toHaveText("einzel");
  await expect(panel.getByText("I'll check the refined array first, then fly the ions.")).toBeVisible();
  await expect(panel.locator("[data-streaming]")).toHaveCount(0);
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
  await expect(approval.getByRole("button", { name: "Allow", exact: true })).toBeVisible();
  await expect(approval).toContainText(/[12]:\d\d left/);
  // Escape in the panel stops the running reply.
  await panel.getByRole("textbox", { name: "Message the assistant" }).focus();
  await page.keyboard.press("Escape");
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
  await answered(page, "ai_request", 7); // chat.cancel, chat.send
  while ((type = await aiStep(page)) !== "approval") expect(type).not.toBeNull();
  await panel.getByTestId("ai-approval").last().getByRole("button", { name: "Allow", exact: true }).click();
  await expect.poll(() => aiQueued(page)).toBeGreaterThan(0);
  while ((await aiStep(page)) !== null);
  const fly = panel.locator('[data-testid="ai-tool-card"][data-tool="fly"]').last();
  await expect(fly).toContainText("Done");
  await expect(fly.locator('[data-field="executed"]')).toContainText("cpu · f64");
  await expect(fly.locator('[data-field="compute_ms"]')).toContainText("6.84");
  await expect(fly.locator('[data-field="arrivals"]')).toContainText("37");
  await expect(panel.getByText(/37 of 40 ions reached the detector plane/)).toBeVisible();
  await page.screenshot({ path: test.info().outputPath("harness-desktop.png") });
  await page.setViewportSize({ width: 390, height: 844 });
  await page.screenshot({ path: test.info().outputPath("harness-mobile.png") });

  expect(errors).toEqual([]);
});

test("AI: changing the selected account starts a distinct billed session", async ({ page }) => {
  await workspaceReady(page);
  await page.getByTestId("ai-accounts-open").click();
  const dialog = page.getByRole("dialog", { name: "AI accounts" });
  await dialog.getByRole("button", { name: "Continue with ChatGPT" }).click();
  expect(await aiStep(page)).toBe("auth");
  await page.getByTestId("ai-welcome").getByRole("button", { name: "Got it" }).click();
  await dialog.getByRole("button", { name: "Continue with ChatGPT" }).click();
  expect(await aiStep(page)).toBe("auth");
  await dialog.getByRole("button", { name: "Close" }).first().click();
  await page.getByTestId("ai-toggle").click();
  const panel = page.getByTestId("ai-panel");
  await panel.getByRole("textbox", { name: "Message the assistant" }).fill("Read the first account");
  await panel.getByTestId("ai-send").click();
  await expect(panel.getByTestId("ai-stop")).toBeVisible();
  await panel.getByTestId("ai-stop").click();
  await answered(page, "ai_request", 7);
  expect(await aiStep(page)).toBe("done");
  await panel.getByLabel("Account", { exact: true }).selectOption("chatgpt-demo-2");
  await panel.getByRole("textbox", { name: "Message the assistant" }).fill("Read the second account");
  await panel.getByTestId("ai-send").click();
  await expect(panel.getByTestId("ai-stop")).toBeVisible();
  await answered(page, "ai_request", 9);
  const started = await page.evaluate(() => (window as unknown as AiWindow).__simrepDemoAi.started);
  expect(started.map((s) => s.account)).toEqual(["chatgpt-demo-1", "chatgpt-demo-2"]);
  expect(started[0]!.sessionId).not.toBe(started[1]!.sessionId);
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
  await expect(dialog.locator('[data-provider="chatgpt"] [data-testid="ai-status"]')).toHaveText("Plan usage not granted");
  await expect(dialog.getByRole("button", { name: "Reauthorize" })).toBeVisible();
  await expect(page.getByTestId("ai-welcome")).toHaveCount(0);

  await page.goto("/?replay=manual&ai=apikey");
  await page.getByTestId("ai-accounts-open").click();
  const claude = dialog.locator('li[data-provider="claude"]');
  await expect(claude.getByTestId("ai-status")).toHaveText("API key (environment)");
  await expect(claude.getByText(/Sign-out can't remove a key from the environment/)).toBeVisible();
  await expect(claude.getByRole("button", { name: /^More for / })).toHaveCount(0);

  expect(errors).toEqual([]);
});

test("AI: a device-code provider shows its code to confirm, without a WebView link, then signs in", async ({ page }) => {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));

  await page.goto("/?replay=manual&ai=device");
  await page.getByTestId("ai-accounts-open").click();
  const dialog = page.getByRole("dialog", { name: "AI accounts" });
  const card = dialog.locator('li[data-provider="github-copilot"]');
  // A provider the UI has no entry for: the bridge's label, generic texts and a neutral monogram.
  await expect(card.getByRole("heading", { name: "GitHub Copilot" })).toBeVisible();
  await card.getByRole("button", { name: "Sign in with GitHub Copilot" }).click();
  await expect(card.getByTestId("ai-status")).toHaveText("Signing in…");
  expect(await aiStep(page)).toBe("device_code");
  const code = card.getByTestId("ai-device-code");
  await expect(code.getByLabel("Device code")).toHaveText("ABCD-EFGH");
  await expect(code).toContainText(/Expires in 1[45]:\d\d/);
  await expect(code).toContainText("https://github.com/login/device");
  await expect(card.getByRole("link")).toHaveCount(0);
  expect(await aiStep(page)).toBe("auth");
  await expect(code).toHaveCount(0);
  await expect(card.getByTestId("ai-status")).toHaveText("Signed in");

  // Bridge metadata: a sign-in-only provider says so; Kimi asks for its account region.
  await expect(dialog.locator('li[data-provider="cursor"]').getByTestId("ai-sign-in-only")).toHaveText("Sign-in only");
  await expect(dialog.locator('li[data-provider="kimi-coding"]').getByLabel("Account region")).toHaveValue("mainland-cn");

  expect(errors).toEqual([]);
});

/** Signs in with ChatGPT in the demo bridge and opens the assistant panel. */
async function signedInPanel(page: Page) {
  await page.getByTestId("ai-accounts-open").click();
  const dialog = page.getByRole("dialog", { name: "AI accounts" });
  await dialog.getByRole("button", { name: "Continue with ChatGPT" }).click();
  expect(await aiStep(page)).toBe("auth");
  await page.getByTestId("ai-welcome").getByRole("button", { name: "Got it" }).click();
  await dialog.getByRole("button", { name: "Close" }).first().click();
  await page.getByTestId("ai-toggle").click();
  return page.getByTestId("ai-panel");
}

test("AI: allow all in this chat, the Bypass confirmation, warning chip and auto-approved cards", async ({ page }) => {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  await workspaceReady(page);
  const panel = await signedInPanel(page);
  const box = panel.getByRole("textbox", { name: "Message the assistant" });
  const mode = panel.getByLabel("Approval", { exact: true });
  await expect(mode).toHaveValue("ask");
  await expect(panel.getByTestId("ai-approval-skipping")).toHaveCount(0);

  // Turn 1 asks; "Allow all in this chat" allows it and switches the session.
  await box.fill("Fly the einzel ions");
  await panel.getByTestId("ai-send").click();
  let type: string | null;
  while ((type = await aiStep(page)) !== "approval") expect(type).not.toBeNull();
  const approval = panel.getByTestId("ai-approval");
  await expect(approval.getByRole("button", { name: "Allow all in this chat" })).toBeVisible();
  await approval.getByRole("button", { name: "Allow all in this chat" }).click();
  await expect(approval).toContainText("Allowed");
  await expect.poll(() => aiQueued(page)).toBeGreaterThan(0);
  while ((await aiStep(page)) !== null);
  const skipping = panel.getByTestId("ai-approval-skipping");
  await expect(skipping).toContainText("Approvals skipped");
  await expect(skipping).toHaveAttribute("data-mode", "session");
  await expect(panel.locator('[data-testid="ai-tool-card"][data-tool="fly"] [data-testid="ai-auto"]')).toHaveCount(0);

  // Turn 2 runs fly without asking: no new approval card, the fly card carries the auto marker.
  await box.fill("Fly again");
  await panel.getByTestId("ai-send").click();
  await expect.poll(() => aiQueued(page)).toBeGreaterThan(0);
  while ((await aiStep(page)) !== null);
  await expect(panel.getByTestId("ai-approval")).toHaveCount(1);
  const autoFly = panel.locator('[data-testid="ai-tool-card"][data-tool="fly"]').last();
  await expect(autoFly.getByTestId("ai-auto").locator("[aria-hidden]")).toHaveText("auto");
  await expect(autoFly.getByTestId("ai-auto")).toContainText("Approved automatically, without asking");
  await expect(autoFly).toContainText("Done");

  // "Ask again" (keyboard) ends it; the next chat.start asks.
  await skipping.getByRole("button", { name: "Ask again" }).focus();
  await page.keyboard.press("Enter");
  await expect(panel.getByTestId("ai-approval-skipping")).toHaveCount(0);

  // Bypass: the confirmation explains it; Escape keeps asking, confirming turns it on.
  await mode.selectOption("bypass");
  const confirm = page.getByRole("alertdialog", { name: "Run write tools without asking?" });
  await expect(confirm).toContainText("refine, fly, export");
  await expect(confirm).toContainText("only inside the project folder");
  await expect(confirm.getByRole("button", { name: "Keep asking" })).toBeFocused();
  await page.keyboard.press("Escape");
  await expect(confirm).toHaveCount(0);
  await expect(mode).toHaveValue("ask");
  await mode.selectOption("bypass");
  await confirm.getByRole("button", { name: "Run without asking" }).click();
  await expect(mode).toHaveValue("bypass");
  await expect(skipping).toHaveAttribute("data-mode", "bypass");
  await expect(skipping).toContainText("Write tools run without asking, inside the project folder only.");
  const modes = await page.evaluate(() => (window as unknown as AiWindow).__simrepDemoAi.modes);
  expect(modes).toEqual(["session", "ask", "bypass"]);

  // A new chat starts in bypass; its fly is auto-approved.
  await panel.getByRole("button", { name: "New chat" }).click();
  await box.fill("Fly the ions");
  await panel.getByTestId("ai-send").click();
  await expect.poll(() => aiQueued(page)).toBeGreaterThan(0);
  while ((await aiStep(page)) !== null);
  const started = await page.evaluate(() => (window as unknown as AiWindow).__simrepDemoAi.started);
  expect(started.map((s) => s.approvalMode)).toEqual(["ask", "bypass"]);
  await expect(panel.getByTestId("ai-approval")).toHaveCount(0);
  await expect(panel.locator('[data-testid="ai-tool-card"][data-tool="fly"]').getByTestId("ai-auto")).toHaveCount(1);

  // Not remembered: a new launch starts in Ask.
  await workspaceReady(page);
  await page.getByTestId("ai-toggle").click();
  await expect(page.getByTestId("ai-panel").getByLabel("Approval", { exact: true })).toHaveValue("ask");
  expect(errors).toEqual([]);
});

test("AI: a remembered Bypass survives a new launch", async ({ page }) => {
  await workspaceReady(page);
  await page.getByTestId("ai-toggle").click();
  const panel = page.getByTestId("ai-panel");
  await panel.getByLabel("Approval", { exact: true }).selectOption("bypass");
  const confirm = page.getByRole("alertdialog", { name: "Run write tools without asking?" });
  await confirm.getByRole("checkbox", { name: "Keep this after SimRep restarts" }).check();
  await confirm.getByRole("button", { name: "Run without asking" }).click();
  await workspaceReady(page);
  await page.getByTestId("ai-toggle").click();
  await expect(page.getByTestId("ai-panel").getByLabel("Approval", { exact: true })).toHaveValue("bypass");
  await expect(page.getByTestId("ai-approval-skipping")).toContainText("Approvals skipped");
});

type FilesWindow = { __simrepDemo: DemoLog };
/** Saved chats in the demo's project data folders (src/demo/ipc.ts, kept across a reload): path -> text. */
const savedFiles = (page: Page) =>
  page.evaluate(() => {
    const demo = (window as unknown as FilesWindow).__simrepDemo;
    return Object.fromEntries(Object.keys(demo.files()).filter((p) => /\.simrep\.d[\\/]ai[\\/]/.test(p)).map((p) => [p, demo.text(p)!]));
  });

/** Einzel refined and saved as a project, so it has a data folder (`<X>.simrep.d/`). */
async function savedProject(page: Page) {
  await workspaceReady(page);
  await page.getByRole("button", { name: "Save", exact: true }).click();
  await answered(page, "write_text");
  await expect(page.getByRole("status").filter({ hasText: /^Saved / }).first()).toBeVisible();
}

test("AI: chats are saved per project, listed after a reload, opened read-only and continued", async ({ page }) => {
  test.setTimeout(120_000);
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  await savedProject(page);
  let panel = await signedInPanel(page);
  const box = panel.getByRole("textbox", { name: "Message the assistant" });

  // One turn (no write tool asked: the second turn of a session); autosave follows its `done`.
  await box.fill("Check the refined einzel array");
  await panel.getByTestId("ai-send").click();
  let type: string | null;
  while ((type = await aiStep(page)) !== "approval") expect(type).not.toBeNull();
  await panel.getByTestId("ai-approval").getByRole("button", { name: "Allow", exact: true }).click();
  await expect.poll(() => aiQueued(page)).toBeGreaterThan(0);
  while ((await aiStep(page)) !== null);
  await expect.poll(async () => Object.keys(await savedFiles(page)).length).toBe(1);
  const [path, text] = Object.entries(await savedFiles(page))[0]!;
  expect(path).toMatch(/\.simrep\.d[\\/]ai[\\/][0-9a-f-]{36}\.json$/);
  const saved = JSON.parse(text);
  expect(saved).toMatchObject({ title: "Check the refined einzel array", provider: "chatgpt", providerLabel: "ChatGPT", model: "gpt-6.1-sol", recipe: "einzel" });
  expect(saved.approvals).toEqual([{ name: "fly", decision: "allowed" }]);
  // The demo account is researcher@example.com (id chatgpt-demo-1): neither is in the file.
  expect(text).not.toMatch(/@|chatgpt-demo-1|eyJ[\w-]+\.[\w-]+\.|token/i);

  // The report's AI appendix carries the active chat, redacted like the saved file.
  await page.getByTestId("topbar-export").getByRole("button", { name: "Export" }).click();
  await page.getByRole("menu").getByRole("menuitem", { name: /Report/ }).click();
  const report = page.getByTestId("report-dialog");
  await expect(report.getByRole("switch", { name: "AI chat transcript (appendix)" })).toBeChecked();
  await expect(report.getByText("No chat transcript is available.")).toHaveCount(0);
  await report.getByTestId("report-run").click();
  await answered(page, "write_bytes", 1);
  const html = (await page.evaluate(() => (window as unknown as FilesWindow).__simrepDemo.writes)).at(-1)!.text;
  expect(html).toContain("Appendix: AI assistant transcript");
  expect(html).toContain("Check the refined einzel array");
  expect(html).toContain("Tool · fly");
  expect(html).not.toMatch(/researcher@example\.com|chatgpt-demo-1|eyJ[\w-]+\.[\w-]+\./);
  await report.getByRole("button", { name: "Close" }).first().click();
  await expect(report).toBeHidden();

  // Reload: the same project (saved again to its path) lists the chat. The one-time welcome is
  // already dismissed, so this sign-in has none.
  await page.reload();
  await savedProject(page);
  await page.getByTestId("ai-accounts-open").click();
  const dialog = page.getByRole("dialog", { name: "AI accounts" });
  await dialog.getByRole("button", { name: "Continue with ChatGPT" }).click();
  expect(await aiStep(page)).toBe("auth");
  await dialog.getByRole("button", { name: "Close" }).first().click();
  await page.getByTestId("ai-toggle").click();
  panel = page.getByTestId("ai-panel");
  await panel.getByTestId("ai-history-toggle").click();
  const history = panel.getByTestId("ai-history");
  await expect(history.getByRole("heading", { name: "Previous chats" })).toBeVisible();
  const item = history.getByTestId("ai-history-item");
  await expect(item).toHaveCount(1);
  await expect(item).toContainText("Check the refined einzel array");
  await expect(item).toContainText("ChatGPT · gpt-6.1-sol");

  // Open read-only: the transcript, no composer, a Continue action.
  await item.getByRole("button", { name: "Open Check the refined einzel array (read only)" }).click();
  await expect(panel.getByTestId("ai-history-readonly")).toContainText("Read only");
  await expect(panel.getByText(/37 of 40 ions reached the detector plane/)).toBeVisible();
  await expect(panel.locator('[data-testid="ai-tool-card"][data-tool="fly"]')).toContainText("Done");
  await expect(box).toBeDisabled();

  // Continue: a new chat marked as continued; its chat.start carries the bounded summary.
  await panel.getByTestId("ai-history-continue-open").click();
  await expect(panel.getByTestId("ai-continued")).toContainText("Continued");
  await expect(panel.getByTestId("ai-continued")).toContainText("Check the refined einzel array");
  await expect(box).toBeEnabled();
  await box.fill("Lower electrode 2 and fly again");
  await panel.getByTestId("ai-send").click();
  await expect(panel.getByTestId("ai-stop")).toBeVisible();
  const started = await page.evaluate(() => (window as unknown as AiWindow).__simrepDemoAi.started);
  expect(started).toHaveLength(1);
  expect(started[0]!.context).toContain("User: Check the refined einzel array");
  expect(started[0]!.context!.length).toBeLessThanOrEqual(4000);
  while ((type = await aiStep(page)) !== "approval") expect(type).not.toBeNull();
  await panel.getByTestId("ai-approval").getByRole("button", { name: "Deny" }).click();
  await expect.poll(() => aiQueued(page)).toBeGreaterThan(0);
  while ((await aiStep(page)) !== null);
  await expect.poll(async () => Object.keys(await savedFiles(page)).length).toBe(2);

  // The list shows both, the continued one marked; delete asks first and removes the file.
  await panel.getByTestId("ai-history-toggle").click();
  await expect(item).toHaveCount(2);
  await expect(item.first()).toContainText("Lower electrode 2 and fly again");
  await expect(item.first()).toContainText("Continued");
  await page.screenshot({ path: test.info().outputPath("history-list.png") });
  await item.last().getByTestId("ai-history-delete").click();
  await expect(item.last().getByRole("group", { name: "Delete this chat?" })).toBeVisible();
  await item.last().getByTestId("ai-history-delete-confirm").click();
  await expect(item).toHaveCount(1);
  expect(Object.keys(await savedFiles(page))).toHaveLength(1);
  expect(errors).toEqual([]);
});
