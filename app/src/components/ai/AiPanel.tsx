import { useEffect, useId, useMemo, useRef, useState, type KeyboardEvent } from "react";
import { isTauri } from "@tauri-apps/api/core";
import { ArrowLeft, Bot, Check, CheckCheck, ChevronRight, CircleAlert, CircleSlash, CornerDownRight, Eye, FolderOpen, History, Loader2, MessageSquarePlus, Pencil, Send, ShieldAlert, ShieldOff, Square, Trash2, UserRound, X } from "lucide-react";
import { useI18n } from "../../i18n";
import type { MessageKey } from "../../i18n/en";
import { accountKind, providerState, usableAccounts } from "../../ai/accounts";
import { keyArgs, writesFiles, type ChatItem } from "../../ai/chat";
import { toChatState, type SavedChat } from "../../ai/history";
import type { Provider } from "../../ai/protocol";
import { providerIds, providerInfo } from "../../ai/providers";
import { allowAllInChat, answerApproval, approvalModeOf, cancel, continueChat, deleteSavedChat, newChat, openHistory, openPanel, openSettings, refreshHistory, send, setApproval, setProvider, useAi, viewChat, workspaceOf } from "../../ai/store";
import { allows, unavailable } from "../../host";
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
import dialog from "../Dialog.module.css";
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
  // Sign-in-only providers (no verified tool calling) stay in AI accounts, not in the chat picker.
  const providers = providerIds(reported).filter((p) => providerInfo(p).toolCalling !== false);
  // The bridge's live model list when it reported one, else the provider's catalog.
  const models = ["", ...(state.status?.models ?? info.models).filter((m) => m !== "")];
  const loaded = useAi((s) => s.loaded);
  const activeChat = useAi((s) => s.chat);
  const viewing = useAi((s) => s.viewing);
  const historyOpen = useAi((s) => s.historyOpen);
  const continuedFrom = useAi((s) => s.saved?.continuedFrom);
  // A saved chat opened read-only replaces the transcript until Back, Continue or New chat.
  const viewed = useMemo(() => (viewing ? toChatState(viewing) : null), [viewing]);
  const chat = viewed ?? activeChat;
  const starting = useAi((s) => s.starting);
  const approval = useAi((s) => s.approval);
  const mode = useAi(approvalModeOf);
  const [confirming, setConfirming] = useState(false);
  const project = useApp((s) => s.project);
  const workspace = workspaceOf(project);
  const usable = usableAccounts(state);
  const active = usable.find((a) => a.id === account) ?? usable[0] ?? null;
  const [draft, setDraft] = useState("");
  const list = useRef<HTMLOListElement>(null);
  const input = useRef<HTMLTextAreaElement>(null);
  const desktop = isTauri() && allows("ai");

  // The previous-chats list follows the open project.
  useEffect(() => {
    void refreshHistory();
  }, [project.path, project.dir]);

  // Follow the stream: keep the newest item in view.
  useEffect(() => {
    const el = list.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [chat.items]);

  const busy = chat.running || starting;
  const ready = desktop && !viewing && !historyOpen && info.toolCalling !== false && Boolean(active) && Boolean(workspace);
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
  const blocked = unavailable("ai") ?? refusals.aiDesktopOnly();
  const using = !active ? null : accountKind(active) === "apiKey" ? t("ai.panel.usingApiKey", { provider: info.label }) : info.usingLabel ? t(info.usingLabel) : t("ai.panel.usingAccount", { account: active.label });
  const last = chat.items.at(-1);
  const streaming = chat.running && last?.kind === "assistant" ? last.id : null;
  const historyLabel = t(historyOpen || viewing ? "ai.history.back" : "ai.history.open");

  return (
    <aside className={`${panel.panel} ${styles.panel}`} aria-labelledby="ai-panel-title" data-testid="ai-panel" onKeyDown={onPanelKey}>
      <div className={panel.header}>
        <Bot className={styles.headerIcon} aria-hidden />
        <h2 id="ai-panel-title" className={panel.title}>
          {t("ai.panel.title")}
        </h2>
        {chat.activeSkill && <span data-testid="ai-active-skill"><Badge>{chat.activeSkill}</Badge></span>}
        <select
          aria-label={t("ai.approval.mode")}
          title={t("ai.approval.mode")}
          className={`${inputs.input} ${inputs.select} ${styles.select} ${styles.approvalSelect}`}
          value={approval}
          onChange={(e) => (e.target.value === "bypass" ? setConfirming(true) : void setApproval("ask"))}
          data-testid="ai-approval-mode"
        >
          <option value="ask">{`${t("ai.approval.mode")}: ${t("ai.approval.modeAsk")}`}</option>
          <option value="bypass">{`${t("ai.approval.mode")}: ${t("ai.approval.modeBypass")}`}</option>
        </select>
        <Button
          variant="ghost"
          size="sm"
          iconOnly
          icon={historyOpen || viewing ? <ArrowLeft /> : <History />}
          aria-label={historyLabel}
          title={historyLabel}
          aria-expanded={historyOpen}
          onClick={() => (viewing ? viewChat(null) : openHistory(!historyOpen))}
          data-testid="ai-history-toggle"
        />
        <Button variant="ghost" size="sm" iconOnly icon={<MessageSquarePlus />} disabled={activeChat.running || (activeChat.items.length === 0 && !viewing && !historyOpen && !continuedFrom)} aria-label={t("ai.panel.newChat")} title={t("ai.panel.newChat")} onClick={newChat} />
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
            {models.map((m) => (
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

      {mode !== "ask" && (
        // Words, not only colour: the chip names the state and the line says what it means.
        <div className={styles.skipping} role="status" data-testid="ai-approval-skipping" data-mode={mode}>
          <span className={styles.chip} data-tone="warning">
            <ShieldOff aria-hidden />
            {t("ai.approval.skipping")}
          </span>
          <span className={styles.skippingText}>{t(mode === "bypass" ? "ai.approval.skippingBypass" : "ai.approval.skippingChat")}</span>
          <Button size="sm" variant="ghost" onClick={() => void setApproval("ask")}>
            {t("ai.approval.askAgain")}
          </Button>
        </div>
      )}
      {confirming && <BypassConfirm onClose={() => setConfirming(false)} />}

      {viewing && (
        <div className={styles.savedBar} role="status" data-testid="ai-history-readonly">
          <span className={styles.chip} data-tone="neutral">
            <Eye aria-hidden />
            {t("ai.history.readOnly")}
          </span>
          <span className={styles.skippingText}>{t("ai.history.readOnlyBody")}</span>
          <Button size="sm" variant="primary" icon={<CornerDownRight />} disabled={busy} onClick={() => continueChat(viewing)} data-testid="ai-history-continue-open">
            {t("ai.history.continue")}
          </Button>
        </div>
      )}
      {!viewing && !historyOpen && continuedFrom && (
        <div className={styles.savedBar} data-continued role="note" data-testid="ai-continued">
          <span className={styles.chip} data-tone="info">
            <CornerDownRight aria-hidden />
            {t("ai.history.continued")}
          </span>
          <span className={styles.skippingText}>{t("ai.history.continuedBody", { title: continuedFrom.title || t("ai.history.untitled") })}</span>
        </div>
      )}

      {historyOpen ? (
        <HistoryList busy={busy} />
      ) : (
      <ol ref={list} className={styles.transcript} aria-live={viewing ? "off" : "polite"} aria-label={viewing ? `${t("ai.panel.transcript")} (${t("ai.history.readOnly")})` : t("ai.panel.transcript")} data-readonly={viewing ? true : undefined}>
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
        ) : chat.items.length === 0 && !viewing ? (
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
          chat.items.map((item) => <Item key={item.id} item={item} provider={viewing?.provider ?? chat.session?.provider ?? provider} streaming={item.id === streaming} />)
        )}
        {chat.running && last?.kind === "user" && (
          <li className={styles.thinking} role="status">
            <Loader2 className={styles.spin} aria-hidden />
            {t("ai.panel.thinking")}
          </li>
        )}
      </ol>
      )}

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

/** The open project's saved chats: title (first message), date, provider and model; open, continue, delete. */
function HistoryList({ busy }: { busy: boolean }) {
  const { t, lang } = useI18n();
  const history = useAi((s) => s.history);
  const ready = useAi((s) => s.historyReady);
  const [confirming, setConfirming] = useState<string | null>(null);
  const date = (ms: number) => new Intl.DateTimeFormat(lang === "ko" ? "ko-KR" : "en-US", { dateStyle: "medium", timeStyle: "short" }).format(new Date(ms));
  const titleOf = (c: SavedChat) => c.title || t("ai.history.untitled");
  return (
    <section className={styles.history} aria-labelledby="ai-history-title" data-testid="ai-history">
      <h3 id="ai-history-title" className={styles.historyTitle}>
        {t("ai.history.title")}
      </h3>
      {!ready ? (
        <p className={styles.historyEmpty}>{t("ai.history.unsaved")}</p>
      ) : history.length === 0 ? (
        <p className={styles.historyEmpty}>{t("ai.history.empty")}</p>
      ) : (
        <ul className={styles.historyList} aria-label={t("ai.history.list")}>
          {history.map((c) => {
            const title = titleOf(c);
            const messages = c.items.filter((i) => i.kind === "user" || i.kind === "assistant").length;
            return (
              <li key={c.id} className={styles.historyItem} data-testid="ai-history-item" data-chat={c.id}>
                <button type="button" className={styles.historyMain} onClick={() => viewChat(c)} title={t("ai.history.viewTitle", { title })} aria-label={t("ai.history.viewTitle", { title })}>
                  <span className={styles.historyName}>{title}</span>
                  <span className={styles.historyMeta}>
                    <span className={styles.historyMark} data-provider={c.provider} aria-hidden>
                      <ProviderMark provider={c.provider} label={c.providerLabel} />
                    </span>
                    <span className="num">{t("ai.history.meta", { provider: c.providerLabel, model: c.model || t("ai.history.modelDefault") })}</span>
                    <span aria-hidden>·</span>
                    <time dateTime={new Date(c.updatedAt).toISOString()} className="tnum">
                      {date(c.updatedAt)}
                    </time>
                    <span aria-hidden>·</span>
                    <span className="tnum">{t("ai.history.messages", { n: messages })}</span>
                    {c.continuedFrom && (
                      <span className={styles.historyContinued} title={t("ai.history.continuedBody", { title: c.continuedFrom.title || t("ai.history.untitled") })}>
                        <CornerDownRight aria-hidden />
                        {t("ai.history.continued")}
                      </span>
                    )}
                  </span>
                </button>
                {confirming === c.id ? (
                  <div className={styles.historyActions} role="group" aria-label={t("ai.history.deleteConfirm")}>
                    <span className={styles.historyConfirm}>{t("ai.history.deleteConfirm")}</span>
                    <Button size="sm" autoFocus onClick={() => setConfirming(null)}>
                      {t("ai.history.deleteKeep")}
                    </Button>
                    <Button size="sm" variant="danger" icon={<Trash2 />} onClick={() => { setConfirming(null); void deleteSavedChat(c.id); }} data-testid="ai-history-delete-confirm">
                      {t("ai.history.delete")}
                    </Button>
                  </div>
                ) : (
                  <div className={styles.historyActions}>
                    <Button size="sm" icon={<CornerDownRight />} disabled={busy} title={t("ai.history.continueTitle", { title })} aria-label={t("ai.history.continueTitle", { title })} onClick={() => continueChat(c)} data-testid="ai-history-continue">
                      {t("ai.history.continue")}
                    </Button>
                    <Button size="sm" variant="ghost" iconOnly icon={<Trash2 />} title={t("ai.history.deleteTitle", { title })} aria-label={t("ai.history.deleteTitle", { title })} onClick={() => setConfirming(c.id)} data-testid="ai-history-delete" />
                  </div>
                )}
              </li>
            );
          })}
        </ul>
      )}
    </section>
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
            {item.code === "PLAN_PERMISSION_REQUIRED" || item.code === "AUTH_REQUIRED" || item.code === "AUTH_EXPIRED" ? (
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
            {item.auto && (
              <span className={styles.autoMark} title={t("ai.approval.autoTitle")} data-testid="ai-auto">
                <span aria-hidden>{t("ai.approval.auto")}</span>
                <span className="visually-hidden">{t("ai.approval.autoTitle")}</span>
              </span>
            )}
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
  const left = Math.max(0, (item.expiresAt ?? approvalSeen.get(item.approvalId)! + APPROVAL_MS) - now);
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
          <Button size="sm" icon={<CheckCheck />} onClick={() => void allowAllInChat(item.approvalId)} data-testid="ai-allow-all">
            {t("ai.approval.allowAll")}
          </Button>
          <Button size="sm" variant="primary" icon={<Check />} onClick={() => void answerApproval(item.approvalId, true)}>
            {t("ai.approval.allow")}
          </Button>
        </div>
      )}
    </li>
  );
}

/** The one confirmation before Bypass: what runs without asking, and where. Escape or Keep asking cancels. */
function BypassConfirm({ onClose }: { onClose(): void }) {
  const { t } = useI18n();
  const id = useId();
  const [remember, setRemember] = useState(false);
  const keep = useRef<HTMLButtonElement>(null);
  useEffect(() => keep.current?.focus(), []);
  useEffect(() => {
    const onKey = (e: globalThis.KeyboardEvent) => {
      if (e.key !== "Escape") return;
      e.stopPropagation();
      onClose();
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [onClose]);
  const confirm = () => {
    onClose();
    void setApproval("bypass", remember);
  };
  return (
    <div className={dialog.scrim} onClick={onClose}>
      <div className={`${dialog.dialog} ${styles.bypassDialog}`} role="alertdialog" aria-modal="true" aria-labelledby={`${id}-title`} aria-describedby={`${id}-body`} onClick={(e) => e.stopPropagation()} data-testid="ai-bypass-confirm">
        <header className={dialog.header}>
          <ShieldOff className={dialog.headerIcon} aria-hidden />
          <h2 id={`${id}-title`} className={dialog.title}>
            {t("ai.bypass.title")}
          </h2>
        </header>
        <p id={`${id}-body`} className={styles.intro}>
          {t("ai.bypass.body")}
        </p>
        <label className={styles.remember}>
          <input type="checkbox" checked={remember} onChange={(e) => setRemember(e.target.checked)} />
          {t("ai.bypass.remember")}
        </label>
        <div className={styles.approvalActions}>
          <Button ref={keep} size="sm" onClick={onClose}>
            {t("ai.bypass.cancel")}
          </Button>
          <Button size="sm" variant="danger" icon={<ShieldOff />} onClick={confirm}>
            {t("ai.bypass.confirm")}
          </Button>
        </div>
      </div>
    </div>
  );
}
