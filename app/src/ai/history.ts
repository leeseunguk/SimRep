import type { ChatItem, ChatState, ToolSummary } from "./chat";
import type { DoneStatus, Provider } from "./protocol";
import { translate, type MessageKey } from "../i18n";
import type { TranscriptItem } from "../report/model";
import type { Lang } from "../state/store";
import * as projectDir from "../state/projectDir";

/**
 * Saved AI chats: one `ai/<chat-id>.json` per conversation in the open project's data folder
 * (`<X>.simrep.d/`, state/projectDir.ts). A saved chat holds what the panel showed (messages, tool
 * calls with their compact card summary, approvals), who answered (provider, model) and when.
 * Known credential/identity keys and text patterns are redacted before clipping. This is
 * best-effort filtering, not detection of arbitrary secrets. Tool arguments are compacted,
 * and a tool result is only its card summary, never the original file contents.
 */
export const HISTORY_DIR = "ai";
export const CHAT_FORMAT = "simrep-ai-chat";
export const CHAT_VERSION = 1;
/** Upper bound of the summary a continued chat starts from (characters). */
export const SUMMARY_MAX = 4000;

const TITLE_MAX = 80;
const ARG_STRING_MAX = 160;
const ARG_ARRAY_MAX = 16;
const ARG_DEPTH = 2;
const TEXT_MAX = 20_000;

export type SavedItem =
  | { kind: "user"; text: string; at?: number }
  | { kind: "assistant"; text: string }
  | { kind: "tool"; callId: string; name: string; args: Record<string, unknown>; status: "done" | "error" | "skipped"; result: ToolSummary | null; auto?: true }
  | { kind: "approval"; name: string; args: Record<string, unknown>; state: "allowed" | "denied" | "expired" }
  | { kind: "error"; code: string }
  | { kind: "ended"; status: Exclude<DoneStatus, "completed"> };

/** One approval decision, including the bridge's automatic ones (`auto`). */
export interface SavedApproval {
  name: string;
  decision: "allowed" | "denied" | "expired" | "auto";
}

export interface SavedChat {
  format: typeof CHAT_FORMAT;
  version: typeof CHAT_VERSION;
  id: string;
  /** The first user message, shortened. */
  title: string;
  provider: Provider;
  providerLabel: string;
  model: string;
  /** Harness recipe the bridge reported (`activeSkill`), if any. */
  recipe: string | null;
  createdAt: number;
  updatedAt: number;
  /** The saved chat this one continued (a new provider session started from its summary). */
  continuedFrom?: { id: string; title: string };
  items: SavedItem[];
  approvals: SavedApproval[];
}

// ------------------------------------------------------------------------------- redaction

const REDACTED = "[redacted]";
/** Object keys whose values are credentials or identities; dropped wherever they appear. */
const SECRET_KEY = /token|secret|passw(or)?d|api[-_]?key|authorization|cookie|credential|private[-_]?key|^e-?mail$|^(account|user|client)([-_]?id)?$|^session$/i;
const PATTERNS: [RegExp, string][] = [
  // JWTs (OAuth id/access tokens).
  [/\beyJ[A-Za-z0-9_-]{4,}\.[A-Za-z0-9_-]{4,}\.[A-Za-z0-9_-]{4,}/g, REDACTED],
  [/\b(Basic|Bearer)\s+[A-Za-z0-9._~+/=-]+/gi, `$1 ${REDACTED}`],
  // Provider key formats: OpenAI / Anthropic, GitHub, Slack, AWS, Google.
  [/\b(?:sk|pk|rk)-(?:ant-|proj-|live-|test-)?[A-Za-z0-9_-]{16,}/g, REDACTED],
  [/\bgh[pousr]_[A-Za-z0-9]{20,}/g, REDACTED],
  [/\bxox[abpr]-[A-Za-z0-9-]{10,}/g, REDACTED],
  [/\bAKIA[0-9A-Z]{16}\b/g, REDACTED],
  [/\bAIza[0-9A-Za-z_-]{30,}/g, REDACTED],
  [/[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}/g, "[redacted email]"],
];

export function redactText(s: string): string {
  const value = (_match: string, prefix: string, secret: string) => {
    const quote = secret[0] === '"' || secret[0] === "'" ? secret[0] : "";
    return `${prefix}${quote}${REDACTED}${quote}`;
  };
  // Quoted values can contain spaces; unquoted cookie headers run to the end
  // of their line, including additional cookies/attributes after a semicolon.
  s = s.replace(/\b((?:set[-_])?cookie["']?\s*[:=]\s*)("(?:\\.|[^"\\])*"|'(?:\\.|[^'\\])*'|[^\r\n"'}]+)/gi, value);
  s = PATTERNS.reduce((acc, [re, by]) => acc.replace(re, by), s);
  return s.replace(/\b((?:(?:account|user|client)[-_]?id|client[-_]?secret|access[-_]?token|refresh[-_]?token|id[-_]?token|x[-_]?api[-_]?key|api[-_]?key|password|authorization|session|token)["']?\s*[:=]\s*)("(?:\\.|[^"\\])*"|'(?:\\.|[^'\\])*'|[^\s"'&,;}]+)/gi, value);
}

/** A copy of `v` with credential-named keys dropped and every string redacted. */
export function redact<T>(v: T, identities: readonly string[] = []): T {
  if (typeof v === "string") return redactText(identities.reduce((s, id) => id ? s.split(id).join(REDACTED) : s, v)) as T;
  if (Array.isArray(v)) return v.map((x) => redact(x, identities)) as T;
  if (v && typeof v === "object") return Object.fromEntries(Object.entries(v).filter(([k]) => !SECRET_KEY.test(k)).map(([k, x]) => [k, redact(x, identities)])) as T;
  return v;
}

// --------------------------------------------------------------------------- serialization

const clip = (s: string, max: number) => {
  s = redactText(s);
  return s.length > max ? `${s.slice(0, max - 1)}…` : s;
};

/** Tool arguments as short scalars: long strings (file contents) are clipped, deep values dropped. */
export function compactArgs(v: Record<string, unknown>, depth = 0): Record<string, unknown> {
  const one = (x: unknown, d: number): unknown => {
    if (typeof x === "string") return clip(x, ARG_STRING_MAX);
    if (typeof x === "number" || typeof x === "boolean" || x === null) return x;
    if (Array.isArray(x)) return x.length <= ARG_ARRAY_MAX && x.every((y) => typeof y !== "object" || y === null) ? x.map((y) => one(y, d)) : `[${x.length} items]`;
    if (typeof x === "object" && d < ARG_DEPTH) return compactArgs(x as Record<string, unknown>, d + 1);
    return undefined;
  };
  return Object.fromEntries(Object.entries(v).flatMap(([k, x]) => {
    const c = one(x, depth);
    return c === undefined ? [] : [[k, c]];
  }));
}

const titleOf = (items: readonly { kind: string; text?: string }[]) => {
  const first = items.find((i) => i.kind === "user")?.text?.trim().replace(/\s+/g, " ");
  return first ? clip(first, TITLE_MAX) : "";
};

export interface SaveMeta {
  id: string;
  provider: Provider;
  providerLabel: string;
  model: string;
  createdAt: number;
  updatedAt: number;
  continuedFrom?: { id: string; title: string };
}

/** The chat as its saved, redacted JSON object. Open approvals and running tools are closed. */
export function serializeChat(chat: ChatState, meta: SaveMeta): SavedChat {
  const identities = chat.session?.account ? [chat.session.account] : [];
  const items = redact(chat.items, identities).map((i): SavedItem => {
    switch (i.kind) {
      case "user":
        return { kind: "user", text: clip(i.text, TEXT_MAX), ...(i.at !== undefined && { at: i.at }) };
      case "assistant":
        return { kind: "assistant", text: clip(i.text, TEXT_MAX) };
      case "tool":
        return { kind: "tool", callId: i.callId, name: i.name, args: compactArgs(i.args), status: i.status === "running" ? "skipped" : i.status, result: i.result, ...(i.auto && { auto: true as const }) };
      case "approval":
        return { kind: "approval", name: i.name, args: compactArgs(i.args), state: i.state === "pending" ? "expired" : i.state };
      case "error":
        return { kind: "error", code: i.code };
      case "ended":
        return { kind: "ended", status: i.status };
    }
  });
  const approvals: SavedApproval[] = chat.items.flatMap((i): SavedApproval[] =>
    i.kind === "approval" ? [{ name: i.name, decision: i.state === "pending" ? "expired" : i.state }] : i.kind === "tool" && i.auto ? [{ name: i.name, decision: "auto" }] : [],
  );
  return redact({
    format: CHAT_FORMAT,
    version: CHAT_VERSION,
    id: meta.id,
    title: titleOf(items),
    provider: meta.provider,
    providerLabel: meta.providerLabel,
    model: meta.model,
    recipe: chat.activeSkill ?? null,
    createdAt: meta.createdAt,
    updatedAt: meta.updatedAt,
    ...(meta.continuedFrom && { continuedFrom: meta.continuedFrom }),
    items,
    approvals,
  }, identities);
}

const isObj = (v: unknown): v is Record<string, unknown> => typeof v === "object" && v !== null && !Array.isArray(v);
const str = (v: unknown): v is string => typeof v === "string";
const num = (v: unknown): v is number => typeof v === "number" && Number.isFinite(v);
export const validChatId = (id: string) => /^[A-Za-z0-9-]{8,64}$/.test(id);

function parseSummary(v: unknown): ToolSummary | null {
  if (!isObj(v) || !Array.isArray(v.fields)) return null;
  return {
    isError: v.isError === true,
    fields: v.fields.filter((f): f is { key: string; value: string } => isObj(f) && str(f.key) && str(f.value)),
    text: str(v.text) ? v.text : null,
  };
}

function parseItem(v: unknown): SavedItem | null {
  if (!isObj(v)) return null;
  const args = isObj(v.args) ? v.args : {};
  switch (v.kind) {
    case "user":
      return str(v.text) ? { kind: "user", text: v.text, ...(num(v.at) && { at: v.at }) } : null;
    case "assistant":
      return str(v.text) ? { kind: "assistant", text: v.text } : null;
    case "tool":
      return str(v.name) && (v.status === "done" || v.status === "error" || v.status === "skipped")
        ? { kind: "tool", callId: str(v.callId) ? v.callId : "", name: v.name, args, status: v.status, result: parseSummary(v.result), ...(v.auto === true && { auto: true as const }) }
        : null;
    case "approval":
      return str(v.name) && (v.state === "allowed" || v.state === "denied" || v.state === "expired") ? { kind: "approval", name: v.name, args, state: v.state } : null;
    case "error":
      return str(v.code) ? { kind: "error", code: v.code } : null;
    case "ended":
      return v.status === "interrupted" || v.status === "failed" ? { kind: "ended", status: v.status } : null;
    default:
      return null;
  }
}

/** A saved chat file, validated; null when it is not one. The content is redacted again on read. */
export function parseChat(text: string): SavedChat | null {
  let v: unknown;
  try {
    v = JSON.parse(text);
  } catch {
    return null;
  }
  if (!isObj(v) || v.format !== CHAT_FORMAT || v.version !== CHAT_VERSION || !str(v.id) || !validChatId(v.id) || !str(v.provider) || !Array.isArray(v.items)) return null;
  const items = v.items.map(parseItem).filter((i): i is SavedItem => i !== null);
  const from = isObj(v.continuedFrom) && str(v.continuedFrom.id) && str(v.continuedFrom.title) ? { id: v.continuedFrom.id, title: v.continuedFrom.title } : undefined;
  const approvals = Array.isArray(v.approvals)
    ? v.approvals.filter((a): a is SavedApproval => isObj(a) && str(a.name) && ["allowed", "denied", "expired", "auto"].includes(a.decision as string))
    : [];
  return redact({
    format: CHAT_FORMAT,
    version: CHAT_VERSION,
    id: v.id,
    title: str(v.title) ? v.title : titleOf(items),
    provider: v.provider,
    providerLabel: str(v.providerLabel) ? v.providerLabel : v.provider,
    model: str(v.model) ? v.model : "",
    recipe: str(v.recipe) ? v.recipe : null,
    createdAt: num(v.createdAt) ? v.createdAt : 0,
    updatedAt: num(v.updatedAt) ? v.updatedAt : num(v.createdAt) ? v.createdAt : 0,
    ...(from && { continuedFrom: from }),
    items,
    approvals: approvals.map((a) => ({ name: a.name, decision: a.decision })),
  });
}

/** A saved chat as a transcript the panel renders read-only (no session). */
export function toChatState(saved: SavedChat): ChatState {
  const items = redact(saved).items.map((i, n): ChatItem => {
    const id = n + 1;
    switch (i.kind) {
      case "user":
        return { kind: "user", id, text: i.text, ...(i.at !== undefined && { at: i.at }) };
      case "assistant":
        return { kind: "assistant", id, turnId: null, text: i.text };
      case "tool":
        return { kind: "tool", id, callId: i.callId, name: i.name, args: i.args, status: i.status, result: i.result, ...(i.auto && { auto: true as const }) };
      case "approval":
        return { kind: "approval", id, approvalId: `saved-${id}`, name: i.name, args: i.args, state: i.state };
      case "error":
        return { kind: "error", id, code: i.code, provider: saved.provider };
      case "ended":
        return { kind: "ended", id, status: i.status };
    }
  });
  return { session: null, items, running: false, nextId: items.length + 1, ...(saved.recipe && { activeSkill: saved.recipe }) };
}

/**
 * The first context of a chat that continues `saved`: a short, bounded summary (never more than
 * `max` characters). It keeps the first request and as many of the latest lines as fit.
 */
export function continueSummary(saved: SavedChat, max = SUMMARY_MAX): string {
  const head = clip(
    `Summary of an earlier SimRep chat in this project (${saved.providerLabel}${saved.model ? `, ${saved.model}` : ""}, ${new Date(saved.updatedAt).toISOString().slice(0, 10)}). ` +
      "That provider session cannot be resumed; use this only as background, and re-check files and results before relying on them.",
    max,
  );
  const line = (i: SavedItem): string | null => {
    const one = (s: string, n: number) => clip(s.replace(/\s+/g, " ").trim(), n);
    switch (i.kind) {
      case "user":
        return `User: ${one(i.text, 300)}`;
      case "assistant":
        return i.text.trim() ? `Assistant: ${one(i.text, 400)}` : null;
      case "tool": {
        const result = i.result ? (i.result.fields.length ? i.result.fields.map((f) => `${f.key} ${f.value}`).join(", ") : (i.result.text ?? "")) : "";
        return one(`Tool ${i.name} [${i.status}${i.auto ? ", auto-approved" : ""}]${result ? `: ${result}` : ""}`, 240);
      }
      case "approval":
        return `Approval ${i.name}: ${i.state}`;
      case "error":
        return `Error: ${i.code}`;
      case "ended":
        return `Reply ${i.status}`;
    }
  };
  const lines = redact(saved).items.map(line).filter((l): l is string => l !== null);
  const firstUser = lines.findIndex((l) => l.startsWith("User: "));
  const picked: string[] = [];
  let used = head.length;
  const fits = (l: string) => used + 1 + l.length <= max;
  if (firstUser >= 0 && fits(lines[firstUser]!)) {
    picked.push(lines[firstUser]!);
    used += 1 + lines[firstUser]!.length;
  }
  const tail: string[] = [];
  const gap = "…";
  for (let n = lines.length - 1; n > firstUser; n--) {
    const l = lines[n]!;
    // Keep room for the gap marker between the first request and the latest lines.
    if (used + 1 + l.length + 2 > max) break;
    tail.unshift(l);
    used += 1 + l.length;
  }
  const skipped = lines.length - (firstUser >= 0 ? firstUser + 1 : 0) - tail.length;
  const body = [...picked, ...(skipped > 0 && picked.length ? [gap] : []), ...tail];
  return [head, ...body].join("\n").slice(0, max);
}

/**
 * A saved chat as the report's transcript appendix (report/transcript.ts):
 * messages, tool calls with their card summary, approvals, errors and ended replies, in `lang`.
 */
export function transcriptOf(saved: SavedChat, lang: Lang): TranscriptItem[] {
  const t = (key: MessageKey, params?: Record<string, string>) => translate(lang, key, params);
  return redact(saved).items.flatMap((i): TranscriptItem[] => {
    switch (i.kind) {
      case "user":
        return [{ role: "user", text: i.text }];
      case "assistant":
        return i.text.trim() ? [{ role: "assistant", text: i.text }] : [];
      case "tool": {
        const status = t(i.status === "done" ? "ai.tool.done" : i.status === "error" ? "ai.tool.error" : "ai.tool.skipped");
        const result = i.result ? (i.result.fields.length ? i.result.fields.map((f) => `${f.key}: ${f.value}`).join("\n") : (i.result.text ?? "")) : "";
        return [{ role: "tool", tool: i.name, text: [`${status}${i.auto ? ` (${t("ai.approval.auto")})` : ""}`, result].filter(Boolean).join("\n") }];
      }
      case "approval":
        return [{ role: "system", text: `${t("ai.approval.title", { tool: i.name })} ${t(i.state === "allowed" ? "ai.approval.allowed" : i.state === "denied" ? "ai.approval.denied" : "ai.approval.expired")}` }];
      case "error": {
        const key = `ai.reason.${i.code}` as MessageKey;
        return [{ role: "system", text: translate(lang, key) ?? i.code }];
      }
      case "ended":
        return [{ role: "system", text: t(i.status === "interrupted" ? "ai.ended.interrupted" : "ai.ended.failed") }];
    }
  });
}

// ------------------------------------------------------------------------------------ files

/** The project data-folder functions this module uses (state/projectDir.ts; demo/ipc.ts fakes its commands). */
function files() {
  return {
    projectDataDir: () => projectDir.projectDataDir(),
    writeProjectFile: (rel: string, data: string) => projectDir.writeProjectFile(rel, data),
    readProjectText: (rel: string) => projectDir.readProjectText(rel),
    listProjectFiles: (prefix: string) => projectDir.listProjectFiles(prefix),
    deleteProjectFile: (rel: string) => projectDir.deleteProjectFile(rel),
  };
}

const relOf = (id: string) => `${HISTORY_DIR}/${id}.json`;

/** The open project's data folder; null when it has none (an unsaved project, the browser). */
export const historyDataDir = () => files().projectDataDir();

export async function saveChat(saved: SavedChat): Promise<void> {
  if (!validChatId(saved.id)) throw new Error(`invalid chat id ${saved.id}`);
  if (!(await files().writeProjectFile(relOf(saved.id), `${JSON.stringify(redact(saved), null, 2)}\n`))) throw new Error("the project has no data folder");
}

/** The project's saved chats, newest first; unreadable files are skipped. */
export async function listChats(): Promise<SavedChat[]> {
  const f = files();
  if (f.projectDataDir() === null) return [];
  const entries = await f.listProjectFiles(HISTORY_DIR);
  // Only direct children `ai/<id>.json`.
  const ids = [...new Set(entries.map((e) => e.path).filter((p) => p.startsWith(`${HISTORY_DIR}/`) && !p.slice(HISTORY_DIR.length + 1).includes("/")).map((p) => p.slice(HISTORY_DIR.length + 1)).filter((n) => n.endsWith(".json")).map((n) => n.slice(0, -5)).filter(validChatId))];
  const chats: SavedChat[] = [];
  // Bound concurrent native reads for externally supplied project folders.
  for (let start = 0; start < ids.length; start += 8) {
    const batch = await Promise.all(ids.slice(start, start + 8).map(async (id) => {
      const text = await f.readProjectText(relOf(id)).catch(() => null);
      const chat = text === null ? null : parseChat(text);
      return chat && chat.id === id ? chat : null;
    }));
    chats.push(...batch.filter((c): c is SavedChat => c !== null));
  }
  return chats.sort((a, b) => b.updatedAt - a.updatedAt);
}

export async function deleteChat(id: string): Promise<void> {
  if (!validChatId(id)) return;
  await files().deleteProjectFile(relOf(id));
}
