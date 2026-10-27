import { useEffect, useRef, useState, type KeyboardEvent } from "react";
import { isTauri } from "@tauri-apps/api/core";
import { Bot, Check, CircleAlert, CircleSlash, FolderOpen, Loader2, MessageSquarePlus, Send, ShieldAlert, Square, UserRound, Wrench, X } from "lucide-react";
import { useI18n } from "../../i18n";
import type { MessageKey } from "../../i18n/en";
import { accountKind, providerState, usableAccounts } from "../../ai/accounts";
import { keyArgs, writesFiles, type ChatItem } from "../../ai/chat";
import type { Provider } from "../../ai/protocol";
import { providerIds, providerInfo } from "../../ai/providers";
import { answerApproval, cancel, newChat, openPanel, openSettings, send, setProvider, useAi, workspaceOf } from "../../ai/store";
import { refusals } from "../../refusals";
import { useApp } from "../../state/store";
import { Badge } from "../ui/Badge";
import { Button } from "../ui/Button";
import { SegmentedControl } from "../ui/SegmentedControl";
import { ManageUsageLink } from "./AiNotice";
import { ProviderMark } from "./BrandMarks";
import inputs from "../ui/inputs.module.css";
import panel from "../Panel.module.css";
import styles from "./Ai.module.css";

/** The AI assistant: provider + model, the project folder as workspace, the streamed transcript. */
export function AiPanel() {
  const { t, tm } = useI18n();
  const provider = useAi((s) => s.provider);
  const model = useAi((s) => s.model);
  const account = useAi((s) => s.account);
  const state = useAi((s) => providerState(s.accounts, s.provider));
  const reported = Object.keys(useAi((s) => s.accounts));
  const info = providerInfo(provider);
  const loaded = useAi((s) => s.loaded);
  const chat = useAi((s) => s.chat);
  const starting = useAi((s) => s.starting);
  const project = useApp((s) => s.project);
  const workspace = workspaceOf(project);
  const usable = usableAccounts(state);
  const active = usable.find((a) => a.id === account) ?? usable[0] ?? null;
  const [draft, setDraft] = useState("");
  const list = useRef<HTMLOListElement>(null);
  const desktop = isTauri();

  // Follow the stream: keep the newest item in view.
  useEffect(() => {
    const el = list.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [chat.items]);

  const busy = chat.running || starting;
  const canSend = desktop && Boolean(active) && Boolean(workspace) && !busy && draft.trim().length > 0;
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
  const blocked = refusals.aiDesktopOnly();
  const using = !active ? null : accountKind(active) === "apiKey" ? t("ai.panel.usingApiKey", { provider: info.label }) : info.usingLabel ? t(info.usingLabel) : t("ai.panel.usingAccount", { account: active.label });

  return (
    <aside className={`${panel.panel} ${styles.panel}`} aria-labelledby="ai-panel-title" data-testid="ai-panel">
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
        <SegmentedControl<Provider>
          ariaLabel={t("ai.panel.provider")}
          value={provider}
          onChange={(p) => !busy && setProvider(p)}
          options={providerIds(reported).map((p) => ({
            value: p,
            label: providerInfo(p).label,
            icon: <ProviderMark provider={p} />,
            title: providerInfo(p).label,
          }))}
        />
        <div className={styles.controlRow}>
          <label className={styles.controlLabel} htmlFor="ai-model">
            {t("ai.panel.model")}
          </label>
          <select id="ai-model" className={`${inputs.input} ${inputs.select} ${styles.select}`} value={model} disabled={busy} onChange={(e) => useAi.setState({ model: e.target.value })}>
            {info.models.map((m) => (
              <option key={m} value={m}>
                {m || t("ai.panel.modelDefault")}
              </option>
            ))}
          </select>
        </div>
        {usable.length > 1 && (
          <div className={styles.controlRow}>
            <label className={styles.controlLabel} htmlFor="ai-account">
              {t("ai.panel.account")}
            </label>
            <select id="ai-account" className={`${inputs.input} ${inputs.select} ${styles.select}`} value={active?.id ?? ""} disabled={busy} onChange={(e) => useAi.setState({ account: e.target.value })}>
              {usable.map((a) => (
                <option key={a.id} value={a.id}>
                  {a.label}
                </option>
              ))}
            </select>
          </div>
        )}
        <div className={styles.controlRow}>
          <span className={styles.controlLabel}>{t("ai.panel.workspace")}</span>
          <span className={`${styles.workspace} num`} title={workspace ?? undefined}>
            <FolderOpen aria-hidden />
            <span className={styles.ellipsis}>{workspace ?? "–"}</span>
          </span>
        </div>
        {using && (
          <div className={styles.using} data-testid="ai-using">
            <span>{using}</span>
            <ManageUsageLink provider={provider} />
          </div>
        )}
      </div>

      <ol ref={list} className={styles.transcript} aria-live="polite" aria-label={t("ai.panel.title")}>
        {!desktop ? (
          <li className={styles.empty}>
            <p className={styles.noticeTitle}>{tm(blocked.what)}</p>
            <p>{tm(blocked.why)}</p>
          </li>
        ) : !loaded ? (
          <li className={styles.empty}>{t("ai.settings.loading")}</li>
        ) : !active ? (
          <li className={styles.empty}>
            <p>{t("ai.panel.notSignedIn", { provider: info.label })}</p>
            <Button size="sm" variant="primary" icon={<UserRound />} onClick={() => openSettings(true)}>
              {t("ai.panel.openAccounts")}
            </Button>
          </li>
        ) : !workspace ? (
          <li className={styles.empty}>{t("ai.panel.noWorkspace")}</li>
        ) : chat.items.length === 0 ? (
          <li className={styles.empty}>{t("ai.panel.empty")}</li>
        ) : (
          chat.items.map((item) => <Item key={item.id} item={item} />)
        )}
        {chat.running && chat.items.at(-1)?.kind === "user" && (
          <li className={styles.thinking} role="status">
            <Loader2 className={styles.spin} aria-hidden />
            {t("ai.panel.thinking")}
          </li>
        )}
      </ol>

      <div className={styles.composer}>
        <textarea
          className={`${inputs.input} ${styles.textarea}`}
          aria-label={t("ai.panel.placeholder")}
          placeholder={t("ai.panel.placeholder")}
          value={draft}
          rows={2}
          disabled={!desktop || !active || !workspace}
          onChange={(e) => setDraft(e.target.value)}
          onKeyDown={onKey}
        />
        <div className={styles.composerRow}>
          <span className={styles.hint}>{t("ai.panel.sendHint")}</span>
          {chat.running ? (
            <Button size="sm" variant="danger" icon={<Square />} onClick={() => void cancel()} data-testid="ai-stop">
              {t("ai.panel.stop")}
            </Button>
          ) : (
            <Button size="sm" variant="primary" icon={<Send />} disabled={!canSend} onClick={submit} data-testid="ai-send">
              {t("ai.panel.send")}
            </Button>
          )}
        </div>
      </div>
    </aside>
  );
}

function Item({ item }: { item: ChatItem }) {
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
        <li className={styles.assistant}>
          <span className="visually-hidden">{t("ai.panel.assistant")}: </span>
          <p className={styles.bubbleText}>{item.text}</p>
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

const STATUS: Record<Extract<ChatItem, { kind: "tool" }>["status"], { key: MessageKey; tone: "info" | "success" | "danger" | "neutral" }> = {
  running: { key: "ai.tool.running", tone: "info" },
  done: { key: "ai.tool.done", tone: "success" },
  error: { key: "ai.tool.error", tone: "danger" },
  skipped: { key: "ai.tool.skipped", tone: "neutral" },
};

function ToolCard({ item }: { item: Extract<ChatItem, { kind: "tool" }> }) {
  const { t } = useI18n();
  const args = keyArgs(item.args);
  const status = STATUS[item.status];
  return (
    <li className={styles.tool} data-status={item.status} data-testid="ai-tool-card" data-tool={item.name}>
      <div className={styles.toolHead}>
        <Wrench aria-hidden />
        <span className={`${styles.toolName} num`}>{item.name}</span>
        <Badge tone={writesFiles(item.name) ? "warning" : "neutral"}>{t(writesFiles(item.name) ? "ai.tool.writes" : "ai.tool.readOnly")}</Badge>
        <span className={styles.spacer} />
        <Badge tone={status.tone}>{t(status.key)}</Badge>
      </div>
      {args.length > 0 && (
        <dl className={styles.kv} aria-label={t("ai.tool.args")}>
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
      {item.result && (item.result.fields.length > 0 || item.result.text) && (
        <div className={styles.toolResult} aria-label={t("ai.tool.result")}>
          {item.result.fields.length > 0 && (
            <dl className={styles.kv}>
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
          {item.result.text && <pre className={styles.toolText}>{item.result.text}</pre>}
        </div>
      )}
    </li>
  );
}

function ApprovalCard({ item }: { item: Extract<ChatItem, { kind: "approval" }> }) {
  const { t } = useI18n();
  const args = keyArgs(item.args);
  const done: Record<Exclude<typeof item.state, "pending">, MessageKey> = { allowed: "ai.approval.allowed", denied: "ai.approval.denied", expired: "ai.approval.expired" };
  return (
    <li className={styles.approval} data-state={item.state} data-testid="ai-approval" role={item.state === "pending" ? "group" : undefined} aria-labelledby={`approval-${item.id}`}>
      <div className={styles.toolHead}>
        <ShieldAlert aria-hidden />
        <p id={`approval-${item.id}`} className={styles.noticeTitle}>
          {t("ai.approval.title", { tool: item.name })}
        </p>
      </div>
      <p className={styles.noticeBody}>{t("ai.approval.body")}</p>
      {args.length > 0 && (
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
      {item.state === "pending" ? (
        <div className={styles.row}>
          <Button size="sm" variant="primary" icon={<Check />} onClick={() => void answerApproval(item.approvalId, true)}>
            {t("ai.approval.allow")}
          </Button>
          <Button size="sm" icon={<X />} onClick={() => void answerApproval(item.approvalId, false)}>
            {t("ai.approval.deny")}
          </Button>
        </div>
      ) : (
        <Badge tone={item.state === "allowed" ? "success" : "neutral"}>{t(done[item.state])}</Badge>
      )}
    </li>
  );
}
