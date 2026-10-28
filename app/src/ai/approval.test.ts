import { beforeEach, describe, expect, it, vi } from "vitest";
import type { Method, Methods } from "./protocol";

// The store against a recorded bridge: which approval mode chat.start and chat.setApprovalMode get.
const calls: { method: Method; params: unknown }[] = [];
vi.mock("./client", () => ({
  onAiEvent: () => {},
  request: async <M extends Method>(method: M, params: Methods[M]["params"]) => {
    calls.push({ method, params });
    if (method === "chat.setApprovalMode") {
      const p = params as Methods["chat.setApprovalMode"]["params"];
      return { sessionId: p.sessionId, approvalMode: p.mode };
    }
    return { resolved: true };
  },
}));

// The project module pulls in the 3D import pipeline (DOM listeners); the store needs only dirname.
vi.mock("../state/project", () => ({ dirname: (p: string) => p.replace(/[\\/][^\\/]*$/, "") }));

const STORAGE = "simrep.ai.approvalBypass";
const session = { id: "s1", provider: "chatgpt", model: "m", workspace: "/w" };

async function freshStore() {
  vi.resetModules();
  return import("./store");
}

describe("approval mode", () => {
  beforeEach(() => {
    calls.length = 0;
    localStorage.clear();
  });

  it("starts in Ask unless bypass was remembered", async () => {
    expect((await freshStore()).useAi.getState().approval).toBe("ask");
    localStorage.setItem(STORAGE, "1");
    expect((await freshStore()).useAi.getState().approval).toBe("bypass");
  });

  it("remembers bypass only when asked to and tells the live session", async () => {
    const store = await freshStore();
    store.useAi.setState({ chat: { ...store.useAi.getState().chat, session } });
    await store.setApproval("bypass");
    expect(localStorage.getItem(STORAGE)).toBeNull();
    expect(store.approvalModeOf()).toBe("bypass");
    await store.setApproval("bypass", true);
    expect(localStorage.getItem(STORAGE)).toBe("1");
    await store.setApproval("ask");
    expect(localStorage.getItem(STORAGE)).toBeNull();
    expect(calls.map((c) => c.params)).toEqual([
      { sessionId: "s1", mode: "bypass" },
      { sessionId: "s1", mode: "bypass" },
      { sessionId: "s1", mode: "ask" },
    ]);
  });

  it("allow all in this chat switches the session to session mode before allowing the approval", async () => {
    const store = await freshStore();
    store.useAi.setState({ chat: { ...store.useAi.getState().chat, session } });
    await store.allowAllInChat("a1");
    expect(store.approvalModeOf()).toBe("session");
    expect(calls).toEqual([
      { method: "chat.setApprovalMode", params: { sessionId: "s1", mode: "session" } },
      { method: "approval.resolve", params: { approvalId: "a1", allow: true } },
    ]);
    // A new chat asks again; Ask from the panel also ends it.
    store.newChat();
    expect(store.approvalModeOf()).toBe("ask");
  });
});
