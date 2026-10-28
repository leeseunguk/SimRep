import type { MessageKey } from "../i18n/en";
import type { ProviderMetadata } from "./protocol";

/**
 * The AI providers the UI knows, as data: the accounts page, the panel's provider picker, the
 * sign-in states and the account badges all come from this list. A new provider is a new entry
 * here (plus its bridge support); a provider the bridge reports that is not listed still shows,
 * with generic texts (`providerInfo`).
 */
export type AuthKind =
  /** Browser OAuth run by the bridge (the Rust host opens the authorization page). */
  | "oauth"
  /** The provider's own command-line tool owns the login (e.g. `claude auth login`). */
  | "cli-login"
  /** Device authorization: the user confirms a code on the provider's page. */
  | "device-code"
  /** Only an API key from the environment SimRep starts in; no sign-in button. */
  | "api-key";

export interface ProviderInfo {
  id: string;
  /** Brand name, not translated. */
  label: string;
  authKind: AuthKind;
  /** What signing in gives (plan usage, subscription): headline and one sentence. */
  planNote?: { title: MessageKey; body: MessageKey };
  /** Exact sign-in button label when the provider's brand rules fix it; else "Sign in with {provider}". */
  signInLabel?: MessageKey;
  /** cli-login: the tool that owns the login, named in the "finish signing in" notice. */
  cli?: string;
  /** Environment variable that can replace the sign-in with an API key. */
  apiKeyEnv?: string;
  /**
   * An authenticated account without `planEnabled`: "blocked" = signed in but not allowed to run
   * inference (ChatGPT without the plan grant); "apiKey" = runs on an API key instead of the plan.
   */
  withoutPlan: "blocked" | "apiKey";
  /** Badge of a ready account and the panel's "Using ..." line; generic texts otherwise. */
  readyLabel?: MessageKey;
  usingLabel?: MessageKey;
  /** Where the user manages plan usage (shown as "Manage usage"). */
  usageUrl?: string;
  /** Show the one-time "You're using your ChatGPT plan" welcome after the first sign-in. */
  planWelcome?: boolean;
  /** Models offered; "" = the provider default. */
  models: readonly string[];
  toolCalling?: boolean;
  unsupportedReason?: string;
  signInSupported?: boolean;
  bridgePlanNote?: string;
  regions?: readonly string[];
}

export const PROVIDER_LIST: readonly ProviderInfo[] = [
  {
    id: "chatgpt",
    label: "ChatGPT",
    authKind: "oauth",
    planNote: { title: "ai.chatgpt.headline", body: "ai.chatgpt.body" },
    // Sign in with ChatGPT UI guidelines: this exact label, approved logo, black/white format.
    signInLabel: "ai.chatgpt.continue",
    withoutPlan: "blocked",
    readyLabel: "ai.account.ready.chatgpt",
    usingLabel: "ai.panel.usingChatgpt",
    usageUrl: "https://chatgpt.com/settings/usage",
    planWelcome: true,
    models: ["", "gpt-5.4"],
  },
  {
    id: "claude",
    label: "Claude",
    authKind: "oauth",
    planNote: { title: "ai.claude.headline", body: "ai.claude.body" },
    apiKeyEnv: "ANTHROPIC_API_KEY",
    withoutPlan: "apiKey",
    readyLabel: "ai.account.ready.claude",
    usingLabel: "ai.panel.usingClaude",
    usageUrl: "https://claude.ai/settings/usage",
    models: ["", "sonnet", "opus", "haiku"],
  },
];

const BY_ID = new Map(PROVIDER_LIST.map((p) => [p.id, p]));

/** Merge only display metadata; the bridge never sends key or token material here. */
export function installProviderMetadata(providers: readonly ProviderMetadata[]): void {
  for (const p of providers) {
    if (!/^[a-z][a-z0-9_-]{0,31}$/.test(p.id) || !["oauth", "cli-login", "device-code", "api-key"].includes(p.authKind)) continue;
    const previous = BY_ID.get(p.id);
    BY_ID.set(p.id, {
      ...(previous ?? { id: p.id, label: p.id, models: [""], withoutPlan: "apiKey" as const }),
      label: p.label,
      authKind: p.authKind,
      models: ["", ...(p.models ?? []).filter((m) => typeof m === "string" && m.length > 0)],
      withoutPlan: p.withoutPlan === "blocked" ? "blocked" : "apiKey",
      ...(typeof p.apiKeyEnv === "string" && { apiKeyEnv: p.apiKeyEnv }),
      ...(typeof p.planNote === "string" && { bridgePlanNote: p.planNote }),
      ...(typeof p.unsupportedReason === "string" && { unsupportedReason: p.unsupportedReason }),
      ...(typeof p.toolCalling === "boolean" && { toolCalling: p.toolCalling }),
      ...(typeof p.signInSupported === "boolean" && { signInSupported: p.signInSupported }),
      ...(Array.isArray(p.regions) && { regions: p.regions.filter((r) => typeof r === "string") }),
      // Usage URLs are display metadata only. Rust opens the provider's fixed
      // URL on auth.openUsage; no WebView URL is passed to the operating system.
      ...(typeof p.usageUrl === "string" && { usageUrl: p.usageUrl }),
    });
  }
}

/** The entry for `id`, or a generic one for a provider the bridge knows and this list does not. */
export function providerInfo(id: string): ProviderInfo {
  return BY_ID.get(id) ?? { id, label: id, authKind: "oauth", withoutPlan: "apiKey", models: [""] };
}

/** Listed providers first, in order, then any others the bridge reported. */
export function providerIds(reported: Iterable<string> = []): string[] {
  const ids = PROVIDER_LIST.map((p) => p.id);
  for (const id of reported) if (!ids.includes(id)) ids.push(id);
  return ids;
}
