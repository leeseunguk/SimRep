/**
 * SimRep AI bridge protocol v1, as the desktop UI sees it (ai/PROTOCOL.md).
 *
 * The UI calls the Tauri command `ai_request` with `{ request: { jsonrpc: "2.0", id, method, params } }`
 * and gets the whole JSON-RPC response back. Notifications arrive as the Tauri event `ai-event`,
 * whose payload is the notification's `params`. No method accepts or returns tokens or API keys,
 * and authorization URLs never reach the WebView (the Rust host opens the browser itself).
 */

/**
 * Provider id ("chatgpt", "claude" today). Kept open: the bridge may add providers, and the UI
 * describes each one from data (providers.ts), not from this type.
 */
export type Provider = string;

/** Tauri command and event names (app/src-tauri/src/ai.rs). */
export const AI_COMMAND = "ai_request";
export const AI_EVENT = "ai-event";

export interface Account {
  /** Opaque, stable. */
  id: string;
  provider: Provider;
  label: string;
  email?: string;
  authenticated: boolean;
  /**
   * ChatGPT: the direct-use (plan usage) grant is present. Without it a valid identity still cannot
   * run inference. Claude: signed in with the Claude subscription login (false for an inherited
   * ANTHROPIC_API_KEY, which takes precedence over the login).
   */
  planEnabled: boolean;
}

export interface ProviderStatus {
  provider: Provider;
  authenticated: boolean;
  accounts: Account[];
  message?: string;
}

/** Request params and results per method. */
export interface Methods {
  "auth.status": { params: { provider?: Provider }; result: { providers: ProviderStatus[] } };
  "auth.accounts": { params: { provider?: Provider }; result: { accounts: Account[] } };
  "auth.login": { params: { provider: Provider; account?: string }; result: { started: true } };
  "auth.logout": { params: { provider: Provider; account?: string }; result: { signedOut: true; remoteRevocationConfirmed: boolean } };
  "chat.start": {
    params: { provider: Provider; model?: string; workspace: string; account?: string };
    result: { sessionId: string; provider: Provider; model: string; workspace: string };
  };
  "chat.send": { params: { sessionId: string; text: string }; result: { turnId: string } };
  "chat.cancel": { params: { sessionId: string }; result: { cancelled: true } };
  "approval.resolve": { params: { approvalId: string; allow: boolean }; result: { resolved: true } };
}
export type Method = keyof Methods;

export interface RpcRequest<M extends Method = Method> {
  jsonrpc: "2.0";
  id: number | string;
  method: M;
  params: Methods[M]["params"];
}

export interface RpcError {
  code: number;
  message: string;
  data?: { code?: string; details?: unknown };
}

export type RpcResponse<M extends Method = Method> =
  | { jsonrpc: "2.0"; id: number | string; result: Methods[M]["result"] }
  | { jsonrpc: "2.0"; id: number | string; error: RpcError };

export type DoneStatus = "completed" | "interrupted" | "failed";

interface Base {
  provider: Provider;
}
interface ChatBase extends Base {
  sessionId: string;
  turnId?: string;
}

export type AiEvent =
  | (Base & { type: "auth"; status: ProviderStatus })
  | (ChatBase & { type: "delta"; text: string })
  | (ChatBase & { type: "tool_call"; callId: string; name: string; arguments: Record<string, unknown> })
  | (ChatBase & { type: "tool_result"; callId: string; name?: string; result: unknown })
  | (ChatBase & { type: "approval"; approvalId: string; name: string; arguments: Record<string, unknown> })
  | (ChatBase & { type: "done"; status: DoneStatus })
  | (Base & { type: "error"; code: string; message: string; details?: unknown; sessionId?: string; turnId?: string });

/**
 * Stable bridge error codes (`error.data.code` of a response, `code` of an `error` event) that the UI
 * explains in its own words. OAuth refresh failures arrive as the token endpoint's own code.
 * BRIDGE_UNAVAILABLE is the UI's code for a failed `ai_request` call itself (no sidecar, no Node).
 */
export const AI_ERROR_CODES = [
  "AUTH_REQUIRED",
  "PLAN_PERMISSION_REQUIRED",
  "AUTH_DENIED",
  "AUTH_TIMEOUT",
  "AUTH_CALLBACK",
  "AUTH_IDENTITY",
  "AUTH_RESPONSE",
  "AUTH_HTTP",
  "AUTH_FAILED",
  "CANCELLED",
  "BUSY",
  "CLAUDE_CLI_MISSING",
  "ENV_API_KEY",
  "WORKSPACE_UNSAFE",
  "INVALID_PARAMS",
  "INVALID_REQUEST",
  "CONFIG_UNSAFE",
  "TOOL_DENIED",
  "PROVIDER_ERROR",
  "PROVIDER_CLOSED",
  "PROVIDER_TIMEOUT",
  "PROVIDER_RPC",
  "BRIDGE_UNAVAILABLE",
] as const;
export type AiErrorCode = (typeof AI_ERROR_CODES)[number];

/** OAuth token-endpoint codes the bridge forwards when a ChatGPT refresh fails for good (oauth.mjs). */
export const REFRESH_FAILED_CODES = [
  "invalid_grant",
  "invalid_refresh_token",
  "token_expired",
  "refresh_token_expired",
  "refresh_token_invalidated",
  "refresh_token_reused",
] as const;

/** SimRep MCP tools that only read (no approval); the rest may write files in the workspace (ai/tools.mjs). */
export const READ_ONLY_TOOLS = ["gpu_info", "pa_info", "read_csv_summary", "cad_measure"] as const;

const isObject = (v: unknown): v is Record<string, unknown> => typeof v === "object" && v !== null && !Array.isArray(v);
const isProvider = (v: unknown): v is Provider => typeof v === "string" && /^[a-z][a-z0-9_-]{0,31}$/.test(v);
const str = (v: unknown): string | undefined => (typeof v === "string" ? v : undefined);

/** An account with only the protocol's display fields (anything else the payload carries is dropped). */
export function parseAccount(v: unknown): Account | null {
  if (!isObject(v) || !isProvider(v.provider) || typeof v.id !== "string") return null;
  return {
    id: v.id,
    provider: v.provider,
    label: str(v.label) ?? v.id,
    ...(typeof v.email === "string" && { email: v.email }),
    authenticated: v.authenticated === true,
    planEnabled: v.planEnabled === true,
  };
}

export function parseStatus(v: unknown): ProviderStatus | null {
  if (!isObject(v) || !isProvider(v.provider)) return null;
  const accounts = Array.isArray(v.accounts) ? v.accounts.map(parseAccount).filter((a): a is Account => a !== null) : [];
  return { provider: v.provider, authenticated: v.authenticated === true, accounts, ...(typeof v.message === "string" && { message: v.message }) };
}

/**
 * Validates an `ai-event` payload and keeps only its protocol fields, so nothing the UI did not ask
 * for (and nothing credential-shaped) lands in a store. Unknown types and providers give null.
 */
export function parseEvent(raw: unknown): AiEvent | null {
  if (!isObject(raw) || !isProvider(raw.provider) || typeof raw.type !== "string") return null;
  const provider = raw.provider;
  const sessionId = str(raw.sessionId);
  const turnId = str(raw.turnId);
  const chat = sessionId ? { provider, sessionId, ...(turnId && { turnId }) } : null;
  const args = (v: unknown) => (isObject(v) ? v : {});
  switch (raw.type) {
    case "auth": {
      const status = parseStatus(raw.status);
      return status ? { type: "auth", provider, status } : null;
    }
    case "delta":
      return chat && typeof raw.text === "string" ? { type: "delta", ...chat, text: raw.text } : null;
    case "tool_call":
      return chat && typeof raw.callId === "string" && typeof raw.name === "string"
        ? { type: "tool_call", ...chat, callId: raw.callId, name: raw.name, arguments: args(raw.arguments) }
        : null;
    case "tool_result":
      return chat && typeof raw.callId === "string"
        ? { type: "tool_result", ...chat, callId: raw.callId, ...(typeof raw.name === "string" && { name: raw.name }), result: raw.result }
        : null;
    case "approval":
      return chat && typeof raw.approvalId === "string" && typeof raw.name === "string"
        ? { type: "approval", ...chat, approvalId: raw.approvalId, name: raw.name, arguments: args(raw.arguments) }
        : null;
    case "done":
      return chat && (raw.status === "completed" || raw.status === "interrupted" || raw.status === "failed")
        ? { type: "done", ...chat, status: raw.status }
        : null;
    case "error":
      return {
        type: "error",
        provider,
        code: str(raw.code) ?? "PROVIDER_ERROR",
        message: str(raw.message) ?? "",
        ...(raw.details !== undefined && { details: raw.details }),
        ...(sessionId && { sessionId }),
        ...(turnId && { turnId }),
      };
    default:
      return null;
  }
}

/** A failed request: the bridge's stable code (or BRIDGE_UNAVAILABLE) and its safe English message. */
export class AiError extends Error {
  constructor(
    readonly code: string,
    message: string,
  ) {
    super(message);
  }
}

/** The result of a response, or an AiError with the bridge code from `error.data.code`. */
export function unwrap<M extends Method>(response: RpcResponse<M>): Methods[M]["result"] {
  if ("error" in response) {
    const e = response.error;
    throw new AiError(e.data?.code ?? (e.code === -32602 ? "INVALID_PARAMS" : "PROVIDER_ERROR"), e.message);
  }
  return response.result;
}
