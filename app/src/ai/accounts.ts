import { providerInfo } from "./providers";
import { REFRESH_FAILED_CODES, type Account, type AiEvent, type Provider, type ProviderStatus } from "./protocol";

/**
 * Sign-in state per provider as a pure reducer (the settings page and its tests share it).
 * Only display fields live here: the bridge never sends tokens, and auth URLs stay in Rust.
 */
export type Notice =
  /** The browser (ChatGPT) or Claude Code's login opened; waiting for the `auth` or `error` event. */
  | { kind: "signingIn"; device?: DeviceCode }
  | { kind: "success"; account: string }
  /** Signed in, but ChatGPT plan usage was not granted (planEnabled false / PLAN_PERMISSION_REQUIRED). */
  | { kind: "notEligible"; account: string | null }
  | { kind: "signedOut"; remoteConfirmed: boolean }
  /** A failed sign-in, sign-out or chat start: the bridge code, explained via refusals.ts. */
  | { kind: "failed"; action: "signIn" | "signOut"; code: string };

/** A device authorization the user confirms at the provider: the code to enter and when it lapses. */
export interface DeviceCode {
  userCode: string;
  /** Epoch ms; null when the provider gave no lifetime. */
  expiresAt: number | null;
}

/**
 * The device code of an `auth.login` result, if the bridge sent one (`userCode`, optional
 * `expiresIn` seconds). Anything else in the result is ignored, so the plain `{started: true}`
 * of a browser sign-in gives null.
 */
export function deviceCodeOf(result: unknown, now = Date.now()): DeviceCode | null {
  if (typeof result !== "object" || result === null) return null;
  const r = result as { userCode?: unknown; expiresIn?: unknown };
  if (typeof r.userCode !== "string" || !/^[A-Za-z0-9-]{4,16}$/.test(r.userCode)) return null;
  const ttl = typeof r.expiresIn === "number" && r.expiresIn > 0 ? r.expiresIn : null;
  return { userCode: r.userCode, expiresAt: ttl === null ? null : now + ttl * 1000 };
}

/** What a provider card's status chip says, from its accounts and the latest sign-in. */
export type ProviderSummary = AccountKind | "expired" | "signingIn" | "none";

export function providerSummary(s: ProviderState): ProviderSummary {
  if (s.notice?.kind === "signingIn") return "signingIn";
  const kinds = new Set((s.status?.accounts ?? []).map(accountKind));
  for (const k of ["ready", "apiKey", "noPlan"] as const) if (kinds.has(k)) return k;
  if (s.notice?.kind === "failed" && isExpired(s.notice.code)) return "expired";
  return kinds.has("signedOut") ? "signedOut" : "none";
}

export interface ProviderState {
  status: ProviderStatus | null;
  notice: Notice | null;
}

/** Per provider id; a provider not in the map has no status yet. */
export type AccountsState = Record<Provider, ProviderState | undefined>;

export const EMPTY_ACCOUNTS: AccountsState = {};
const NONE: ProviderState = { status: null, notice: null };
export const providerState = (s: AccountsState, p: Provider): ProviderState => s[p] ?? NONE;

export type AccountKind =
  /** Signed in with plan usage (ChatGPT plan grant, Claude subscription login). */
  | "ready"
  /** Signed in, plan usage not granted, and the provider needs it (ChatGPT): no inference until reauthorized. */
  | "noPlan"
  /** Runs on an API key from the environment (e.g. ANTHROPIC_API_KEY, which logout cannot remove). */
  | "apiKey"
  /** A saved registration whose credential is gone (signed out, expired, refresh failed). */
  | "signedOut";

export function accountKind(a: Account): AccountKind {
  if (!a.authenticated) return "signedOut";
  if (a.planEnabled) return "ready";
  return providerInfo(a.provider).withoutPlan === "blocked" ? "noPlan" : "apiKey";
}

/** Accounts a chat can use, the protocol's default (first authenticated) first. */
export const usableAccounts = (s: ProviderState = NONE) => (s.status?.accounts ?? []).filter((a) => accountKind(a) === "ready" || accountKind(a) === "apiKey");

/** A token-endpoint refusal of a refresh, or an expired session: the user must sign in again. */
export const isExpired = (code: string) => code === "AUTH_REQUIRED" || code === "AUTH_TIMEOUT" || (REFRESH_FAILED_CODES as readonly string[]).includes(code);

export function setStatus(s: AccountsState, status: ProviderStatus): AccountsState {
  return { ...s, [status.provider]: { ...providerState(s, status.provider), status } };
}

export function startSignIn(s: AccountsState, provider: Provider): AccountsState {
  return { ...s, [provider]: { ...providerState(s, provider), notice: { kind: "signingIn" } } };
}

export function setNotice(s: AccountsState, provider: Provider, notice: Notice | null): AccountsState {
  return { ...s, [provider]: { ...providerState(s, provider), notice } };
}

/**
 * `auth` events update the provider's status and, while a sign-in is waiting, finish it: an
 * account its provider cannot use without the plan grant (ChatGPT) is "not eligible", else a success. `error` events of a
 * provider without a chat session end a waiting sign-in (denied, timed out, CLI missing...).
 */
export function applyAuthEvent(s: AccountsState, e: AiEvent): AccountsState {
  if (e.type === "auth") {
    const p = e.provider;
    const next = setStatus(s, e.status);
    const was = providerState(s, p);
    if (was.notice?.kind !== "signingIn") return next;
    // The newest account the sign-in produced: the first not known before, else the first signed in.
    const before = new Set((was.status?.accounts ?? []).filter((a) => a.authenticated).map((a) => a.id));
    const account = e.status.accounts.find((a) => a.authenticated && !before.has(a.id)) ?? e.status.accounts.find((a) => a.authenticated);
    if (!account) return setNotice(next, p, { kind: "failed", action: "signIn", code: "AUTH_FAILED" });
    if (accountKind(account) === "noPlan") return setNotice(next, p, { kind: "notEligible", account: account.label });
    return setNotice(next, p, { kind: "success", account: account.label });
  }
  if (e.type === "error" && !e.sessionId && providerState(s, e.provider).notice?.kind === "signingIn") {
    if (e.code === "PLAN_PERMISSION_REQUIRED") return setNotice(s, e.provider, { kind: "notEligible", account: null });
    return setNotice(s, e.provider, { kind: "failed", action: "signIn", code: e.code });
  }
  return s;
}
