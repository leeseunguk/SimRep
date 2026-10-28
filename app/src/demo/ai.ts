import { emit } from "@tauri-apps/api/event";
import { AI_EVENT, type Account, type AiEvent, type Method, type Methods, type Provider, type ProviderMetadata, type ProviderStatus, type RpcRequest, type RpcResponse } from "../ai/protocol";

/**
 * Demo mode's AI bridge (ai/PROTOCOL.md): `ai_request` answers and `ai-event` notifications for
 * both providers, with no accounts, network or sidecar. Sign-ins succeed after the "browser"; a
 * chat turn streams text, runs a read-only tool, asks approval for `fly` and reports its summary.
 *
 * Address flags: `?ai=noplan` (ChatGPT sign-in without plan usage), `?ai=denied` (access_denied),
 * `?ai=apikey` (Claude through ANTHROPIC_API_KEY), `?ai=device` (adds the bridge's other sign-in
 * kinds: GitHub Copilot and Kimi Code by device code, sent as a `device_code` event before the
 * `auth`; Kimi with its region choice; Cursor, sign-in only). With `?replay=manual` events wait for `step()`
 * (e2e and captures), otherwise they follow on a timer.
 */
export interface DemoAi {
  /** Sends the next queued `ai-event`; its type, or null when nothing is queued. */
  step(): Promise<string | null>;
  /** Types of the events sent so far, in order. */
  sent: string[];
  queued(): number;
  started: { sessionId: string; provider: Provider; account?: string }[];
}

const EVENT_STEP_MS = 140;

const FLY_RESULT = {
  schema_version: 1,
  simrep_version: "0.1.0",
  ion_count: 40,
  arrivals: 37,
  splat_counts: { electrode: 3, outside: 0, plane: 37, time_limit: 0 },
  arrival_tof_us: { mean: 15.2998855, std: 0.0213, min: 15.2611, max: 15.3402 },
  compute_ms: 6.84,
  prep_ms: 3.1,
  requested: { device: "cpu", precision: "f64", backend: null, surface_mode: "grid" },
  executed: { device: "cpu", precision: "f64", backend: null, surface_mode: "grid" },
  reason: "explicit --device cpu",
  fallback_reason: null,
  output_paths: ["einzel-results.csv"],
};

const PA_INFO = {
  schema_version: 1,
  simrep_version: "0.1.0",
  file: "einzel.pa0",
  symmetry: "cylindrical",
  field_type: "electrostatic",
  dims: [201, 41, 1],
  spacing_mm: [0.5, 0.5, 0.5],
  points: 8241,
  refinable: false,
  adjustable_electrodes: 3,
};

const text = (value: unknown, isError = false) => ({ content: [{ type: "text", text: typeof value === "string" ? value : JSON.stringify(value, null, 2) }], isError });

export function createDemoAi(manual: boolean, flags: URLSearchParams) {
  const mode = flags.get("ai");
  const outbox: AiEvent[] = [];
  const demo: DemoAi = { step: async () => null, sent: [], queued: () => outbox.length, started: [] };
  const chatgpt: Account[] = [];
  let claude: Account = { id: "claude-local", provider: "claude", label: "Claude local login", authenticated: false, planEnabled: false };
  if (mode === "apikey") claude = { id: "claude-local", provider: "claude", label: "Anthropic API key (environment)", authenticated: true, planEnabled: false };
  // `?ai=device`: providers the UI has no entry for, described only by the bridge's metadata (like ai/providers/table.mjs).
  const extras: ProviderMetadata[] =
    mode === "device"
      ? [
          { id: "github-copilot", label: "GitHub Copilot", authKind: "device-code", models: ["gpt-4.1"], toolCalling: true, withoutPlan: "apiKey" },
          { id: "kimi-coding", label: "Kimi Code", authKind: "device-code", models: ["kimi-for-coding"], toolCalling: true, withoutPlan: "apiKey", regions: ["mainland-cn", "global"], planNote: "Kimi Code subscription. Select the region that hosts your account." },
          { id: "cursor", label: "Cursor", authKind: "oauth", models: [], toolCalling: false, withoutPlan: "blocked", unsupportedReason: "Sign-in only: Cursor has no verified tool-calling path in SimRep yet." },
        ]
      : [];
  const extraAccounts = new Map<Provider, Account>();
  const providerIds: Provider[] = ["chatgpt", "claude", ...extras.map((p) => p.id)];
  const sessions = new Map<string, { provider: Provider; turn: number; turnId: string | null }>();
  const approvals = new Map<string, { sessionId: string; callId: string }>();
  let ids = 0;
  let timer: ReturnType<typeof setTimeout> | null = null;

  const status = (p: Provider): ProviderStatus => {
    const extra = extraAccounts.get(p);
    const accounts = p === "chatgpt" ? chatgpt.map((a) => ({ ...a })) : p === "claude" ? [claude].filter((a) => a.authenticated).map((a) => ({ ...a })) : extra ? [{ ...extra }] : [];
    return { provider: p, authenticated: accounts.some((a) => a.authenticated), accounts };
  };

  const send = async (): Promise<string | null> => {
    const e = outbox.shift();
    if (!e) return null;
    demo.sent.push(e.type);
    await emit(AI_EVENT, e);
    return e.type;
  };
  demo.step = () => (manual ? send() : Promise.resolve(null));
  const queue = (...events: AiEvent[]) => {
    outbox.push(...events);
    if (manual || timer) return;
    const tick = () => {
      timer = null;
      void send().then(() => {
        if (outbox.length) timer = setTimeout(tick, EVENT_STEP_MS);
      });
    };
    timer = setTimeout(tick, EVENT_STEP_MS);
  };

  const words = (s: string) => s.match(/\S+\s*/g) ?? [];
  const chat = (sessionId: string) => {
    const s = sessionId && sessions.get(sessionId)!;
    return { provider: s ? s.provider : ("chatgpt" as Provider), sessionId, activeSkill: "einzel", ...(s && s.turnId ? { turnId: s.turnId } : {}) };
  };
  const deltas = (sessionId: string, s: string): AiEvent[] => words(s).map((w) => ({ type: "delta", ...chat(sessionId), text: w }));

  function turn(sessionId: string, prompt: string) {
    const c = chat(sessionId);
    const callA = `call-${++ids}`;
    const callB = `call-${++ids}`;
    const approvalId = `approval-${++ids}`;
    approvals.set(approvalId, { sessionId, callId: callB });
    const ask = /fly|비행|tof/i.test(prompt) || sessions.get(sessionId)!.turn === 1;
    queue(
      ...deltas(sessionId, "I'll check the refined array first, then fly the ions."),
      { type: "tool_call", ...c, callId: callA, name: "pa_info", arguments: { pa: "einzel.pa0" } },
      { type: "tool_result", ...c, callId: callA, name: "pa_info", result: text(PA_INFO) },
      ...(ask
        ? [
            { type: "tool_call", ...c, callId: callB, name: "fly", arguments: { pa: "einzel.pa0", particles: "einzel.ion", voltage: { "1": 0, "2": 110, "3": 0 }, device: "cpu", output: "einzel-results.csv" } } as AiEvent,
            { type: "approval", ...c, approvalId, name: "fly", arguments: { pa: "einzel.pa0", particles: "einzel.ion", output: "einzel-results.csv" } } as AiEvent,
          ]
        : [{ type: "done", ...c, status: "completed" } as AiEvent]),
    );
  }

  function resolve(approvalId: string, allow: boolean) {
    const a = approvals.get(approvalId);
    if (!a) return;
    approvals.delete(approvalId);
    const c = chat(a.sessionId);
    queue(
      { type: "tool_result", ...c, callId: a.callId, name: "fly", result: allow ? text(FLY_RESULT) : text("User declined tool execution.", true) },
      ...deltas(
        a.sessionId,
        allow
          ? "37 of 40 ions reached the detector plane. Mean TOF is 15.300 µs (σ 0.021 µs) on the CPU in f64; 3 ions hit an electrode. Lowering electrode 2 a little should bring them through."
          : "OK, I didn't fly anything. Tell me if you want different settings.",
      ),
      { type: "done", ...c, status: "completed" },
    );
  }

  function cancel(sessionId: string) {
    // Like the bridge: the rest of the turn is dropped, open approvals are denied, one `done` follows.
    for (let i = outbox.length - 1; i >= 0; i--) if ("sessionId" in outbox[i]! && (outbox[i] as { sessionId?: string }).sessionId === sessionId) outbox.splice(i, 1);
    for (const [id, a] of approvals) if (a.sessionId === sessionId) approvals.delete(id);
    queue({ type: "done", ...chat(sessionId), status: "interrupted" });
  }

  type Handler<M extends Method> = (params: Methods[M]["params"]) => Methods[M]["result"];
  const methods: { [M in Method]: Handler<M> } = {
    "auth.providers": () => ({
      providers: [
        { id: "chatgpt", label: "ChatGPT", authKind: "oauth", models: ["gpt-6.1-sol", "gpt-5.6-terra", "gpt-5.5", "gpt-5.3-codex-spark"], toolCalling: true, withoutPlan: "blocked" },
        { id: "claude", label: "Claude", authKind: "oauth", models: ["sonnet", "opus", "haiku"], toolCalling: true, withoutPlan: "apiKey" },
        ...extras,
      ],
    }),
    "auth.openUsage": () => ({ opened: true }),
    "auth.status": (p) => ({ providers: (p.provider ? [p.provider] : providerIds).map(status) }),
    "auth.accounts": (p) => ({ accounts: (p.provider ? [p.provider] : providerIds).flatMap((x) => status(x).accounts) }),
    "auth.login": (p) => {
      const extra = extras.find((x) => x.id === p.provider);
      if (extra) {
        extraAccounts.set(extra.id, { id: `${extra.id}-demo`, provider: extra.id, label: "researcher", authenticated: true, planEnabled: true });
        // Device authorization: the code to confirm comes first, the `auth` event when the user has.
        if (extra.authKind === "device-code")
          queue({ type: "device_code", provider: extra.id, userCode: "ABCD-EFGH", verificationUri: extra.id === "kimi-coding" ? `https://www.kimi.${p.region === "global" ? "ai" : "com"}/device` : "https://github.com/login/device", expiresIn: 900 });
        queue({ type: "auth", provider: extra.id, status: status(extra.id) });
      } else if (mode === "denied") {
        queue({ type: "error", provider: p.provider, code: "AUTH_DENIED", message: "ChatGPT sign-in was declined. You can sign in again." });
      } else if (p.provider === "chatgpt") {
        const known = p.account ? chatgpt.find((a) => a.id === p.account) : undefined;
        const account: Account = known ?? { id: `chatgpt-demo-${chatgpt.length + 1}`, provider: "chatgpt", label: `researcher@example.com (demo000${chatgpt.length + 1})`, email: "researcher@example.com", authenticated: false, planEnabled: false };
        if (!known) chatgpt.push(account);
        account.authenticated = true;
        account.planEnabled = mode !== "noplan" || Boolean(known);
        queue({ type: "auth", provider: "chatgpt", status: status("chatgpt") });
      } else {
        claude = { id: "claude-local", provider: "claude", label: "researcher@example.com", email: "researcher@example.com", authenticated: true, planEnabled: true };
        queue({ type: "auth", provider: "claude", status: status("claude") });
      }
      return { started: true };
    },
    "auth.logout": (p) => {
      if (p.provider === "claude") {
        if (mode === "apikey") throw { code: "ENV_API_KEY", message: "Remove ANTHROPIC_API_KEY from the app environment to sign out of API-key authentication." };
        claude = { id: "claude-local", provider: "claude", label: "Claude local login", authenticated: false, planEnabled: false };
        return { signedOut: true, remoteRevocationConfirmed: false };
      }
      // Saved registrations stay listed; only their credential goes.
      for (const a of chatgpt) if (!p.account || a.id === p.account) Object.assign(a, { authenticated: false, planEnabled: false });
      return { signedOut: true, remoteRevocationConfirmed: true };
    },
    "chat.start": (p) => {
      const ready = p.provider === "chatgpt" ? chatgpt.find((a) => a.authenticated && (!p.account || a.id === p.account)) : p.provider === "claude" ? (claude.authenticated ? claude : undefined) : extraAccounts.get(p.provider);
      if (!ready) throw { code: "AUTH_REQUIRED", message: "Continue with ChatGPT to sign in." };
      if (p.provider === "chatgpt" && !ready.planEnabled) throw { code: "PLAN_PERMISSION_REQUIRED", message: "ChatGPT plan use is not enabled." };
      const sessionId = `session-${++ids}`;
      demo.started.push({ sessionId, provider: p.provider, account: p.account });
      sessions.set(sessionId, { provider: p.provider, turn: 0, turnId: null });
      return { sessionId, provider: p.provider, model: p.model ?? (p.provider === "claude" ? "sonnet" : "gpt-6.1-sol"), workspace: p.workspace };
    },
    "chat.send": (p) => {
      const s = sessions.get(p.sessionId);
      if (!s) throw { code: "INVALID_PARAMS", message: "Unknown session." };
      if (s.turnId && outbox.some((e) => "sessionId" in e && e.sessionId === p.sessionId)) throw { code: "BUSY", message: "A turn is already running." };
      s.turn++;
      s.turnId = `turn-${++ids}`;
      turn(p.sessionId, p.text);
      return { turnId: s.turnId };
    },
    "chat.cancel": (p) => {
      cancel(p.sessionId);
      return { cancelled: true };
    },
    "approval.resolve": (p) => {
      resolve(p.approvalId, p.allow);
      return { resolved: true };
    },
  };

  /** The `ai_request` command: always a JSON-RPC response, `error.data.code` on failure (like ai.rs). */
  function handle(request: RpcRequest): RpcResponse {
    try {
      const run = methods[request.method] as (params: unknown) => unknown;
      return { jsonrpc: "2.0", id: request.id, result: run(request.params ?? {}) } as RpcResponse;
    } catch (err) {
      const e = err as { code?: string; message?: string };
      return { jsonrpc: "2.0", id: request.id, error: { code: -32000, message: e.message ?? "failed", data: { code: e.code ?? "PROVIDER_ERROR" } } };
    }
  }

  return { demo, handle };
}
