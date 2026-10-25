import { useEffect, useRef } from "react";
import { CircleAlert, CircleCheck, Info, Trash2, TriangleAlert, X } from "lucide-react";
import { useI18n } from "../i18n";
import { useApp, type LogLevel } from "../state/store";
import { Button } from "./ui/Button";
import styles from "./LogDrawer.module.css";

const ICONS: Record<LogLevel, typeof Info> = {
  info: Info,
  success: CircleCheck,
  warning: TriangleAlert,
  error: CircleAlert,
};

export function LogDrawer() {
  const { t, tm, lang } = useI18n();
  const logs = useApp((s) => s.logs);
  const { clearLogs, setLogOpen } = useApp.getState();
  const listRef = useRef<HTMLOListElement>(null);
  const time = new Intl.DateTimeFormat(lang === "ko" ? "ko-KR" : "en-GB", {
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
    hour12: false,
  });

  useEffect(() => {
    const el = listRef.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [logs.length]);

  return (
    <section className={styles.drawer} aria-label={t("log.title")}>
      <header className={styles.header}>
        <h2 className={styles.title}>{t("log.title")}</h2>
        <Button size="sm" variant="danger" icon={<Trash2 />} onClick={clearLogs} disabled={logs.length === 0}>
          {t("log.clear")}
        </Button>
        <Button size="sm" variant="ghost" iconOnly icon={<X />} aria-label={t("log.close")} title={t("log.close")} onClick={() => setLogOpen(false)} />
      </header>
      <ol ref={listRef} className={styles.list}>
        {logs.length === 0 && <li className={styles.empty}>{t("log.empty")}</li>}
        {logs.map((l) => {
          const Icon = ICONS[l.level];
          return (
            <li key={l.id} className={styles.entry} data-level={l.level}>
              <time className={styles.time}>{time.format(l.time)}</time>
              <Icon className={styles.icon} aria-hidden />
              <span className={styles.message}>{tm(l.message)}</span>
            </li>
          );
        })}
      </ol>
    </section>
  );
}
