import { useEffect, useRef } from "react";
import { isTauri } from "@tauri-apps/api/core";
import { KeyRound, LogOut, RefreshCw, UserRound, X } from "lucide-react";
import { useI18n } from "../../i18n";
import { accountKind, providerState } from "../../ai/accounts";
import type { Account, Provider } from "../../ai/protocol";
import { providerIds, providerInfo } from "../../ai/providers";
import { dismissWelcome, openSettings, signIn, signOut, useAi } from "../../ai/store";
import { refusals } from "../../refusals";
import { Badge } from "../ui/Badge";
import { Button } from "../ui/Button";
import { AiNotice, ManageUsageLink } from "./AiNotice";
import { OpenAiMark, ProviderMark } from "./BrandMarks";
import dialog from "../Dialog.module.css";
import styles from "./Ai.module.css";

/** Settings > AI accounts: provider sign-in, the account list with plan status, sign-out. */
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
      if (e.key === "Escape") {
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
      <div className={`${dialog.dialog} ${styles.accountsDialog}`} role="dialog" aria-modal="true" aria-labelledby="ai-accounts-title" onClick={(e) => e.stopPropagation()} data-testid="ai-accounts">
        <header className={dialog.header}>
          <UserRound className={dialog.headerIcon} aria-hidden />
          <h2 id="ai-accounts-title" className={dialog.title}>
            {t("ai.settings.title")}
          </h2>
          <button ref={closeButton} type="button" className={dialog.close} onClick={close} aria-label={t("ai.close")} title={t("ai.close")}>
            <X aria-hidden />
          </button>
        </header>
        <p className={styles.intro}>{t("ai.settings.intro")}</p>
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
            {providerIds(reported).map((id) => (
              <ProviderSection key={id} provider={id} />
            ))}
          </>
        )}
      </div>
    </div>
  );
}

function ProviderSection({ provider }: { provider: Provider }) {
  const { t } = useI18n();
  const info = providerInfo(provider);
  const state = useAi((s) => providerState(s.accounts, provider));
  const pending = useAi((s) => s.pending[provider] === true);
  const accounts = state.status?.accounts ?? [];
  const signingIn = state.notice?.kind === "signingIn";
  // One shared login (cli-login) gives way once it, or its API key, is active; an API-key-only
  // provider has no sign-in at all. OAuth and device-code providers can add more accounts.
  const shared = info.authKind === "cli-login" && accounts.some((a) => a.authenticated);
  const canSignIn = info.authKind !== "api-key" && !shared;

  return (
    <section className={styles.provider} aria-labelledby={`ai-${provider}-title`} data-provider={provider} data-auth={info.authKind}>
      <div className={styles.providerHead}>
        <span className={styles.providerMark} data-provider={provider}>
          <ProviderMark provider={provider} />
        </span>
        <div className={styles.noticeText}>
          <h3 id={`ai-${provider}-title`} className={styles.providerTitle}>
            {info.planNote ? t(info.planNote.title) : info.label}
          </h3>
          <p className={styles.providerBody}>
            {info.planNote ? t(info.planNote.body) : t(info.authKind === "api-key" ? "ai.auth.apiKeyOnly" : "ai.auth.generic", { provider: info.label, env: info.apiKeyEnv ?? "" })}
          </p>
        </div>
      </div>
      {(canSignIn || info.usageUrl) && (
        <div className={styles.row}>
          {canSignIn && (
            <button type="button" className={styles.brandButton} disabled={pending || signingIn} onClick={() => void signIn(provider)} data-testid={`ai-signin-${provider}`}>
              <ProviderMark provider={provider} className={styles.buttonMark} data-provider={provider} />
              {/* A brand-fixed label (e.g. "Continue with ChatGPT") is never translated. */}
              <span lang={info.signInLabel ? "en" : undefined}>{info.signInLabel ? t(info.signInLabel) : t("ai.auth.signIn", { provider: info.label })}</span>
            </button>
          )}
          {info.usageUrl && <ManageUsageLink provider={provider} />}
        </div>
      )}
      {state.notice && <AiNotice provider={provider} notice={state.notice} />}
      {accounts.length > 0 && (
        <ul className={styles.accounts} aria-label={t("ai.settings.accounts")}>
          {accounts.map((a) => (
            <AccountRow key={a.id} account={a} busy={pending || signingIn} />
          ))}
        </ul>
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
    </section>
  );
}

function AccountRow({ account: a, busy }: { account: Account; busy: boolean }) {
  const { t } = useI18n();
  const info = providerInfo(a.provider);
  const kind = accountKind(a);
  const badge =
    kind === "ready" ? (
      <Badge tone="success">{info.readyLabel ? t(info.readyLabel) : t("ai.account.ready")}</Badge>
    ) : kind === "noPlan" ? (
      <Badge tone="warning">{t("ai.account.noPlan")}</Badge>
    ) : kind === "apiKey" ? (
      <Badge tone="info">{t("ai.account.apiKey")}</Badge>
    ) : (
      <Badge>{t("ai.account.signedOut")}</Badge>
    );
  const hint = kind === "apiKey" && info.apiKeyEnv ? t("ai.account.apiKeyHint", { env: info.apiKeyEnv }) : kind === "noPlan" ? t("ai.account.noPlanHint") : null;
  // Per-account providers (oauth, device-code) keep signed-out registrations: offer to sign in again.
  const perAccount = info.authKind === "oauth" || info.authKind === "device-code";
  const reauthorize = perAccount && kind !== "ready";
  return (
    <li className={styles.account} data-kind={kind} data-testid="ai-account">
      <div className={styles.accountMain}>
        <span className={styles.accountLabel} title={a.label}>
          {a.label}
        </span>
        {badge}
      </div>
      {hint && <p className={styles.accountHint}>{hint}</p>}
      <div className={styles.accountActions}>
        {reauthorize && (
          <Button size="sm" icon={<RefreshCw />} disabled={busy} title={t("ai.account.reauthorizeTitle", { account: a.label })} onClick={() => void signIn(a.provider, a.id)}>
            {t("ai.account.reauthorize")}
          </Button>
        )}
        {a.authenticated && kind !== "apiKey" && (
          <Button size="sm" variant="ghost" icon={<LogOut />} disabled={busy} title={t("ai.account.signOutTitle", { account: a.label })} onClick={() => void signOut(a.provider, perAccount ? a.id : undefined)}>
            {t("ai.account.signOut")}
          </Button>
        )}
      </div>
    </li>
  );
}
