import { existsSync, readdirSync, readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { en } from "../i18n/en";
import { translate } from "../i18n";
import { aiReason, refusalLine, refusals } from "../refusals";
import { accountKind, applyAuthEvent, deviceCodeOf, EMPTY_ACCOUNTS, providerState, providerSummary, setNotice, startSignIn, usableAccounts } from "./accounts";
import { PROVIDER_LIST, providerIds, providerInfo } from "./providers";
import { addUser, applyEvent, EMPTY_CHAT, keyArgs, resolveApproval, summarizeResult, writesFiles, type ChatState } from "./chat";
import { AI_ERROR_CODES, AiError, parseEvent, REFRESH_FAILED_CODES, unwrap, type Account, type AiEvent } from "./protocol";

const gpt = (over: Partial<Account> = {}): Account => ({ id: "g1", provider: "chatgpt", label: "a@example.com (g1)", authenticated: true, planEnabled: true, ...over });
const chatOf = (sessionId = "s1"): ChatState => ({ ...EMPTY_CHAT, session: { id: sessionId, provider: "chatgpt", model: "gpt-5.4", workspace: "/w" } });
const ev = (e: Record<string, unknown>) => parseEvent({ provider: "chatgpt", sessionId: "s1", turnId: "t1", ...e }) as AiEvent;
const run = (s: ChatState, ...events: Record<string, unknown>[]) => events.reduce((acc, e) => applyEvent(acc, ev(e)), s);

describe("protocol mapping", () => {
  it("keeps only protocol fields of an event and drops unknown types and providers", () => {
    expect(parseEvent({ type: "delta", provider: "chatgpt", sessionId: "s", turnId: "t", text: "hi", token: "secret" })).toEqual({ type: "delta", provider: "chatgpt", sessionId: "s", turnId: "t", text: "hi" });
    expect(parseEvent({ type: "tokens", provider: "chatgpt" })).toBeNull();
    expect(parseEvent({ type: "delta", provider: 42, sessionId: "s", text: "x" })).toBeNull();
    expect(parseEvent({ type: "done", provider: "claude", sessionId: "s", status: "weird" })).toBeNull();
    const auth = parseEvent({ type: "auth", provider: "chatgpt", status: { provider: "chatgpt", authenticated: true, accounts: [{ ...gpt(), access_token: "x" }] } });
    expect(auth?.type === "auth" && auth.status.accounts[0]).toEqual(gpt());
  });

  it("turns a JSON-RPC error into the bridge code", () => {
    expect(unwrap({ jsonrpc: "2.0", id: 1, result: { started: true } })).toEqual({ started: true });
    const failed = () => unwrap({ jsonrpc: "2.0", id: 1, error: { code: -32000, message: "no plan", data: { code: "PLAN_PERMISSION_REQUIRED" } } });
    expect(failed).toThrow(AiError);
    try {
      failed();
    } catch (e) {
      expect((e as AiError).code).toBe("PLAN_PERMISSION_REQUIRED");
    }
  });
});

describe("provider card state", () => {
  it("reads a device code from auth.login and ignores a plain browser start", () => {
    expect(deviceCodeOf({ started: true })).toBeNull();
    expect(deviceCodeOf({ started: true, userCode: "WDJB-MJHT", expiresIn: 900 }, 1000)).toEqual({ userCode: "WDJB-MJHT", expiresAt: 901_000 });
    expect(deviceCodeOf({ started: true, userCode: "ABCD-1234" })).toEqual({ userCode: "ABCD-1234", expiresAt: null });
    expect(deviceCodeOf({ started: true, userCode: "<b>no</b>" })).toBeNull();
  });

  it("summarizes a provider for its status chip", () => {
    const status = (accounts: Account[]) => ({ status: { provider: "chatgpt", authenticated: accounts.some((a) => a.authenticated), accounts }, notice: null });
    expect(providerSummary(providerState(EMPTY_ACCOUNTS, "chatgpt"))).toBe("none");
    expect(providerSummary(status([gpt()]))).toBe("ready");
    expect(providerSummary(status([gpt({ planEnabled: false })]))).toBe("noPlan");
    expect(providerSummary(status([gpt({ authenticated: false, planEnabled: false })]))).toBe("signedOut");
    expect(providerSummary(status([gpt({ authenticated: false, planEnabled: false }), gpt({ id: "g2" })]))).toBe("ready");
    const expired = setNotice({ chatgpt: status([gpt({ authenticated: false, planEnabled: false })]) }, "chatgpt", { kind: "failed", action: "signIn", code: "invalid_grant" });
    expect(providerSummary(providerState(expired, "chatgpt"))).toBe("expired");
    expect(providerSummary(providerState(startSignIn(EMPTY_ACCOUNTS, "chatgpt"), "chatgpt"))).toBe("signingIn");
  });
});

describe("chat reducer", () => {
  it("retains only a recognized active harness recipe for the current session", () => {
    const s = run(addUser(chatOf(), "focus"), { type: "delta", text: "result", activeSkill: "einzel" });
    expect(s.activeSkill).toBe("einzel");
    expect(run(s, { type: "done", status: "completed" }).activeSkill).toBe("einzel");
    expect(run(s, { type: "delta", text: "x", activeSkill: "../outside" }).activeSkill).toBe("einzel");
    expect(writesFiles("simrep_plan")).toBe(false);
    expect(writesFiles("analyzer_scan")).toBe(true);
    expect(writesFiles("grid_convergence")).toBe(true);
    expect(writesFiles("check_geometry")).toBe(true);
  });

  it("streams deltas into one assistant bubble per run of text", () => {
    let s = addUser(chatOf(), "fly einzel");
    expect(s.running).toBe(true);
    s = run(s, { type: "delta", text: "Checking " }, { type: "delta", text: "the array." });
    expect(s.items.map((i) => i.kind)).toEqual(["user", "assistant"]);
    expect(s.items[1]).toMatchObject({ text: "Checking the array." });
    s = run(s, { type: "tool_call", callId: "c1", name: "pa_info", arguments: { pa: "einzel.pa0" } }, { type: "delta", text: "Done." });
    expect(s.items.map((i) => i.kind)).toEqual(["user", "assistant", "tool", "assistant"]);
  });

  it("ignores events of another session", () => {
    const s = addUser(chatOf(), "x");
    expect(applyEvent(s, parseEvent({ type: "delta", provider: "chatgpt", sessionId: "old", text: "late" })!)).toBe(s);
  });

  it("fills a tool card with its result summary", () => {
    const result = { content: [{ type: "text", text: JSON.stringify({ schema_version: 1, executed: { device: "cpu", precision: "f64", backend: null }, compute_ms: 6.84, ion_count: 40, arrivals: 37, arrival_tof_us: { mean: 15.3, std: 0.02 } }) }], isError: false };
    const s = run(addUser(chatOf(), "x"), { type: "tool_call", callId: "c1", name: "fly", arguments: { pa: "a.pa0" } }, { type: "tool_result", callId: "c1", result });
    const card = s.items[1]!;
    expect(card).toMatchObject({ kind: "tool", status: "done" });
    expect(card.kind === "tool" && card.result?.fields).toEqual([
      { key: "executed", value: "cpu · f64" },
      { key: "compute_ms", value: "6.84" },
      { key: "ion_count", value: "40" },
      { key: "arrivals", value: "37" },
      { key: "arrival_tof_us", value: "15.3 ± 0.02" },
    ]);
  });

  it("marks failed tools and keeps their message as text", () => {
    const s = run(addUser(chatOf(), "x"), { type: "tool_call", callId: "c1", name: "fly", arguments: {} }, { type: "tool_result", callId: "c1", result: { content: [{ type: "text", text: "User declined tool execution." }], isError: true } });
    expect(s.items[1]).toMatchObject({ status: "error", result: { isError: true, text: "User declined tool execution." } });
    expect(summarizeResult({ content: [{ type: "text", text: '{"error":{"message":"bad pa","kind":"io"}}' }] })).toEqual({ isError: true, fields: [], text: "bad pa" });
  });

  it("resolves approvals and closes open ones when the turn ends", () => {
    let s = run(addUser(chatOf(), "x"), { type: "approval", approvalId: "a1", name: "fly", arguments: {} }, { type: "approval", approvalId: "a2", name: "refine", arguments: {} });
    s = resolveApproval(s, "a1", true);
    s = run(s, { type: "tool_call", callId: "c9", name: "export_grid", arguments: {} }, { type: "done", status: "interrupted" });
    expect(s.running).toBe(false);
    expect(s.items.filter((i) => i.kind === "approval").map((i) => i.kind === "approval" && i.state)).toEqual(["allowed", "denied"]);
    expect(s.items.find((i) => i.kind === "tool")).toMatchObject({ status: "skipped" });
    expect(s.items.at(-1)).toMatchObject({ kind: "ended", status: "interrupted" });
  });

  it("adds no end marker for a completed turn", () => {
    const s = run(addUser(chatOf(), "x"), { type: "delta", text: "ok" }, { type: "done", status: "completed" });
    expect(s.items.map((i) => i.kind)).toEqual(["user", "assistant"]);
  });

  it("knows which tools write files and shows short key arguments", () => {
    expect(writesFiles("pa_info")).toBe(false);
    expect(writesFiles("fly")).toBe(true);
    expect(keyArgs({ pa: "einzel.pa0", voltage: { 1: 0, 2: 110 }, nested: { a: { b: 1 } }, device: "cpu" })).toEqual([
      { key: "pa", value: "einzel.pa0" },
      { key: "voltage", value: "1=0, 2=110" },
      { key: "device", value: "cpu" },
    ]);
  });
});

describe("accounts reducer", () => {
  it("classifies accounts by plan and credential", () => {
    expect(accountKind(gpt())).toBe("ready");
    expect(accountKind(gpt({ planEnabled: false }))).toBe("noPlan");
    expect(accountKind(gpt({ authenticated: false, planEnabled: false }))).toBe("signedOut");
    expect(accountKind({ id: "claude-local", provider: "claude", label: "key", authenticated: true, planEnabled: false })).toBe("apiKey");
    expect(accountKind({ id: "claude-local", provider: "claude", label: "me", authenticated: true, planEnabled: true })).toBe("ready");
  });

  it("finishes a waiting sign-in from the auth event: success or not eligible", () => {
    const waiting = startSignIn(EMPTY_ACCOUNTS, "chatgpt");
    const ok = applyAuthEvent(waiting, { type: "auth", provider: "chatgpt", status: { provider: "chatgpt", authenticated: true, accounts: [gpt()] } });
    expect(providerState(ok, "chatgpt").notice).toEqual({ kind: "success", account: gpt().label });
    expect(usableAccounts(ok.chatgpt).map((a) => a.id)).toEqual(["g1"]);
    const noPlan = applyAuthEvent(waiting, { type: "auth", provider: "chatgpt", status: { provider: "chatgpt", authenticated: true, accounts: [gpt({ planEnabled: false })] } });
    expect(providerState(noPlan, "chatgpt").notice).toEqual({ kind: "notEligible", account: gpt().label });
    expect(usableAccounts(noPlan.chatgpt)).toEqual([]);
  });

  it("ends a waiting sign-in on a provider error (denied), and leaves chat errors alone", () => {
    const waiting = startSignIn(EMPTY_ACCOUNTS, "chatgpt");
    const denied = applyAuthEvent(waiting, { type: "error", provider: "chatgpt", code: "AUTH_DENIED", message: "declined" });
    expect(providerState(denied, "chatgpt").notice).toEqual({ kind: "failed", action: "signIn", code: "AUTH_DENIED" });
    expect(applyAuthEvent(waiting, { type: "error", provider: "chatgpt", code: "BUSY", message: "", sessionId: "s1" })).toBe(waiting);
    expect(providerState(applyAuthEvent(waiting, { type: "error", provider: "chatgpt", code: "PLAN_PERMISSION_REQUIRED", message: "" }), "chatgpt").notice).toEqual({ kind: "notEligible", account: null });
  });
});

describe("provider list", () => {
  it("describes each provider as data, with the fixed ChatGPT label", () => {
    expect(PROVIDER_LIST.map((p) => [p.id, p.authKind])).toEqual([
      ["chatgpt", "oauth"],
      ["claude", "cli-login"],
    ]);
    expect(translate("ko", providerInfo("chatgpt").signInLabel!)).toBe("Continue with ChatGPT");
    expect(translate("en", "ai.auth.signIn", { provider: providerInfo("claude").label })).toBe("Sign in with Claude");
  });

  it("shows providers the bridge adds with generic texts and no plan gate", () => {
    expect(providerIds(["claude", "gemini"])).toEqual(["chatgpt", "claude", "gemini"]);
    expect(providerInfo("gemini")).toMatchObject({ label: "gemini", authKind: "oauth", models: [""] });
    const s = applyAuthEvent(startSignIn(EMPTY_ACCOUNTS, "gemini"), parseEvent({ type: "auth", provider: "gemini", status: { provider: "gemini", authenticated: true, accounts: [{ id: "x", provider: "gemini", label: "me", authenticated: true, planEnabled: false }] } })!);
    expect(providerState(s, "gemini").notice).toEqual({ kind: "success", account: "me" });
    expect(parseEvent({ type: "delta", provider: "Bad Name!", sessionId: "s", text: "x" })).toBeNull();
  });
});

describe("AI refusals", () => {
  it("explain every bridge code in both languages", () => {
    for (const lang of ["en", "ko"] as const) {
      for (const code of [...AI_ERROR_CODES, ...REFRESH_FAILED_CODES, "SOMETHING_NEW"]) {
        for (const action of ["signIn", "signOut", "chat"] as const) {
          const r = refusals.ai(action, code);
          const why = translate(lang, r.why.key, r.why.params);
          expect(why).not.toMatch(/\{\w+\}/);
          const line = refusalLine(r);
          expect(translate(lang, line.key, line.params)).toBe(`${translate(lang, r.what.key)}. ${why}`);
        }
      }
    }
    expect(aiReason("refresh_token_expired").key).toBe("ai.reason.expired");
    expect(aiReason("AUTH_DENIED").key).toBe("ai.reason.AUTH_DENIED");
    expect(aiReason("X").key).toBe("ai.reason.unknown");
  });

  it("have a field label for every summary key a tool card shows", () => {
    const keys = ["executed", "compute_ms", "ion_count", "arrivals", "arrival_tof_us", "splat_counts", "converged", "cycles", "final_residual", "fallback_reason", "dims", "output_paths"];
    expect(keys.filter((k) => !(`ai.field.${k}` in en))).toEqual([]);
  });

  // When the bridge lives in this checkout (ai/*.mjs), every BridgeError code it can raise must be known.
  const bridge = new URL("../../../ai/", import.meta.url);
  it.skipIf(!existsSync(bridge))("know every code the bridge raises", () => {
    const src = readdirSync(bridge)
      .filter((f) => f.endsWith(".mjs"))
      .map((f) => readFileSync(new URL(f, bridge), "utf8"))
      .join("\n");
    const codes = [...new Set([...src.matchAll(/BridgeError\('([A-Z_]+)'/g)].map((m) => m[1]!))];
    expect(codes.length).toBeGreaterThan(5);
    expect(codes.filter((c) => !(AI_ERROR_CODES as readonly string[]).includes(c) && c !== "METHOD_NOT_FOUND")).toEqual([]);
  });
});
