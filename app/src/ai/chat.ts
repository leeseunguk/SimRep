import { READ_ONLY_TOOLS, type AiEvent, type DoneStatus, type Provider } from "./protocol";

/**
 * The assistant transcript as a pure reducer over bridge events, so the panel and its tests share
 * one model. Model and tool text stays plain strings here; the view renders it as text, never HTML.
 */
export type ChatItem =
  | { kind: "user"; id: number; text: string }
  | { kind: "assistant"; id: number; turnId: string | null; text: string }
  | {
      kind: "tool";
      id: number;
      callId: string;
      name: string;
      args: Record<string, unknown>;
      /** Waiting for the result, finished, failed (isError), or never ran (denied / turn ended). */
      status: "running" | "done" | "error" | "skipped";
      result: ToolSummary | null;
      /** The bridge approved it without asking (bypass, or "allow all in this chat"). */
      auto?: true;
    }
  | { kind: "approval"; id: number; approvalId: string; name: string; args: Record<string, unknown>; expiresAt?: number; state: "pending" | "allowed" | "denied" | "expired" }
  | { kind: "error"; id: number; code: string; provider: Provider }
  | { kind: "ended"; id: number; status: Exclude<DoneStatus, "completed"> };

/** An item before it gets its id (Omit distributed over the union). */
type NewItem = ChatItem extends infer I ? (I extends ChatItem ? Omit<I, "id"> : never) : never;

export interface Session {
  id: string;
  provider: Provider;
  model: string;
  workspace: string;
  account?: string;
}

export interface ChatState {
  session: Session | null;
  items: ChatItem[];
  /** A turn is running (from chat.send until its one `done`). */
  running: boolean;
  nextId: number;
  activeSkill?: string;
  /** "Allow all in this chat" was chosen: the bridge session runs in `session` approval mode. */
  allowAll?: boolean;
}

export const EMPTY_CHAT: ChatState = { session: null, items: [], running: false, nextId: 1 };

export function addUser(s: ChatState, text: string): ChatState {
  return { ...s, items: [...s.items, { kind: "user", id: s.nextId, text }], running: true, nextId: s.nextId + 1 };
}

export function addError(s: ChatState, provider: Provider, code: string): ChatState {
  return { ...s, items: [...s.items, { kind: "error", id: s.nextId, code, provider }], nextId: s.nextId + 1 };
}

export function resolveApproval(s: ChatState, approvalId: string, allow: boolean): ChatState {
  return {
    ...s,
    items: s.items.map((i) => (i.kind === "approval" && i.approvalId === approvalId && i.state === "pending" ? { ...i, state: allow ? "allowed" : "denied" } : i)),
  };
}

/** Applies one `ai-event` to the transcript. Events of other sessions (an older chat) are ignored. */
export function applyEvent(s: ChatState, e: AiEvent): ChatState {
  if (e.type === "auth" || e.type === "device_code") return s;
  if (!s.session || e.sessionId !== s.session.id) return s;
  if ("activeSkill" in e && e.activeSkill) s = { ...s, activeSkill: e.activeSkill };
  const items = s.items;
  const push = (item: NewItem): ChatState => ({ ...s, items: [...items, { ...item, id: s.nextId } as ChatItem], nextId: s.nextId + 1 });
  switch (e.type) {
    case "delta": {
      const last = items.at(-1);
      const turnId = e.turnId ?? null;
      // Text after a tool card starts a new bubble, so the transcript reads in order.
      if (last?.kind === "assistant" && last.turnId === turnId) return { ...s, items: [...items.slice(0, -1), { ...last, text: last.text + e.text }] };
      return push({ kind: "assistant", turnId, text: e.text });
    }
    case "tool_call":
      return push({ kind: "tool", callId: e.callId, name: e.name, args: e.arguments, status: "running", result: null });
    case "tool_result": {
      const summary = summarizeResult(e.result);
      return {
        ...s,
        items: items.map((i) => (i.kind === "tool" && i.callId === e.callId ? { ...i, status: summary.isError ? "error" : "done", result: summary } : i)),
      };
    }
    case "approval": {
      // An auto-approval is an audit record, not a question: it marks the tool card it belongs to
      // (the newest running call of that tool) instead of adding an approval card.
      if (e.auto) {
        const at = items.findLastIndex((i) => i.kind === "tool" && i.name === e.name && i.status === "running" && !i.auto);
        if (at >= 0) return { ...s, items: items.map((i, n) => (n === at ? { ...i, auto: true } : i)) as ChatItem[] };
        return push({ kind: "tool", callId: e.approvalId, name: e.name, args: e.arguments, status: "running", result: null, auto: true });
      }
      return push({ kind: "approval", approvalId: e.approvalId, name: e.name, args: e.arguments, expiresAt: e.expiresAt, state: "pending" });
    }
    case "error":
      return push({ kind: "error", code: e.code, provider: e.provider });
    case "done": {
      // The bridge denies every approval still open when a turn ends; tools still waiting never ran.
      const closed = items.map((i): ChatItem => {
        if (i.kind === "approval" && i.state === "pending") return { ...i, state: e.status === "completed" ? "expired" : "denied" };
        if (i.kind === "tool" && i.status === "running") return { ...i, status: "skipped" };
        return i;
      });
      const next = { ...s, items: closed, running: false };
      if (e.status === "completed") return next;
      return { ...next, items: [...closed, { kind: "ended", id: s.nextId, status: e.status }], nextId: s.nextId + 1 };
    }
  }
}

/** Whether a SimRep tool can change files in the workspace (the bridge asks for approval first). */
export const writesFiles = (name: string) => !(READ_ONLY_TOOLS as readonly string[]).includes(name);

export interface SummaryField {
  key: string;
  value: string;
}

/** What a tool card shows of a SimRep MCP result: the CLI JSON summary's key numbers, or its text. */
export interface ToolSummary {
  isError: boolean;
  fields: SummaryField[];
  /** Plain text when the result is not a JSON summary (an error message, a refusal). */
  text: string | null;
}

/** Summary keys shown on a card, in order (AGENTS.md "Machine-readable output"). */
const KEYS = ["executed", "compute_ms", "ion_count", "arrivals", "arrival_tof_us", "splat_counts", "converged", "cycles", "final_residual", "fallback_reason", "dims", "output_paths"] as const;
const MAX_TEXT = 600;

const fmt = (v: number) => (Number.isInteger(v) ? String(v) : Math.abs(v) < 1e-3 || Math.abs(v) >= 1e6 ? v.toExponential(3) : String(Number(v.toPrecision(6))));

function value(key: string, v: unknown): string | null {
  if (v === null || v === undefined) return null;
  if (typeof v === "number") return fmt(v);
  if (typeof v === "string" || typeof v === "boolean") return String(v);
  if (Array.isArray(v)) return v.map((x) => (typeof x === "number" ? fmt(x) : String(x))).join(key === "dims" ? " × " : ", ");
  if (typeof v === "object") {
    const o = v as Record<string, unknown>;
    if (key === "executed") return [o.device, o.precision, o.backend].filter((x) => typeof x === "string").join(" · ");
    if (key === "arrival_tof_us") return typeof o.mean === "number" ? `${fmt(o.mean)} ± ${typeof o.std === "number" ? fmt(o.std) : "?"}` : null;
    const parts = Object.entries(o).filter(([, x]) => typeof x === "number" && x !== 0);
    return parts.length ? parts.map(([k, x]) => `${k} ${fmt(x as number)}`).join(", ") : null;
  }
  return null;
}

/** An MCP `tools/call` result (`{content: [{type: "text", text}], isError}`) as a card summary. */
export function summarizeResult(result: unknown): ToolSummary {
  const r = (typeof result === "object" && result !== null ? result : {}) as { content?: unknown; isError?: unknown };
  const isError = r.isError === true;
  const text = Array.isArray(r.content)
    ? r.content
        .filter((c): c is { type: "text"; text: string } => typeof c === "object" && c !== null && (c as { type?: unknown }).type === "text" && typeof (c as { text?: unknown }).text === "string")
        .map((c) => c.text)
        .join("\n")
    : typeof result === "string"
      ? result
      : "";
  let json: unknown = null;
  try {
    json = JSON.parse(text);
  } catch {
    json = null;
  }
  if (json && typeof json === "object" && !Array.isArray(json)) {
    const o = json as Record<string, unknown>;
    const err = o.error as { message?: unknown } | undefined;
    if (err && typeof err.message === "string") return { isError: true, fields: [], text: clip(err.message) };
    const fields = KEYS.flatMap((k) => {
      const v = value(k, o[k]);
      return v ? [{ key: k, value: v }] : [];
    });
    if (fields.length) return { isError, fields, text: null };
  }
  return { isError, fields: [], text: text ? clip(text) : null };
}

const clip = (t: string) => (t.length > MAX_TEXT ? `${t.slice(0, MAX_TEXT)}…` : t);

/** Up to `max` arguments with short scalar values, for the card header ("pa: einzel.pa0"). */
export function keyArgs(args: Record<string, unknown>, max = 4): SummaryField[] {
  return Object.entries(args)
    .flatMap(([key, v]) => {
      if (typeof v === "number" || typeof v === "boolean") return [{ key, value: String(v) }];
      if (typeof v === "string") return [{ key, value: v.length > 48 ? `…${v.slice(-47)}` : v }];
      if (Array.isArray(v) && v.every((x) => typeof x === "number" || typeof x === "string")) return [{ key, value: v.join(", ").slice(0, 48) }];
      if (v && typeof v === "object") {
        const s = Object.entries(v as Record<string, unknown>)
          .filter(([, x]) => typeof x === "number" || typeof x === "string")
          .map(([k, x]) => `${k}=${String(x)}`)
          .join(", ");
        return s ? [{ key, value: s.slice(0, 48) }] : [];
      }
      return [];
    })
    .slice(0, max);
}
