import { beforeEach, describe, expect, it, vi } from "vitest";
import type { Method, Methods } from "./protocol";

// The store against a recorded bridge and an in-memory project data folder: autosave after a
// turn, list, and Continue sending the bounded summary as chat.start's `context`.
const calls: { method: Method; params: Record<string, unknown> }[] = [];
let emit: (e: unknown) => void = () => {};
vi.mock("./client", () => ({
  onAiEvent: (fn: (e: unknown) => void) => (emit = fn),
  request: async <M extends Method>(method: M, params: Methods[M]["params"]) => {
    calls.push({ method, params: params as Record<string, unknown> });
    if (method === "chat.start") return { sessionId: `s${calls.length}`, provider: "chatgpt", model: "gpt-6.1-sol", workspace: "/w" };
    return { turnId: "t1" };
  },
}));
const files = new Map<string, string>();
/** Resolves on the next file write (subscribed before the event that causes it). */
let wrote: () => void = () => {};
const nextWrite = () => new Promise<void>((r) => (wrote = r));
vi.mock("../state/projectDir", () => ({
  projectDataDir: () => "/w/p.simrep.d",
  writeProjectFile: async (rel: string, data: string) => {
    files.set(rel, data);
    wrote();
    return true;
  },
  readProjectText: async (rel: string) => files.get(rel) ?? null,
  listProjectFiles: async (prefix: string) => [...files.keys()].filter((k) => k.startsWith(`${prefix}/`)).map((path) => ({ path, size: 0 })),
  deleteProjectFile: async (rel: string) => files.delete(rel),
}));
vi.mock("../state/project", () => ({ dirname: (p: string) => p.replace(/[\\/][^\\/]*$/, "") }));

async function freshStore() {
  vi.resetModules();
  const store = await import("./store");
  const { useApp } = await import("../state/store");
  useApp.setState({ project: { name: "p", nameKey: null, path: "/w/p.simrep", dir: "/w" } });
  store.useAi.setState({ accounts: { chatgpt: { status: { provider: "chatgpt", authenticated: true, accounts: [{ id: "g1", provider: "chatgpt", label: "me", authenticated: true, planEnabled: true }] } } } as never });
  return store;
}

const done = async () => {
  const written = nextWrite();
  emit({ type: "done", provider: "chatgpt", sessionId: calls.find((c) => c.method === "chat.start") ? `s${calls.findLastIndex((c) => c.method === "chat.start") + 1}` : "", status: "completed" });
  await written;
  // autosave updates the in-memory list right after the write resolves.
  await Promise.resolve();
};

describe("chat history in the store", () => {
  beforeEach(() => {
    calls.length = 0;
    files.clear();
  });

  it("autosaves after a turn, lists it, deletes it, and Continue starts a new session with the summary", async () => {
    const store = await freshStore();
    await store.send("Fly the einzel ions");
    await done();
    const [rel] = [...files.keys()];
    expect(rel).toMatch(/^ai\/[0-9a-f-]{36}\.json$/);
    expect(JSON.parse(files.get(rel!)!)).toMatchObject({ title: "Fly the einzel ions", provider: "chatgpt", model: "gpt-6.1-sol" });
    expect(JSON.stringify(files.get(rel!))).not.toContain("g1");

    await store.refreshHistory();
    const [saved] = store.useAi.getState().history;
    expect(saved?.title).toBe("Fly the einzel ions");

    store.continueChat(saved!);
    expect(store.useAi.getState().saved?.continuedFrom).toEqual({ id: saved!.id, title: saved!.title });
    await store.send("and again");
    const starts = calls.filter((c) => c.method === "chat.start");
    expect(starts).toHaveLength(2);
    expect(starts[0]!.params.context).toBeUndefined();
    expect(String(starts[1]!.params.context)).toContain("User: Fly the einzel ions");
    expect(String(starts[1]!.params.context).length).toBeLessThanOrEqual(4000);
    await done();
    expect(files.size).toBe(2);
    const second = [...files.values()].map((t) => JSON.parse(t)).find((c) => c.id !== saved!.id);
    expect(second.continuedFrom).toEqual({ id: saved!.id, title: saved!.title });

    await store.deleteSavedChat(saved!.id);
    expect(files.has(rel!)).toBe(false);
    expect(store.useAi.getState().history.map((c) => c.id)).not.toContain(saved!.id);
  });

  it("offers the active chat, or the saved chat open read-only, to the report appendix", async () => {
    const store = await freshStore();
    const { reportTranscript } = await import("../report/transcript");
    expect(reportTranscript()).toBeNull();
    await store.send("Fly the einzel ions, key sk-proj-abcdefghijklmnopqrstuvwx");
    await done();
    const active = reportTranscript();
    expect(active?.[0]).toEqual({ role: "user", text: "Fly the einzel ions, key [redacted]" });
    await store.refreshHistory();
    const [saved] = store.useAi.getState().history;
    store.newChat();
    expect(reportTranscript()).toBeNull();
    store.viewChat({ ...saved!, items: [{ kind: "user", text: "An older chat" }] });
    expect(reportTranscript()).toEqual([{ role: "user", text: "An older chat" }]);
  });
});
