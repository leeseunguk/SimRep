import { create } from "zustand";
import { dirname } from "../state/project";
import { useApp } from "../state/store";
import { applyAuthEvent, EMPTY_ACCOUNTS, providerState, setNotice, setStatus, startSignIn, usableAccounts, type AccountsState } from "./accounts";
import { addError, addUser, applyEvent, EMPTY_CHAT, resolveApproval, type ChatState } from "./chat";
import { onAiEvent, request } from "./client";
import { AiError, type Provider } from "./protocol";
import { PROVIDER_LIST, providerIds, providerInfo } from "./providers";

const STORAGE_WELCOMED = "simrep.ai.chatgptWelcomed";
const STORAGE_PROVIDER = "simrep.ai.provider";

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
});

/** The project folder the assistant works in: the project's folder, else the saved file's folder. */
export function workspaceOf(project = useApp.getState().project): string | null {
  return project.dir ?? (project.path ? dirname(project.path) : null);
}

export async function loadAccounts(): Promise<void> {
  try {
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
export async function signIn(provider: Provider, account?: string): Promise<void> {
  set({ accounts: startSignIn(get().accounts, provider), pending: { ...get().pending, [provider]: true } });
  try {
    await request("auth.login", { provider, ...(account && { account }) });
  } catch (err) {
    set({ accounts: setNotice(get().accounts, provider, { kind: "failed", action: "signIn", code: code(err) }) });
  } finally {
    set({ pending: { ...get().pending, [provider]: false } });
  }
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

/** Starts a chat (if the provider, model, account or workspace changed) and sends `text` as one turn. */
export async function send(text: string): Promise<void> {
  const s = get();
  const workspace = workspaceOf();
  if (!text.trim() || s.chat.running || s.starting || !workspace) return;
  const account = s.account || usableAccounts(s.accounts[s.provider])[0]?.id;
  let session = s.chat.session;
  const stale = !session || session.provider !== s.provider || session.workspace !== workspace || (s.model && session.model !== s.model);
  set({ chat: addUser(stale ? { ...EMPTY_CHAT, nextId: s.chat.nextId, items: s.chat.items } : s.chat, text) });
  try {
    if (stale) {
      set({ starting: true });
      const started = await request("chat.start", { provider: s.provider, workspace, language: useApp.getState().lang, ...(s.model && { model: s.model }), ...(account && { account }) });
      session = { id: started.sessionId, provider: started.provider, model: started.model, workspace: started.workspace };
      set({ chat: { ...get().chat, session } });
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
  set({ chat: { ...EMPTY_CHAT, nextId: get().chat.nextId } });
}

export function openSettings(open: boolean): void {
  set({ settingsOpen: open });
  if (open && !get().loaded) void loadAccounts();
}

export function openPanel(open: boolean): void {
  set({ panelOpen: open });
  if (open && !get().loaded) void loadAccounts();
}
