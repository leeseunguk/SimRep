import { describe, expect, it } from "vitest";
import { addUser, EMPTY_CHAT } from "./chat";
import { compactArgs, parseChat, redact, redactText, serializeChat, transcriptOf } from "./history";

const meta = { id: "security-chat-123", provider: "chatgpt", providerLabel: "ChatGPT", model: "model", createdAt: 1, updatedAt: 2 };
const attacks = [
  ["account_id=chatgpt-acct-123", "chatgpt-acct-123"],
  ["Authorization: Basic YWJjZGVmOnBhc3N3b3Jk", "YWJjZGVmOnBhc3N3b3Jk"],
  ["client_secret=abc", "abc"],
  ['{"AcCoUnTiD":"identity-123"}', "identity-123"],
  ['{"USER_ID": "person-42"}', "person-42"],
  ['{"clientId": "client-42", "clientSecret": "s"}', "client-42"],
  ["authorization: bEaReR x", "bEaReR x"],
  ["Cookie: sid=short; theme=dark", "short"],
  ["Set-Cookie: sid=short; HttpOnly", "short"],
  ["X-API-Key: k", "Key: k"],
  ["api_key=a password=b refresh_token=c id_token=d access_token=e session=f", "session=f"],
  ['{"password":"a b c","session":"tiny"}', "a b c"],
] as const;

describe("identity and credential redaction boundaries", () => {
  for (const [text, secret] of attacks) {
    it(`redacts ${text.split(/[:=]/)[0]} on save, read and report export`, () => {
      const raw = { ...serializeChat(EMPTY_CHAT, meta), items: [{ kind: "user" as const, text }] };
      const saved = serializeChat(addUser(EMPTY_CHAT, text), meta);
      for (const result of [redactText(text), JSON.stringify(saved), JSON.stringify(parseChat(JSON.stringify(raw))), JSON.stringify(transcriptOf(raw, "en"))]) {
        expect(result).not.toContain(secret);
        expect(result).toContain("[redacted]");
      }
    });
  }

  it("drops mixed-case identity and credential object keys", () => {
    expect(redact({ accountId: "a", USER_ID: "b", session: "c", clientId: "d", CLIENT_SECRET: "e", label: "ok" })).toEqual({ label: "ok" });
  });

  it("removes the current session identity even without an assignment label", () => {
    const session = { id: "s1", provider: "chatgpt", model: "model", workspace: "/w", account: "chatgpt-acct-123" };
    const saved = serializeChat(addUser({ ...EMPTY_CHAT, session }, "Using chatgpt-acct-123 for this run"), meta);
    expect(JSON.stringify(saved)).not.toContain(session.account);
    expect(JSON.stringify(transcriptOf(saved, "en"))).not.toContain(session.account);
  });

  it("redacts entire credentials before clipping argument and message strings", () => {
    const token = `sk-proj-${"a".repeat(200)}`;
    expect(compactArgs({ value: token })).toEqual({ value: "[redacted]" });
    const text = `${"x".repeat(19_990)} ${token}`;
    expect(JSON.stringify(serializeChat(addUser(EMPTY_CHAT, text), meta))).not.toContain("sk-proj-");
  });
});
