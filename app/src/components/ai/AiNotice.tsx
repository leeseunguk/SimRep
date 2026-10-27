import { CircleAlert, CircleCheck, ExternalLink, Info, Loader2, TriangleAlert, X } from "lucide-react";
import { useI18n } from "../../i18n";
import type { Notice } from "../../ai/accounts";
import type { Provider } from "../../ai/protocol";
import { refusals } from "../../refusals";
import { setNotice } from "../../ai/accounts";
import { useAi } from "../../ai/store";
import { providerInfo } from "../../ai/providers";
import styles from "./Ai.module.css";

/** The provider's usage settings (SIWC UI guidelines: "Manage usage"); nothing without a URL. */
export function ManageUsageLink({ provider }: { provider: Provider }) {
  const { t } = useI18n();
  const url = providerInfo(provider).usageUrl;
  if (!url) return null;
  return (
    <a className={styles.link} href={url} target="_blank" rel="noreferrer noopener" title={t("ai.chatgpt.manageUsageTitle")}>
      {t("ai.chatgpt.manageUsage")}
      <ExternalLink aria-hidden />
    </a>
  );
}

/** The state of the latest sign-in / sign-out of one provider, in place under its button. */
export function AiNotice({ provider, notice }: { provider: Provider; notice: Notice }) {
  const { t, tm } = useI18n();
  const dismiss = () => useAi.setState((s) => ({ accounts: setNotice(s.accounts, provider, null) }));
  let tone: "info" | "success" | "warning" | "danger";
  let title: string;
  let body: string;
  switch (notice.kind) {
    case "signingIn": {
      // What the user does next depends on how the provider signs in.
      const info = providerInfo(provider);
      tone = "info";
      if (info.authKind === "cli-login") {
        title = t("ai.notice.signingIn.cli", { cli: info.cli ?? info.label });
        body = t("ai.notice.signingIn.cliBody", { cli: info.cli ?? info.label });
      } else if (info.authKind === "device-code") {
        title = t("ai.notice.signingIn.device", { provider: info.label });
        body = t("ai.notice.signingIn.deviceBody", { provider: info.label });
      } else {
        title = t("ai.notice.signingIn.browser");
        body = t("ai.notice.signingIn.browserBody", { provider: info.label });
      }
      break;
    }
    case "success":
      tone = "success";
      title = t("ai.notice.success", { account: notice.account });
      body = t("ai.notice.successBody");
      break;
    case "notEligible":
      tone = "warning";
      title = t("ai.notice.notEligible");
      body = t("ai.notice.notEligibleBody");
      break;
    case "signedOut":
      tone = "info";
      title = t("ai.notice.signedOut");
      body = t(notice.remoteConfirmed ? "ai.notice.signedOutRevoked" : "ai.notice.signedOutLocal");
      break;
    case "failed": {
      const r = refusals.ai(notice.action, notice.code);
      tone = notice.code === "AUTH_DENIED" || notice.code === "CANCELLED" ? "warning" : "danger";
      title = tm(r.what);
      body = tm(r.why);
      break;
    }
  }
  const Icon = notice.kind === "signingIn" ? Loader2 : tone === "success" ? CircleCheck : tone === "warning" ? TriangleAlert : tone === "danger" ? CircleAlert : Info;
  return (
    <div className={styles.notice} data-tone={tone} role={tone === "danger" ? "alert" : "status"} data-notice={notice.kind === "failed" ? notice.code : notice.kind}>
      <Icon className={notice.kind === "signingIn" ? styles.spin : undefined} aria-hidden />
      <div className={styles.noticeText}>
        <p className={styles.noticeTitle}>{title}</p>
        <p className={styles.noticeBody}>{body}</p>
      </div>
      {notice.kind !== "signingIn" && (
        <button type="button" className={styles.iconButton} onClick={dismiss} aria-label={t("ai.notice.dismiss")} title={t("ai.notice.dismiss")}>
          <X aria-hidden />
        </button>
      )}
    </div>
  );
}
