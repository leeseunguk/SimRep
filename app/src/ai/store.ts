import { create } from "zustand";
import { dirname } from "../state/project";
import { useApp } from "../state/store";
import { applyAuthEvent, deviceCodeOf, EMPTY_ACCOUNTS, providerState, setNotice, setStatus, startSignIn, usableAccounts, type AccountsState } from "./accounts";
import { addError, addUser, applyEvent, EMPTY_CHAT, resolveApproval, type ChatState } from "./chat";
import { continueSummary, deleteChat as removeSavedChat, historyDataDir, listChats, saveChat, serializeChat, transcriptOf, type SavedChat } from "./history";
import { registerTranscript } from "../report/transcript";
import { displayLabel } from "../components/ai/BrandMarks";
import { onAiEvent, request } from "./client";
import { AiError, type ApprovalMode, type Provider } from "./protocol";
import { PROVIDER_LIST, installProviderMetadata, providerIds, providerInfo } from "./providers";

const STORAGE_WELCOMED = "simrep.ai.chatgptWelcomed";
const STORAGE_PROVIDER = "simrep.ai.provider";
/** Set only when the user ticked "remember" in the bypass confirmation; otherwise a launch asks. */
const STORAGE_BYPASS = "simrep.ai.approvalBypass";

interface AiState {
  accounts: AccountsState;
  chat: ChatState;
  settingsOpen: boolean;
  panelOpen: boolean;
  provider: Provider;
  /** "" = the provider's default model. */
  model: string;
  /** ChatGPT account id the chat uses ("" = the first usable one). */
  account: string;
  /** chat.start is in flight. */
  starting: boolean;
  /** Request in flight per provider (sign-in start, sign-out), so its buttons stay disabled. */
  pending: Record<Provider, boolean | undefined>;
  /** The first sign-in with ChatGPT plan usage shows a welcome card once (SIWC UI guidelines). */
  welcome: boolean;
  loaded: boolean;
  /** The panel's approval choice: ask for each write tool, or bypass (run without asking). */
  approval: "ask" | "bypass";
  /** Saved identity of the active chat (ai/<id>.json); null until its first message. */
  saved: ActiveChat | null;
  /** The open project's saved chats, newest first, and whether the project has a data folder. */
  history: SavedChat[];
  historyReady: boolean;
  /** The panel shows the previous-chats list instead of the transcript. */
  historyOpen: boolean;
  /** A saved chat opened read-only (the transcript shows it instead of the active chat). */
  viewing: SavedChat | null;
}

interface ActiveChat {
  id: string;
  createdAt: number;
  /** The project data folder the chat belongs to (null: not saved yet). */
  dataDir: string | null;
  /** Continue: the saved chat this one started from, and the summary the next chat.start sends. */
  continuedFrom?: { id: string; title: string };
  context?: string;
}

export const useAi = create<AiState>()(() => ({
  accounts: EMPTY_ACCOUNTS,
  chat: EMPTY_CHAT,
  settingsOpen: false,
  panelOpen: false,
  provider: localStorage.getItem(STORAGE_PROVIDER) ?? PROVIDER_LIST[0]!.id,
  model: "",
  account: "",
  starting: false,
  pending: {},
  welcome: false,
  loaded: false,
  approval: localStorage.getItem(STORAGE_BYPASS) === "1" ? "bypass" : "ask",
  saved: null,
  history: [],
  historyReady: false,
  historyOpen: false,
  viewing: null,
}));

const set = useAi.setState;
const get = useAi.getState;
const code = (err: unknown) => (err instanceof AiError ? err.code : "PROVIDER_ERROR");

onAiEvent((e) => {
  const s = get();
  const accounts = applyAuthEvent(s.accounts, e);
  const chat = applyEvent(s.chat, e);
  // First success with plan usage of a provider that asks for it (ChatGPT): the one-time welcome.
  const welcomed = localStorage.getItem(STORAGE_WELCOMED) === "1";
  const finished = providerState(accounts, e.provider).notice?.kind === "success" && providerState(s.accounts, e.provider).notice?.kind === "signingIn";
  const welcome = s.welcome || (!welcomed && finished && providerInfo(e.provider).planWelcome === true);
  set({ accounts, chat, welcome });
  // Autosave after each turn of the active chat.
  if (e.type === "done" && chat !== s.chat) void autosave();
});

/** The project folder the assistant works in: the project's folder, else the saved file's folder. */
export function workspaceOf(project = useApp.getState().project): string | null {
  return project.dir ?? (project.path ? dirname(project.path) : null);
}

export async function loadAccounts(): Promise<void> {
  try {
    const catalog = await request("auth.providers", {});
    installProviderMetadata(catalog.providers);
    const { providers } = await request("auth.status", {});
    let accounts = get().accounts;
    for (const p of providers) accounts = setStatus(accounts, p);
    set({ accounts, loaded: true });
  } catch (err) {
    let accounts = get().accounts;
    for (const p of providerIds()) accounts = setNotice(accounts, p, { kind: "failed", action: "signIn", code: code(err) });
    set({ accounts, loaded: true });
  }
}

/** Starts a sign-in; it finishes with an `auth` or `error` event. `account` reauthorizes a saved ChatGPT registration. */
export async function signIn(provider: Provider, account?: string, region?: string): Promise<void> {
  set({ accounts: startSignIn(get().accounts, provider), pending: { ...get().pending, [provider]: true } });
  try {
    const started = await request("auth.login", { provider, ...(account && { account }), ...(region && { region }) });
    // Device authorization normally arrives as a `device_code` event; a code in the login result
    // is shown too, unless the event already put one up while the sign-in is still waiting.
    const device = deviceCodeOf(started);
    const notice = providerState(get().accounts, provider).notice;
    if (device && notice?.kind === "signingIn" && !notice.device) set({ accounts: setNotice(get().accounts, provider, { kind: "signingIn", device }) });
  } catch (err) {
    set({ accounts: setNotice(get().accounts, provider, { kind: "failed", action: "signIn", code: code(err) }) });
  } finally {
    set({ pending: { ...get().pending, [provider]: false } });
  }
}

/** Only the Rust host opens the fixed provider URL, never a WebView-provided URL. */
export async function openUsage(provider: Provider): Promise<void> {
  try { await request("auth.openUsage", { provider }); }
  catch (err) { set({ accounts: setNotice(get().accounts, provider, { kind: "failed", action: "signIn", code: code(err) }) }); }
}

export async function signOut(provider: Provider, account?: string): Promise<void> {
  set({ pending: { ...get().pending, [provider]: true } });
  try {
    const r = await request("auth.logout", { provider, ...(account && { account }) });
    set({ accounts: setNotice(get().accounts, provider, { kind: "signedOut", remoteConfirmed: r.remoteRevocationConfirmed }) });
    // The chat of a signed-out provider cannot continue.
    if (get().chat.session?.provider === provider) set({ chat: EMPTY_CHAT });
    const { providers } = await request("auth.status", { provider });
    let accounts = get().accounts;
    for (const p of providers) accounts = setStatus(accounts, p);
    set({ accounts });
  } catch (err) {
    set({ accounts: setNotice(get().accounts, provider, { kind: "failed", action: "signOut", code: code(err) }) });
  } finally {
    set({ pending: { ...get().pending, [provider]: false } });
  }
}

export function dismissWelcome(): void {
  localStorage.setItem(STORAGE_WELCOMED, "1");
  set({ welcome: false });
}

export function setProvider(provider: Provider): void {
  localStorage.setItem(STORAGE_PROVIDER, provider);
  set({ provider, model: providerInfo(provider).models[0] ?? "", account: "" });
}

/** The bridge mode for the current chat: bypass from the panel, else "allow all in this chat", else ask. */
export function approvalModeOf(s: Pick<AiState, "approval" | "chat"> = get()): ApprovalMode {
  return s.approval === "bypass" ? "bypass" : s.chat.allowAll ? "session" : "ask";
}

async function syncApprovalMode(): Promise<void> {
  const session = get().chat.session;
  if (!session) return;
  try {
    await request("chat.setApprovalMode", { sessionId: session.id, mode: approvalModeOf() });
  } catch (err) {
    set({ chat: addError(get().chat, session.provider, code(err)) });
  }
}

/**
 * The panel's approval switch. Bypass (after its confirmation) is remembered across launches only
 * with `remember`; Ask also ends "allow all in this chat".
 */
export async function setApproval(approval: "ask" | "bypass", remember = false): Promise<void> {
  if (approval === "bypass" && remember) localStorage.setItem(STORAGE_BYPASS, "1");
  else localStorage.removeItem(STORAGE_BYPASS);
  set({ approval, chat: approval === "ask" ? { ...get().chat, allowAll: false } : get().chat });
  await syncApprovalMode();
}

/** "Allow all in this chat": this approval and every later one of the chat run without asking. */
export async function allowAllInChat(approvalId: string): Promise<void> {
  set({ chat: { ...get().chat, allowAll: true } });
  await syncApprovalMode();
  await answerApproval(approvalId, true);
}

/** Starts a chat (if the provider, model, account or workspace changed) and sends `text` as one turn. */
export async function send(text: string): Promise<void> {
  const s = get();
  const workspace = workspaceOf();
  if (!text.trim() || s.chat.running || s.starting || !workspace) return;
  const account = s.account || usableAccounts(s.accounts[s.provider])[0]?.id;
  let session = s.chat.session;
  const stale = !session || session.provider !== s.provider || session.account !== account || session.workspace !== workspace || (s.model && session.model !== s.model);
  const saved = s.saved ?? { id: crypto.randomUUID(), createdAt: Date.now(), dataDir: historyDataDir() };
  set({ saved, viewing: null, historyOpen: false, chat: addUser(stale ? { ...EMPTY_CHAT, nextId: s.chat.nextId, items: s.chat.items, ...(s.chat.activeSkill && { activeSkill: s.chat.activeSkill }) } : s.chat, text, Date.now()) });
  try {
    if (stale) {
      set({ starting: true });
      // A continued chat's new provider session gets the old chat's summary as its first context.
      const context = get().saved?.context;
      const started = await request("chat.start", { provider: s.provider, workspace, language: useApp.getState().lang, approvalMode: approvalModeOf(get()), ...(s.model && { model: s.model }), ...(account && { account }), ...(context && { context }) });
      session = { id: started.sessionId, provider: started.provider, model: started.model, workspace: started.workspace, account };
      const current = get().saved;
      set({ chat: { ...get().chat, session }, saved: current && { ...current, context: undefined } });
    }
    await request("chat.send", { sessionId: session!.id, text });
  } catch (err) {
    set({ chat: { ...addError(get().chat, s.provider, code(err)), running: false } });
  } finally {
    set({ starting: false });
  }
}

/** Cancels the running turn; it still ends with its one `done` ("interrupted"). */
export async function cancel(): Promise<void> {
  const session = get().chat.session;
  if (!session || !get().chat.running) return;
  try {
    await request("chat.cancel", { sessionId: session.id });
  } catch (err) {
    set({ chat: addError(get().chat, session.provider, code(err)) });
  }
}

export async function answerApproval(approvalId: string, allow: boolean): Promise<void> {
  set({ chat: resolveApproval(get().chat, approvalId, allow) });
  try {
    await request("approval.resolve", { approvalId, allow });
  } catch (err) {
    const provider = get().chat.session?.provider ?? get().provider;
    set({ chat: addError(get().chat, provider, code(err)) });
  }
}

export function newChat(): void {
  if (get().chat.running) return;
  set({ chat: { ...EMPTY_CHAT, nextId: get().chat.nextId }, saved: null, viewing: null, historyOpen: false });
}

// ------------------------------------------------------------------------------ chat history

/** The active chat as its saved, redacted record (what autosave writes). */
function activeRecord(chat: ChatState, saved: ActiveChat): SavedChat {
  const provider = chat.session?.provider ?? get().provider;
  return serializeChat(chat, {
    id: saved.id,
    provider,
    providerLabel: displayLabel(provider, providerInfo(provider).label),
    model: chat.session?.model ?? get().model,
    createdAt: saved.createdAt,
    updatedAt: Date.now(),
    ...(saved.continuedFrom && { continuedFrom: saved.continuedFrom }),
  });
}

// The report's AI appendix: the saved chat open read-only, else the active chat, redacted exactly
// like a saved chat (serializeChat).
registerTranscript(() => {
  const { viewing, chat, saved } = get();
  if (viewing) return transcriptOf(viewing, useApp.getState().lang);
  if (!chat.items.some((i) => i.kind === "user")) return null;
  return transcriptOf(activeRecord(chat, saved ?? { id: "active", createdAt: Date.now(), dataDir: null }), useApp.getState().lang);
});

/** Saves the active chat to the open project's `ai/<id>.json`; failures go to the log. */
export async function autosave(): Promise<void> {
  const { chat, saved } = get();
  if (!saved || !chat.items.some((i) => i.kind === "user")) return;
  const dir = historyDataDir();
  // A chat belongs to the project it started in; one started before the first save joins it.
  if (!dir || (saved.dataDir !== null && saved.dataDir !== dir)) return;
  if (saved.dataDir === null) set({ saved: { ...saved, dataDir: dir } });
  const record = activeRecord(chat, saved);
  try {
    await saveChat(record);
    set({ history: [record, ...get().history.filter((c) => c.id !== record.id)] });
  } catch (err) {
    useApp.getState().log("error", { key: "log.aiChatSaveFailed", params: { reason: String(err) } });
  }
}

/** Reloads the open project's saved chats. */
export async function refreshHistory(): Promise<void> {
  try {
    const ready = historyDataDir() !== null;
    set({ history: ready ? await listChats() : [], historyReady: ready });
  } catch (err) {
    set({ history: [], historyReady: false });
    useApp.getState().log("error", { key: "log.aiChatListFailed", params: { reason: String(err) } });
  }
}

export function openHistory(open: boolean): void {
  set({ historyOpen: open, ...(open && { viewing: null }) });
  if (open) void refreshHistory();
}

/** Opens a saved chat read-only. */
export function viewChat(chat: SavedChat | null): void {
  set({ viewing: chat, historyOpen: chat === null && get().historyOpen });
}

export async function deleteSavedChat(id: string): Promise<void> {
  try {
    await removeSavedChat(id);
  } catch (err) {
    useApp.getState().log("error", { key: "log.aiChatDeleteFailed", params: { reason: String(err) } });
    return;
  }
  set({ history: get().history.filter((c) => c.id !== id), ...(get().viewing?.id === id && { viewing: null, historyOpen: true }), ...(get().saved?.id === id && { saved: null }) });
}

/**
 * Continue a saved chat: provider sessions cannot be resumed, so this starts a new chat whose
 * provider session (at its first message) gets a short, bounded summary of the old one.
 */
export function continueChat(from: SavedChat): void {
  if (get().chat.running || get().starting) return;
  set({
    chat: { ...EMPTY_CHAT, nextId: get().chat.nextId },
    saved: { id: crypto.randomUUID(), createdAt: Date.now(), dataDir: null, continuedFrom: { id: from.id, title: from.title }, context: continueSummary(from) },
    viewing: null,
    historyOpen: false,
  });
}

export function openSettings(open: boolean): void {
  set({ settingsOpen: open });
  if (open && !get().loaded) void loadAccounts();
}

export function openPanel(open: boolean): void {
  set({ panelOpen: open });
  if (open && !get().loaded) void loadAccounts();
}
