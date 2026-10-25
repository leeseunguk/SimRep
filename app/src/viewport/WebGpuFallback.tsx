import { MonitorX, RotateCw } from "lucide-react";
import { useI18n } from "../i18n";
import { Button } from "../components/ui/Button";
import styles from "./WebGpuFallback.module.css";

export function WebGpuFallback({ lost, detail }: { lost: boolean; detail: string }) {
  const { t } = useI18n();
  return (
    <div className={styles.wrap}>
      <div className={styles.card} role="alert">
        <MonitorX className={styles.icon} aria-hidden />
        <h2 className={styles.title}>{t("viewport.webgpu.title")}</h2>
        <p className={styles.body}>{lost ? t("viewport.webgpu.lost", { reason: detail }) : t("viewport.webgpu.body")}</p>
        {!lost && (
          <ul className={styles.fixes}>
            <li>{t("viewport.webgpu.fixWindows")}</li>
            <li>{t("viewport.webgpu.fixBrowser")}</li>
          </ul>
        )}
        <details className={styles.details}>
          <summary>{t("viewport.webgpu.details")}</summary>
          <code>{detail}</code>
        </details>
        <Button variant="secondary" size="sm" icon={<RotateCw />} onClick={() => location.reload()}>
          {t("viewport.webgpu.reload")}
        </Button>
      </div>
    </div>
  );
}
