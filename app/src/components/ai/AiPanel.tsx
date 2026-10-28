import { useEffect, useRef, useState, type KeyboardEvent } from "react";
import { isTauri } from "@tauri-apps/api/core";
import { Bot, Check, ChevronRight, CircleAlert, CircleSlash, FolderOpen, Loader2, MessageSquarePlus, Pencil, Send, ShieldAlert, Square, UserRound, X } from "lucide-react";
import { useI18n } from "../../i18n";
import type { MessageKey } from "../../i18n/en";
import { accountKind, providerState, usableAccounts } from "../../ai/accounts";
import { keyArgs, writesFiles, type ChatItem } from "../../ai/chat";
import type { Provider } from "../../ai/protocol";
import { providerIds, providerInfo } from "../../ai/providers";
import { answerApproval, cancel, newChat, openPanel, openSettings, send, setProvider, useAi, workspaceOf } from "../../ai/store";
import { refusals } from "../../refusals";
import { useApp } from "../../state/store";
import { Button } from "../ui/Button";
import { Badge } from "../ui/Badge";
import { Kbd } from "../ui/Kbd";
import { SegmentedControl } from "../ui/SegmentedControl";
import { ManageUsageLink } from "./AiNotice";
import { StatusChip } from "./AiAccountsDialog";
import { displayLabel, ProviderMark } from "./BrandMarks";
import { clock, useNow } from "./time";
import inputs from "../ui/inputs.module.css";
import panel from "../Panel.module.css";
import styles from "./Ai.module.css";

/** Example prompts of the empty chat; a click puts one in the composer. */
const EXAMPLES: readonly MessageKey[] = ["ai.panel.example.einzel", "ai.panel.example.esa", "ai.panel.example.step"];
/** More providers than this get a menu instead of a segmented control. */
const MAX_SEGMENTS = 3;

/** The AI assistant: provider + model, the project folder as workspace, the streamed transcript. */
export function AiPanel() {
  const { t, tm } = useI18n();
  const provider = useAi((s) => s.provider);
  const model = useAi((s) => s.model);
  const account = useAi((s) => s.account);
  const state = useAi((s) => providerState(s.accounts, s.provider));
  const reported = Object.keys(useAi((s) => s.accounts));
  const listed = providerInfo(provider);
  const info = { ...listed, label: displayLabel(provider, listed.label) };
  const loaded = useAi((s) => s.loaded);
  const chat = useAi((s) => s.chat);
  const starting = useAi((s) => s.starting);
  const project = useApp((s) => s.project);
  const workspace = workspaceOf(project);
  const usable = usableAccounts(state);
  const active = usable.find((a) => a.id === account) ?? usable[0] ?? null;
  const [draft, setDraft] = useState("");
  const list = useRef<HTMLOListElement>(null);
  const input = useRef<HTMLTextAreaElement>(null);
  const desktop = isTauri();

  // Follow the stream: keep the newest item in view.
  useEffect(() => {
    const el = list.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [chat.items]);

  const busy = chat.running || starting;
  const ready = desktop && Boolean(active) && Boolean(workspace);
  const canSend = ready && !busy && draft.trim().length > 0;
  const submit = () => {
    if (!canSend) return;
    void send(draft.trim());
    setDraft("");
  };
  const onKey = (e: KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) {
      e.preventDefault();
      submit();
    }
  };
  // Escape anywhere in the panel stops a running reply (and nothing else on the page).
  const onPanelKey = (e: KeyboardEvent<HTMLElement>) => {
    if (e.key === "Escape" && chat.running && !e.nativeEvent.isComposing) {
      e.preventDefault();
      e.stopPropagation();
      void cancel();
    }
  };
  const example = (key: MessageKey) => {
    setDraft(t(key));
    input.current?.focus();
  };
  const blocked = refusals.aiDesktopOnly();
  const using = !active ? null : accountKind(active) === "apiKey" ? t("ai.panel.usingApiKey", { provider: info.label }) : info.usingLabel ? t(info.usingLabel) : t("ai.panel.usingAccount", { account: active.label });
  const providers = providerIds(reported);
  const last = chat.items.at(-1);
  const streaming = chat.running && last?.kind === "assistant" ? last.id : null;

  return (
    <aside className={`${panel.panel} ${styles.panel}`} aria-labelledby="ai-panel-title" data-testid="ai-panel" onKeyDown={onPanelKey}>
      <div className={panel.header}>
        <Bot className={styles.headerIcon} aria-hidden />
        <h2 id="ai-panel-title" className={panel.title}>
          {t("ai.panel.title")}
        </h2>
        {chat.activeSkill && <span data-testid="ai-active-skill"><Badge>{chat.activeSkill}</Badge></span>}
        <Button variant="ghost" size="sm" iconOnly icon={<MessageSquarePlus />} disabled={chat.running || chat.items.length === 0} aria-label={t("ai.panel.newChat")} title={t("ai.panel.newChat")} onClick={newChat} />
        <Button variant="ghost" size="sm" iconOnly icon={<X />} aria-label={t("ai.close")} title={t("ai.close")} onClick={() => openPanel(false)} />
      </div>
      <div className={styles.controls}>
        <div className={styles.pickers}>
          {providers.length <= MAX_SEGMENTS ? (
            <SegmentedControl<Provider>
              ariaLabel={t("ai.panel.provider")}
              value={provider}
              onChange={(p) => !busy && setProvider(p)}
              options={providers.map((p) => ({
                value: p,
                label: displayLabel(p, providerInfo(p).label),
                icon: <ProviderMark provider={p} label={displayLabel(p, providerInfo(p).label)} />,
                title: displayLabel(p, providerInfo(p).label),
              }))}
            />
          ) : (
            <select aria-label={t("ai.panel.provider")} className={`${inputs.input} ${inputs.select} ${styles.select}`} value={provider} disabled={busy} onChange={(e) => setProvider(e.target.value)}>
              {providers.map((p) => (
                <option key={p} value={p}>
                  {displayLabel(p, providerInfo(p).label)}
                </option>
              ))}
            </select>
          )}
          <select id="ai-model" aria-label={t("ai.panel.model")} title={t("ai.panel.model")} className={`${inputs.input} ${inputs.select} ${styles.select} ${styles.modelSelect} num`} value={model} disabled={busy} onChange={(e) => useAi.setState({ model: e.target.value })}>
            {info.models.map((m) => (
              <option key={m} value={m}>
                {m || t("ai.panel.modelDefault")}
              </option>
            ))}
          </select>
        </div>
        {usable.length > 1 && (
          <select id="ai-account" aria-label={t("ai.panel.account")} title={t("ai.panel.account")} className={`${inputs.input} ${inputs.select} ${styles.select}`} value={active?.id ?? ""} disabled={busy} onChange={(e) => useAi.setState({ account: e.target.value })}>
            {usable.map((a) => (
              <option key={a.id} value={a.id}>
                {a.label}
              </option>
            ))}
          </select>
        )}
      </div>

      <ol ref={list} className={styles.transcript} aria-live="polite" aria-label={t("ai.panel.transcript")}>
        {!desktop ? (
          <li className={styles.empty}>
            <p className={styles.emptyTitle}>{tm(blocked.what)}</p>
            <p>{tm(blocked.why)}</p>
          </li>
        ) : !loaded ? (
          <li className={styles.empty}>{t("ai.settings.loading")}</li>
        ) : !active ? (
          <li className={styles.empty}>
            <span className={styles.emptyMark} data-provider={provider}>
              <ProviderMark provider={provider} label={info.label} />
            </span>
            <p className={styles.emptyTitle}>{t("ai.panel.notSignedIn", { provider: info.label })}</p>
            <p>{t("ai.panel.notSignedInBody")}</p>
            <Button size="sm" variant="primary" icon={<UserRound />} onClick={() => openSettings(true)}>
              {t("ai.panel.openAccounts")}
            </Button>
          </li>
        ) : !workspace ? (
          <li className={styles.empty}>
            <FolderOpen className={styles.emptyIcon} aria-hidden />
            <p>{t("ai.panel.noWorkspace")}</p>
          </li>
        ) : chat.items.length === 0 ? (
          <li className={styles.empty} data-testid="ai-empty">
            <Bot className={styles.emptyIcon} aria-hidden />
            <p className={styles.emptyTitle}>{t("ai.panel.emptyTitle")}</p>
            <p>{t("ai.panel.empty")}</p>
            <div className={styles.examples} role="group" aria-label={t("ai.panel.examples")}>
              {EXAMPLES.map((k) => (
                <button key={k} type="button" className={styles.example} onClick={() => example(k)} data-testid="ai-example">
                  {t(k)}
                </button>
              ))}
            </div>
          </li>
        ) : (
          chat.items.map((item) => <Item key={item.id} item={item} provider={chat.session?.provider ?? provider} streaming={item.id === streaming} />)
        )}
        {chat.running && last?.kind === "user" && (
          <li className={styles.thinking} role="status">
            <Loader2 className={styles.spin} aria-hidden />
            {t("ai.panel.thinking")}
          </li>
        )}
      </ol>

      <div className={styles.composer}>
        {/* Where the assistant works and whose plan it uses, next to the composer (SIWC: "Using ChatGPT plan"). */}
        <div className={styles.context}>
          <span className={styles.workspace} title={workspace ? `${t("ai.panel.workspace")}: ${workspace}` : t("ai.panel.workspace")}>
            <FolderOpen aria-hidden />
            <span className="visually-hidden">{t("ai.panel.workspace")}: </span>
            <span className={`${styles.ellipsis} ${styles.path}`}>{workspace ?? "–"}</span>
          </span>
          {using && (
            <span className={styles.using} data-testid="ai-using">
              <span className={styles.ellipsis}>{using}</span>
              <ManageUsageLink provider={provider} />
            </span>
          )}
        </div>
        <div className={styles.composerBox} data-disabled={!ready || undefined}>
          <textarea
            ref={input}
            className={styles.textarea}
            aria-label={t("ai.panel.placeholder")}
            aria-describedby="ai-send-hint"
            placeholder={t("ai.panel.placeholder")}
            value={draft}
            rows={2}
            disabled={!ready}
            onChange={(e) => setDraft(e.target.value)}
            onKeyDown={onKey}
          />
          <div className={styles.composerRow}>
            <span id="ai-send-hint" className={styles.hint}>
              {chat.running ? (
                <>
                  <Kbd>Esc</Kbd> {t("ai.panel.stopHint")}
                </>
              ) : (
                <>
                  <Kbd>Enter</Kbd> {t("ai.panel.sendKey")} <span aria-hidden>·</span> <Kbd>Shift+Enter</Kbd> {t("ai.panel.newlineKey")}
                </>
              )}
            </span>
            {chat.running ? (
              <Button size="sm" variant="danger" icon={<Square />} onClick={() => void cancel()} data-testid="ai-stop" title={t("ai.panel.stopTitle")}>
                {t("ai.panel.stop")}
              </Button>
            ) : (
              <Button size="sm" variant="primary" icon={<Send />} disabled={!canSend} onClick={submit} data-testid="ai-send">
                {t("ai.panel.send")}
              </Button>
            )}
          </div>
        </div>
      </div>
    </aside>
  );
}

function Item({ item, provider, streaming }: { item: ChatItem; provider: Provider; streaming: boolean }) {
  const { t, tm } = useI18n();
  switch (item.kind) {
    case "user":
      return (
        <li className={styles.user}>
          <span className="visually-hidden">{t("ai.panel.you")}: </span>
          {/* Plain text: React escapes it; nothing from the model or tools is parsed as HTML. */}
          <p className={styles.bubbleText}>{item.text}</p>
        </li>
      );
    case "assistant":
      return (
        <li className={styles.assistant} data-streaming={streaming || undefined}>
          <span className={styles.avatar} data-provider={provider} aria-hidden>
            <ProviderMark provider={provider} label={displayLabel(provider, providerInfo(provider).label)} />
          </span>
          <p className={styles.bubbleText}>
            <span className="visually-hidden">{t("ai.panel.assistant")}: </span>
            {item.text}
            {streaming && <span className={styles.caret} aria-hidden />}
          </p>
        </li>
      );
    case "tool":
      return <ToolCard item={item} />;
    case "approval":
      return <ApprovalCard item={item} />;
    case "error": {
      const r = refusals.ai("chat", item.code);
      return (
        <li className={styles.notice} data-tone="danger" role="alert" data-code={item.code}>
          <CircleAlert aria-hidden />
          <div className={styles.noticeText}>
            <p className={styles.noticeTitle}>{tm(r.what)}</p>
            <p className={styles.noticeBody}>{tm(r.why)}</p>
            {item.code === "PLAN_PERMISSION_REQUIRED" || item.code === "AUTH_REQUIRED" ? (
              <div className={styles.row}>
                <Button size="sm" onClick={() => openSettings(true)}>
                  {t("ai.panel.openAccounts")}
                </Button>
                <ManageUsageLink provider={item.provider} />
              </div>
            ) : null}
          </div>
        </li>
      );
    }
    case "ended":
      return (
        <li className={styles.ended} data-status={item.status}>
          <CircleSlash aria-hidden />
          {t(item.status === "interrupted" ? "ai.ended.interrupted" : "ai.ended.failed")}
        </li>
      );
  }
}

type ToolItem = Extract<ChatItem, { kind: "tool" }>;
const STATUS: Record<ToolItem["status"], { key: MessageKey; tone: "info" | "success" | "danger" | "neutral" }> = {
  running: { key: "ai.tool.running", tone: "info" },
  done: { key: "ai.tool.done", tone: "success" },
  error: { key: "ai.tool.error", tone: "danger" },
  skipped: { key: "ai.tool.skipped", tone: "neutral" },
};

function StatusIcon({ status }: { status: ToolItem["status"] }) {
  if (status === "running") return <Loader2 className={styles.spin} aria-hidden />;
  if (status === "done") return <Check aria-hidden />;
  if (status === "error") return <X aria-hidden />;
  return <CircleSlash aria-hidden />;
}

/** One SimRep tool call as a timeline entry: status, name, its main argument, result numbers, details. */
function ToolCard({ item }: { item: ToolItem }) {
  const { t } = useI18n();
  const args = keyArgs(item.args);
  const status = STATUS[item.status];
  const writes = writesFiles(item.name);
  const lead = args[0];
  // The collapsed card still shows its headline numbers: arrivals and TOF when there are any.
  const fields = item.result?.fields ?? [];
  const headline = fields.filter((f) => f.key === "arrivals" || f.key === "arrival_tof_us" || f.key === "converged");
  const glance = (headline.length ? headline : fields).slice(0, 2);
  return (
    <li className={styles.tool} data-status={item.status} data-testid="ai-tool-card" data-tool={item.name}>
      <span className={styles.toolDot} data-tone={status.tone}>
        <StatusIcon status={item.status} />
      </span>
      <div className={styles.toolCol}>
        <details className={styles.toolBody}>
          <summary className={styles.toolHead}>
            <span className={`${styles.toolName} num`}>{item.name}</span>
            {lead && (
              <span className={`${styles.toolArg} num`} title={`${lead.key}: ${lead.value}`}>
                {lead.value}
              </span>
            )}
            <span className={styles.spacer} />
            <StatusChip tone={status.tone}>{t(status.key)}</StatusChip>
            <ChevronRight className={styles.chevron} aria-hidden />
            {glance.length > 0 && (
              <span className={`${styles.toolGlance} num`}>
                {glance.map((f) => `${t(`ai.field.${f.key}` as MessageKey)} ${f.value}`).join(" · ")}
              </span>
            )}
          </summary>
          <div className={styles.toolDetails}>
            {item.result && item.result.fields.length > 0 && (
              <dl className={styles.kv} aria-label={t("ai.tool.result")}>
                {item.result.fields.map((f) => (
                  <div key={f.key} className={styles.kvRow} data-field={f.key}>
                    <dt>{t(`ai.field.${f.key}` as MessageKey)}</dt>
                    <dd className="num" title={f.value}>
                      {f.value}
                    </dd>
                  </div>
                ))}
              </dl>
            )}
            {args.length > 0 && (
              <>
                <p className={styles.toolMeta}>
                  {writes ? <Pencil aria-hidden /> : null}
                  {t("ai.tool.args")} · {t(writes ? "ai.tool.writes" : "ai.tool.readOnly")}
                </p>
                <dl className={styles.kv}>
                  {args.map((a) => (
                    <div key={a.key} className={styles.kvRow}>
                      <dt className="num">{a.key}</dt>
                      <dd className="num" title={a.value}>
                        {a.value}
                      </dd>
                    </div>
                  ))}
                </dl>
              </>
            )}
          </div>
        </details>
        {/* A failure explains itself without a click. */}
        {item.result?.text && <pre className={styles.toolText}>{item.result.text}</pre>}
      </div>
    </li>
  );
}

/** The bridge denies an unanswered approval after this long (ai.approval.body says 2 minutes). */
const APPROVAL_MS = 2 * 60 * 1000;
/** When each approval was first shown, so the countdown survives closing and reopening the panel. */
const approvalSeen = new Map<string, number>();

function ApprovalCard({ item }: { item: Extract<ChatItem, { kind: "approval" }> }) {
  const { t } = useI18n();
  const args = keyArgs(item.args);
  const pending = item.state === "pending";
  if (!approvalSeen.has(item.approvalId)) approvalSeen.set(item.approvalId, Date.now());
  const now = useNow(pending);
  const left = Math.max(0, approvalSeen.get(item.approvalId)! + APPROVAL_MS - now);
  const done: Record<Exclude<typeof item.state, "pending">, MessageKey> = { allowed: "ai.approval.allowed", denied: "ai.approval.denied", expired: "ai.approval.expired" };
  return (
    <li className={styles.approval} data-state={item.state} data-testid="ai-approval" role={pending ? "group" : undefined} aria-labelledby={`approval-${item.id}`}>
      <div className={styles.approvalHead}>
        <ShieldAlert aria-hidden />
        <p id={`approval-${item.id}`} className={styles.noticeTitle}>
          {t("ai.approval.title", { tool: item.name })}
        </p>
        <span className={styles.spacer} />
        {item.state === "pending" ? (
          <span className={`${styles.countdown} tnum`} title={t("ai.approval.expiresTitle")}>
            {t("ai.approval.left", { time: clock(left) })}
          </span>
        ) : (
          <StatusChip tone={item.state === "allowed" ? "success" : "neutral"}>{t(done[item.state])}</StatusChip>
        )}
      </div>
      {pending && <p className={styles.noticeBody}>{t("ai.approval.body")}</p>}
      {/* Once answered, the arguments live on the tool card itself; the approval shrinks to one line. */}
      {pending && args.length > 0 && (
        <dl className={styles.kv}>
          {args.map((a) => (
            <div key={a.key} className={styles.kvRow}>
              <dt className="num">{a.key}</dt>
              <dd className="num" title={a.value}>
                {a.value}
              </dd>
            </div>
          ))}
        </dl>
      )}
      {pending && (
        <div className={styles.approvalActions}>
          <Button size="sm" icon={<X />} onClick={() => void answerApproval(item.approvalId, false)}>
            {t("ai.approval.deny")}
          </Button>
          <Button size="sm" variant="primary" icon={<Check />} onClick={() => void answerApproval(item.approvalId, true)}>
            {t("ai.approval.allow")}
          </Button>
        </div>
      )}
    </li>
  );
}
