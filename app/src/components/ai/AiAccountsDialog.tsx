import { useEffect, useRef, useState } from "react";
import { isTauri } from "@tauri-apps/api/core";
import { Check, Copy, KeyRound, LockKeyhole, LogOut, MoreHorizontal, RefreshCw, UserRound, X } from "lucide-react";
import { useI18n } from "../../i18n";
import type { MessageKey } from "../../i18n/en";
import { accountKind, providerState, providerSummary, type DeviceCode, type ProviderSummary } from "../../ai/accounts";
import type { Account, Provider } from "../../ai/protocol";
import { providerIds, providerInfo } from "../../ai/providers";
import { dismissWelcome, openSettings, signIn, signOut, useAi } from "../../ai/store";
import { refusals } from "../../refusals";
import { Button } from "../ui/Button";
import { AiNotice, ManageUsageLink } from "./AiNotice";
import { displayLabel, OpenAiMark, ProviderMark } from "./BrandMarks";
import { clock, useNow } from "./time";
import dialog from "../Dialog.module.css";
import inputs from "../ui/inputs.module.css";
import styles from "./Ai.module.css";

/** Settings > AI accounts: one card per provider (sign-in, plan status, accounts), privacy line. */
export function AiAccountsDialog() {
  const { t, tm } = useI18n();
  const close = () => openSettings(false);
  const closeButton = useRef<HTMLButtonElement>(null);
  const loaded = useAi((s) => s.loaded);
  const welcome = useAi((s) => s.welcome);
  const reported = Object.keys(useAi((s) => s.accounts));
  useEffect(() => closeButton.current?.focus(), []);
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      // An open account menu takes this Escape for itself.
      if (e.key === "Escape" && !document.querySelector('[data-testid="ai-accounts"] [role="menu"]')) {
        e.stopPropagation();
        close();
      }
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, []);
  const desktop = isTauri();
  const blocked = refusals.aiDesktopOnly();

  return (
    <div className={dialog.scrim} onClick={close}>
      <div className={`${dialog.dialog} ${styles.accountsDialog}`} role="dialog" aria-modal="true" aria-labelledby="ai-accounts-title" aria-describedby="ai-accounts-intro" onClick={(e) => e.stopPropagation()} data-testid="ai-accounts">
        <header className={dialog.header}>
          <UserRound className={dialog.headerIcon} aria-hidden />
          <h2 id="ai-accounts-title" className={dialog.title}>
            {t("ai.settings.title")}
          </h2>
          <button ref={closeButton} type="button" className={dialog.close} onClick={close} aria-label={t("ai.close")} title={t("ai.close")}>
            <X aria-hidden />
          </button>
        </header>
        <p id="ai-accounts-intro" className={styles.intro}>
          {t("ai.settings.intro")}
        </p>
        <div className={styles.accountsBody}>
          {!desktop ? (
            <div className={styles.notice} data-tone="warning" role="status">
              <div className={styles.noticeText}>
                <p className={styles.noticeTitle}>{tm(blocked.what)}</p>
                <p className={styles.noticeBody}>{tm(blocked.why)}</p>
              </div>
            </div>
          ) : !loaded ? (
            <p className={styles.muted} role="status">
              {t("ai.settings.loading")}
            </p>
          ) : (
            <>
              {welcome && (
                <div className={styles.welcome} role="status" data-testid="ai-welcome">
                  <OpenAiMark className={styles.welcomeMark} />
                  <div className={styles.noticeText}>
                    <p className={styles.noticeTitle}>{t("ai.welcome.title")}</p>
                    <p className={styles.noticeBody}>{t("ai.welcome.body")}</p>
                    <div className={styles.row}>
                      <Button size="sm" variant="primary" onClick={dismissWelcome}>
                        {t("ai.welcome.ok")}
                      </Button>
                      <ManageUsageLink provider="chatgpt" />
                    </div>
                  </div>
                </div>
              )}
              <ul className={styles.providerGrid} aria-label={t("ai.settings.providers")}>
                {providerIds(reported).map((id) => (
                  <ProviderCard key={id} provider={id} />
                ))}
              </ul>
            </>
          )}
        </div>
        <p className={styles.privacy}>
          <LockKeyhole aria-hidden />
          {t("ai.settings.privacy")}
        </p>
      </div>
    </div>
  );
}

const CHIP: Record<ProviderSummary, { key: MessageKey; tone: "success" | "warning" | "info" | "danger" | "neutral" }> = {
  ready: { key: "ai.account.ready", tone: "success" },
  noPlan: { key: "ai.account.noPlan", tone: "warning" },
  apiKey: { key: "ai.account.apiKey", tone: "info" },
  signedOut: { key: "ai.account.signedOut", tone: "neutral" },
  expired: { key: "ai.status.expired", tone: "danger" },
  signingIn: { key: "ai.status.signingIn", tone: "info" },
  none: { key: "ai.status.none", tone: "neutral" },
};

/** A status chip: a dot plus the words, so the state never rests on colour alone. */
export function StatusChip({ tone, children, testId }: { tone: string; children: string; testId?: string }) {
  return (
    <span className={styles.chip} data-tone={tone} data-testid={testId}>
      <span className={styles.chipDot} aria-hidden />
      {children}
    </span>
  );
}

function ProviderCard({ provider }: { provider: Provider }) {
  const { t } = useI18n();
  const listed = providerInfo(provider);
  const info = { ...listed, label: displayLabel(provider, listed.label) };
  const [region, setRegion] = useState("mainland-cn");
  const state = useAi((s) => providerState(s.accounts, provider));
  const pending = useAi((s) => s.pending[provider] === true);
  const accounts = state.status?.accounts ?? [];
  const signingIn = state.notice?.kind === "signingIn";
  const summary = providerSummary(state);
  // One shared login (cli-login) gives way once it, or its API key, is active; an API-key-only
  // provider has no sign-in at all. OAuth and device-code providers can add more accounts.
  const shared = info.authKind === "cli-login" && accounts.some((a) => a.authenticated);
  const canSignIn = info.authKind !== "api-key" && info.signInSupported !== false && !shared;
  const chip = CHIP[summary];
  const chipLabel = summary === "ready" && info.readyLabel ? t(info.readyLabel) : t(chip.key);
  // Accounts of one provider differ only when there are several; a single one shares the card's chip.
  const many = accounts.length > 1;
  const device = state.notice?.kind === "signingIn" ? state.notice.device : undefined;
  // A provider the bridge can sign in to but not chat through (no verified tool calling).
  const signInOnly = info.toolCalling === false;
  // One line on what signing in gives: the listed note, else the bridge's (English) note or why chat is unavailable.
  const bridgeText = info.planNote ? undefined : signInOnly ? info.unsupportedReason : info.bridgePlanNote;
  const body = info.planNote
    ? t(info.planNote.body)
    : (bridgeText ?? (signInOnly ? t("ai.auth.signInOnly") : t(info.authKind === "api-key" ? "ai.auth.apiKeyOnly" : "ai.auth.generic", { provider: info.label, env: info.apiKeyEnv ?? "" })));

  return (
    <li className={styles.provider} aria-labelledby={`ai-${provider}-title`} data-provider={provider} data-auth={info.authKind} data-summary={summary}>
      <div className={styles.providerHead}>
        <span className={styles.providerMark} data-provider={provider}>
          <ProviderMark provider={provider} label={info.label} />
        </span>
        <div className={styles.providerName}>
          <h3 id={`ai-${provider}-title`} className={styles.providerTitle} title={info.label}>
            {info.label}
          </h3>
          <div className={styles.chips}>
            <StatusChip tone={chip.tone} testId="ai-status">
              {chipLabel}
            </StatusChip>
            {signInOnly && (
              <StatusChip tone="neutral" testId="ai-sign-in-only">
                {t("ai.status.signInOnly")}
              </StatusChip>
            )}
          </div>
        </div>
      </div>
      <div className={styles.providerText}>
        {info.planNote && <p className={styles.planNote}>{t(info.planNote.title)}</p>}
        <p className={styles.providerBody} lang={bridgeText ? "en" : undefined}>
          {body}
        </p>
      </div>
      {/* Fixed slots (head, text, actions, notices, footer) line up across the cards of one grid row. */}
      <div className={styles.cardActions}>
        {canSignIn && !device && info.regions && (
          <label className={styles.field}>
            {t("ai.auth.region")}
            <select className={`${inputs.input} ${inputs.select} ${styles.select}`} value={region} disabled={pending || signingIn} onChange={(e) => setRegion(e.target.value)} data-testid={`ai-region-${provider}`}>
              {info.regions.map((r) => (
                <option key={r} value={r}>
                  {t(r === "global" ? "ai.auth.regionGlobal" : "ai.auth.regionMainland")}
                </option>
              ))}
            </select>
          </label>
        )}
        {canSignIn && !device && (
          <button type="button" className={styles.brandButton} disabled={pending || signingIn} onClick={() => void signIn(provider, undefined, info.regions ? region : undefined)} data-testid={`ai-signin-${provider}`}>
            <ProviderMark provider={provider} label={info.label} className={styles.buttonMark} data-provider={provider} />
            {/* A brand-fixed label (e.g. "Continue with ChatGPT") is never translated. */}
            <span lang={info.signInLabel ? "en" : undefined}>{info.signInLabel ? t(info.signInLabel) : t("ai.auth.signIn", { provider: info.label })}</span>
          </button>
        )}
        {info.usageUrl && <ManageUsageLink provider={provider} />}
      </div>
      <div className={styles.cardRest}>
        {state.notice && !device && <AiNotice provider={provider} notice={state.notice} />}
        {device && <DeviceCodeBox provider={provider} code={device} onRetry={() => void signIn(provider, undefined, info.regions ? region : undefined)} />}
      </div>
      <div className={styles.cardFooter}>
        {accounts.length > 0 ? (
          <ul className={styles.accounts} aria-label={t("ai.settings.accounts")}>
            {accounts.map((a) => (
              <AccountRow key={a.id} account={a} busy={pending || signingIn} showState={many} />
            ))}
          </ul>
        ) : (
          <p className={styles.noAccount}>{t("ai.settings.none")}</p>
        )}
        {info.apiKeyEnv && info.authKind !== "api-key" && (
          <details className={styles.apiKey}>
            <summary>
              <KeyRound aria-hidden />
              {t("ai.apiKey.title")}
            </summary>
            <p>{t("ai.apiKey.body", { env: info.apiKeyEnv, provider: info.label })}</p>
          </details>
        )}
      </div>
    </li>
  );
}

/** Device authorization: the code to enter at the provider, a copy button and its lifetime. */
function DeviceCodeBox({ provider, code, onRetry }: { provider: Provider; code: DeviceCode; onRetry(): void }) {
  const { t } = useI18n();
  const label = displayLabel(provider, providerInfo(provider).label);
  const now = useNow(code.expiresAt !== null);
  const [copied, setCopied] = useState(false);
  const left = code.expiresAt === null ? null : Math.max(0, code.expiresAt - now);
  const expired = left === 0;
  const copy = () => {
    void navigator.clipboard?.writeText(code.userCode).then(() => setCopied(true));
  };
  useEffect(() => {
    if (!copied) return;
    const id = setTimeout(() => setCopied(false), 2000);
    return () => clearTimeout(id);
  }, [copied]);
  return (
    <div className={styles.device} data-expired={expired || undefined} data-testid="ai-device-code" role="group" aria-labelledby={`ai-${provider}-device`}>
      <div className={styles.deviceHead}>
        <p id={`ai-${provider}-device`} className={styles.noticeTitle}>
          {t("ai.device.title", { provider: label })}
        </p>
        {left !== null && !expired && <span className={`${styles.countdown} tnum`}>{t("ai.device.expires", { time: clock(left) })}</span>}
      </div>
      <div className={styles.deviceRow}>
        <output className={`${styles.deviceCode} num`} aria-label={t("ai.device.label")}>
          {code.userCode}
        </output>
        <Button size="sm" icon={copied ? <Check /> : <Copy />} onClick={copy} disabled={expired} aria-live="polite">
          {t(copied ? "ai.device.copied" : "ai.device.copy")}
        </Button>
      </div>
      {expired ? (
        <div className={styles.row}>
          <p className={styles.noticeBody}>{t("ai.device.expired")}</p>
          <Button size="sm" icon={<RefreshCw />} onClick={onRetry}>
            {t("ai.device.retry")}
          </Button>
        </div>
      ) : (
        <p className={styles.noticeBody}>{t("ai.device.body", { provider: label })}</p>
      )}
      {/* Display only, never a link: Rust opens the separately validated page. */}
      {code.verificationUri && (
        <p className={`${styles.noticeBody} num`} lang="en">
          {code.verificationUri}
        </p>
      )}
    </div>
  );
}

function AccountRow({ account: a, busy, showState }: { account: Account; busy: boolean; showState: boolean }) {
  const { t } = useI18n();
  const info = providerInfo(a.provider);
  const kind = accountKind(a);
  const chip = CHIP[kind];
  const hint = kind === "apiKey" && info.apiKeyEnv ? t("ai.account.apiKeyHint", { env: info.apiKeyEnv }) : kind === "noPlan" ? t("ai.account.noPlanHint") : null;
  // Per-account providers (oauth, device-code) keep signed-out registrations: offer to sign in again.
  const perAccount = info.authKind === "oauth" || info.authKind === "device-code";
  // An API-key account (e.g. Claude through ANTHROPIC_API_KEY) has nothing to reauthorize or sign out of.
  const reauthorize = perAccount && kind !== "ready" && kind !== "apiKey";
  const canSignOut = a.authenticated && kind !== "apiKey";
  return (
    <li className={styles.account} data-kind={kind} data-testid="ai-account">
      <span className={styles.avatar} aria-hidden>
        {(a.email ?? a.label).slice(0, 1).toUpperCase()}
      </span>
      <div className={styles.accountMain}>
        <span className={styles.accountLabel} title={a.label}>
          {a.label}
        </span>
        {showState && <StatusChip tone={chip.tone}>{kind === "ready" && info.readyLabel ? t(info.readyLabel) : t(chip.key)}</StatusChip>}
        {hint && <p className={styles.accountHint}>{hint}</p>}
      </div>
      {/* Reauthorize is the way forward for a blocked or signed-out account, so it stays in view. */}
      {reauthorize && (
        <Button size="sm" icon={<RefreshCw />} disabled={busy} title={t("ai.account.reauthorizeTitle", { account: a.label })} onClick={() => void signIn(a.provider, a.id)}>
          {t("ai.account.reauthorize")}
        </Button>
      )}
      {canSignOut && <AccountMenu label={a.label} disabled={busy} onSignOut={() => void signOut(a.provider, perAccount ? a.id : undefined)} />}
    </li>
  );
}

/** The account's secondary actions behind a "more" button (sign out). */
function AccountMenu({ label, disabled, onSignOut }: { label: string; disabled: boolean; onSignOut(): void }) {
  const { t } = useI18n();
  const [open, setOpen] = useState(false);
  const wrap = useRef<HTMLDivElement>(null);
  const trigger = useRef<HTMLButtonElement>(null);
  const first = useRef<HTMLButtonElement>(null);
  useEffect(() => {
    if (!open) return;
    first.current?.focus();
    const onDown = (e: PointerEvent) => {
      if (!wrap.current?.contains(e.target as Node)) setOpen(false);
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      setOpen(false);
      trigger.current?.focus();
    };
    window.addEventListener("pointerdown", onDown);
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("pointerdown", onDown);
      window.removeEventListener("keydown", onKey);
    };
  }, [open]);
  return (
    <div ref={wrap} className={styles.menuWrap}>
      <Button
        ref={trigger}
        variant="ghost"
        size="sm"
        iconOnly
        icon={<MoreHorizontal />}
        disabled={disabled}
        aria-haspopup="menu"
        aria-expanded={open}
        aria-label={t("ai.account.more", { account: label })}
        title={t("ai.account.more", { account: label })}
        onClick={() => setOpen(!open)}
      />
      {open && (
        <div className={styles.menu} role="menu">
          <button
            ref={first}
            type="button"
            role="menuitem"
            className={styles.menuItem}
            data-tone="danger"
            title={t("ai.account.signOutTitle", { account: label })}
            onClick={() => {
              setOpen(false);
              onSignOut();
            }}
          >
            <LogOut aria-hidden />
            {t("ai.account.signOut")}
          </button>
        </div>
      )}
    </div>
  );
}
