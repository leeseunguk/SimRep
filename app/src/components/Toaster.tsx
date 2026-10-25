import { useEffect } from "react";
import { CircleAlert, CircleCheck, Info, TriangleAlert, X } from "lucide-react";
import { useI18n } from "../i18n";
import { useApp, type Toast, type ToastTone } from "../state/store";
import styles from "./Toaster.module.css";

const ICONS: Record<ToastTone, typeof Info> = {
  success: CircleCheck,
  info: Info,
  warning: TriangleAlert,
  error: CircleAlert,
};

const DURATION_MS: Record<ToastTone, number> = { success: 4500, info: 6000, warning: 7000, error: 9000 };

export function Toaster() {
  const toasts = useApp((s) => s.toasts);
  return (
    <div className={styles.toaster} aria-live="polite">
      {toasts.map((toast) => (
        <ToastCard key={toast.id} toast={toast} />
      ))}
    </div>
  );
}

function ToastCard({ toast }: { toast: Toast }) {
  const { t, tm } = useI18n();
  const dismiss = useApp((s) => s.dismissToast);
  const Icon = ICONS[toast.tone];

  useEffect(() => {
    const timer = window.setTimeout(() => dismiss(toast.id), DURATION_MS[toast.tone]);
    return () => window.clearTimeout(timer);
  }, [toast, dismiss]);

  return (
    <div className={styles.toast} data-tone={toast.tone} role={toast.tone === "error" ? "alert" : "status"}>
      <Icon className={styles.icon} aria-hidden />
      <div className={styles.text}>
        <p className={styles.title}>{tm(toast.title)}</p>
        {toast.body && <p className={styles.body}>{tm(toast.body)}</p>}
      </div>
      <button type="button" className={styles.close} aria-label={t("toast.dismiss")} title={t("toast.dismiss")} onClick={() => dismiss(toast.id)}>
        <X />
      </button>
    </div>
  );
}
