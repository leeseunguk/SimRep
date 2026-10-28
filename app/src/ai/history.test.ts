import { describe, expect, it } from "vitest";
import { addUser, applyEvent, EMPTY_CHAT, resolveApproval, type ChatState } from "./chat";
import { compactArgs, continueSummary, parseChat, redact, serializeChat, SUMMARY_MAX, toChatState, transcriptOf, type SavedChat } from "./history";
import { parseEvent, type AiEvent } from "./protocol";

const JWT = "eyJhbGciOiJSUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.c2lnbmF0dXJlLXNpZ25hdHVyZQ";
const ev = (e: Record<string, unknown>) => parseEvent({ provider: "chatgpt", sessionId: "s1", turnId: "t1", ...e }) as AiEvent;
const run = (s: ChatState, ...events: Record<string, unknown>[]) => events.reduce((acc, e) => applyEvent(acc, ev(e)), s);
const meta = { id: "0b6f1c1e-2c1a-4f7e-9d7a-1c2b3d4e5f60", provider: "chatgpt", providerLabel: "ChatGPT", model: "gpt-6.1-sol", createdAt: 1_000, updatedAt: 2_000 };
const session = { id: "s1", provider: "chatgpt", model: "gpt-6.1-sol", workspace: "/w", account: "chatgpt-acct-123" };

/** A finished chat: a read-only tool, an asked approval, an auto-approved write tool, an error. */
function sampleChat(): ChatState {
  let s: ChatState = { ...EMPTY_CHAT, session };
  s = addUser(s, "Fly the einzel ions; my key is sk-proj-abcdefghijklmnopqrstuvwx and mail me at researcher@example.com", 1_500);
  s = run(
    s,
    { type: "delta", text: "Checking the array.", activeSkill: "einzel" },
    { type: "tool_call", callId: "c1", name: "pa_info", arguments: { pa: "einzel.pa0", access_token: "plain-secret-value", nested: { a: { b: { c: 1 } } } } },
    { type: "tool_result", callId: "c1", result: { content: [{ type: "text", text: JSON.stringify({ dims: [201, 41, 1], compute_ms: 1.5 }) }] } },
    { type: "approval", approvalId: "a1", name: "fly", arguments: { pa: "einzel.pa0", contents: "x".repeat(10_000) } },
  );
  s = resolveApproval(s, "a1", true);
  s = run(
    s,
    { type: "tool_call", callId: "c2", name: "refine", arguments: { pa: "einzel.pa#" } },
    { type: "approval", approvalId: "a2", name: "refine", arguments: {}, auto: true },
    { type: "tool_result", callId: "c2", result: { content: [{ type: "text", text: `Authorization: Bearer ${JWT}` }], isError: true } },
    { type: "error", code: "PROVIDER_ERROR", message: "boom" },
    { type: "done", status: "completed" },
  );
  return s;
}

/** Token, JWT, e-mail and account-id patterns that must never appear in a saved chat. */
const LEAKS = [/eyJ[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+/, /\bsk-[A-Za-z0-9_-]{8,}/, /[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}/, /Bearer\s+(?!\[redacted\])/, /access_token|refresh_token|id_token/, /plain-secret-value/, /chatgpt-acct-123/];

describe("saved AI chats", () => {
  it("serializes messages, tool calls, approvals (incl. auto), provider, model, recipe and times", () => {
    const saved = serializeChat(sampleChat(), meta);
    expect(saved).toMatchObject({ format: "simrep-ai-chat", version: 1, id: meta.id, provider: "chatgpt", providerLabel: "ChatGPT", model: "gpt-6.1-sol", recipe: "einzel", createdAt: 1_000, updatedAt: 2_000 });
    expect(saved.items.map((i) => i.kind)).toEqual(["user", "assistant", "tool", "approval", "tool", "error"]);
    expect(saved.items[0]).toMatchObject({ kind: "user", at: 1_500 });
    expect(saved.items[2]).toMatchObject({ kind: "tool", name: "pa_info", status: "done", result: { fields: [{ key: "compute_ms", value: "1.5" }, { key: "dims", value: "201 × 41 × 1" }] } });
    expect(saved.items[4]).toMatchObject({ kind: "tool", name: "refine", status: "error", auto: true });
    expect(saved.approvals).toEqual([{ name: "fly", decision: "allowed" }, { name: "refine", decision: "auto" }]);
    expect(saved.title.startsWith("Fly the einzel ions")).toBe(true);
  });

  it("redacts tokens, JWTs, e-mails and credential keys, and stores no account or file contents", () => {
    const text = JSON.stringify(serializeChat(sampleChat(), meta));
    for (const re of LEAKS) expect(text, String(re)).not.toMatch(re);
    expect(text).not.toContain("x".repeat(200));
    expect(text).toContain("[redacted]");
    expect(text).toContain("[redacted email]");
    expect(redact({ email: "a@b.co", account: "id-1", accountId: "id-2", label: "ok", nested: [{ refresh_token: "r" }] })).toEqual({ label: "ok", nested: [{}] });
  });

  it("compacts tool arguments: long strings clipped, deep values and big arrays summarized", () => {
    const args = compactArgs({ s: "y".repeat(500), n: 1, deep: { a: { b: { c: 1 } } }, big: Array.from({ length: 40 }, (_, i) => i), small: [1, 2] });
    expect((args.s as string).length).toBeLessThanOrEqual(160);
    expect(args.deep).toEqual({ a: {} });
    expect(args.big).toBe("[40 items]");
    expect(args.small).toEqual([1, 2]);
  });

  it("round-trips through JSON and opens read-only without a session", () => {
    const saved = serializeChat(sampleChat(), meta);
    const back = parseChat(JSON.stringify(saved));
    expect(back).toEqual(saved);
    const view = toChatState(back!);
    expect(view.session).toBeNull();
    expect(view.running).toBe(false);
    expect(view.activeSkill).toBe("einzel");
    expect(view.items.map((i) => i.kind)).toEqual(["user", "assistant", "tool", "approval", "tool", "error"]);
    expect(view.items.every((i) => i.kind !== "approval" || i.state !== "pending")).toBe(true);
  });

  it("closes open approvals and running tools when an unfinished chat is saved", () => {
    const s = run(addUser({ ...EMPTY_CHAT, session }, "go"), { type: "tool_call", callId: "c1", name: "fly", arguments: {} }, { type: "approval", approvalId: "a1", name: "fly", arguments: {} });
    const saved = serializeChat(s, meta);
    expect(saved.items.map((i) => ("status" in i ? i.status : "state" in i ? i.state : i.kind))).toEqual(["user", "skipped", "expired"]);
  });

  it("rejects files that are not saved chats and re-redacts what it reads", () => {
    expect(parseChat("not json")).toBeNull();
    expect(parseChat(JSON.stringify({ format: "other", version: 1 }))).toBeNull();
    expect(parseChat(JSON.stringify({ ...serializeChat(sampleChat(), meta), id: "../escape" }))).toBeNull();
    const tampered = { ...serializeChat(sampleChat(), meta), items: [{ kind: "user", text: `token ${JWT}` }, { kind: "bogus" }], email: "x@y.zz" };
    const back = parseChat(JSON.stringify(tampered))!;
    expect(back.items).toEqual([{ kind: "user", text: "token [redacted]" }]);
    expect(JSON.stringify(back)).not.toMatch(LEAKS[0]!);
  });

  it("builds a continue summary bounded by its limit, keeping the first request and the latest lines", () => {
    const saved = serializeChat(sampleChat(), meta);
    const short = continueSummary(saved);
    expect(short.length).toBeLessThanOrEqual(SUMMARY_MAX);
    expect(short).toContain("User: Fly the einzel ions");
    expect(short).toContain("Tool refine [error, auto-approved]");
    expect(short).toContain("cannot be resumed");

    let long: ChatState = { ...EMPTY_CHAT, session };
    for (let n = 0; n < 200; n++) long = run(addUser(long, `request ${n} ${"z".repeat(500)}`), { type: "delta", text: `answer ${n} ${"w".repeat(900)}` }, { type: "done", status: "completed" });
    const big: SavedChat = serializeChat(long, meta);
    for (const max of [SUMMARY_MAX, 1200, 300, 50]) expect(continueSummary(big, max).length).toBeLessThanOrEqual(max);
    const summary = continueSummary(big);
    expect(summary).toContain("User: request 0 ");
    expect(summary).toContain("Assistant: answer 199 ");
    expect(summary).toContain("\n…\n");
    expect(summary).not.toContain("request 100 ");
  });

  it("turns a saved chat into the report's transcript appendix, redacted like the saved file", () => {
    const items = transcriptOf(serializeChat(sampleChat(), meta), "ko");
    expect(items.map((i) => i.role)).toEqual(["user", "assistant", "tool", "system", "tool", "system"]);
    expect(items[2]).toMatchObject({ role: "tool", tool: "pa_info" });
    expect(items[2]!.text).toContain("dims: 201 × 41 × 1");
    const text = JSON.stringify(items);
    for (const re of LEAKS) expect(text, String(re)).not.toMatch(re);
    expect(text).toContain("[redacted email]");
  });
});
