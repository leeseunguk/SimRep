import { useEffect, useMemo, useRef, useState, type KeyboardEvent as ReactKeyboardEvent } from "react";
import { isTauri } from "@tauri-apps/api/core";
import { Command } from "lucide-react";
import { useI18n } from "../i18n";
import type { MessageKey } from "../i18n/en";
import { saveProject, showStart } from "../state/project";
import { useExport } from "../state/exportDialog";
import { useReportDialog } from "../state/reportDialog";
import { useApp } from "../state/store";
import { pickAndOpen } from "./TopBar";
import dialog from "./Dialog.module.css";
import styles from "./CommandPalette.module.css";

interface Cmd {
  id: string;
  label: MessageKey;
  enabled: boolean;
  run(): void;
}

/** The array "export data" targets: the selected one, else the newest. */
export function exportTargetId(s: ReturnType<typeof useApp.getState>): string | null {
  const arrays = s.objects.filter((o) => o.kind === "pa");
  return (arrays.find((o) => o.id === s.selectedId) ?? arrays.at(-1))?.id ?? null;
}

/** Ctrl+K / Ctrl+Shift+P: type to filter, Enter runs, Escape closes. */
export function CommandPalette() {
  const { t } = useI18n();
  const close = () => useReportDialog.getState().setPalette(false);
  const target = useApp(exportTargetId);
  const hasContent = useApp((s) => s.objects.length > 0);
  const [query, setQuery] = useState("");
  const [active, setActive] = useState(0);
  const input = useRef<HTMLInputElement>(null);
  useEffect(() => input.current?.focus(), []);

  const commands: Cmd[] = useMemo(
    () => [
      { id: "exportReport", label: "palette.cmd.exportReport", enabled: hasContent, run: () => useReportDialog.getState().show() },
      { id: "exportData", label: "palette.cmd.exportData", enabled: !!target && isTauri(), run: () => target && useExport.getState().open(target) },
      {
        id: "open",
        label: "palette.cmd.open",
        enabled: true,
        run: () => void pickAndOpen(t("topbar.openTitle"), t("topbar.openFilter"), t("topbar.openAll"), null),
      },
      { id: "save", label: "palette.cmd.save", enabled: hasContent, run: () => void saveProject() },
      { id: "home", label: "palette.cmd.home", enabled: true, run: showStart },
      { id: "help", label: "palette.cmd.help", enabled: true, run: () => useApp.getState().setHelpOpen(true) },
    ],
    [hasContent, target, t],
  );
  const q = query.trim().toLowerCase();
  const shown = commands.filter((c) => !q || t(c.label).toLowerCase().includes(q) || c.id.toLowerCase().includes(q));
  const pick = Math.min(active, Math.max(0, shown.length - 1));

  const exec = (c: Cmd | undefined) => {
    if (!c?.enabled) return;
    close();
    c.run();
  };
  const onKey = (e: ReactKeyboardEvent) => {
    if (e.key === "Escape") {
      e.preventDefault();
      close();
    } else if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      const n = shown.length;
      if (n) setActive((pick + (e.key === "ArrowDown" ? 1 : n - 1)) % n);
    } else if (e.key === "Enter") {
      e.preventDefault();
      exec(shown[pick]);
    }
  };

  return (
    <div className={`${dialog.scrim} ${styles.scrim}`} onPointerDown={(e) => e.target === e.currentTarget && close()}>
      <div className={`${dialog.dialog} ${styles.palette}`} role="dialog" aria-modal="true" aria-label={t("palette.title")} onKeyDown={onKey}>
        <div className={styles.inputRow}>
          <Command className={styles.icon} aria-hidden />
          <input
            ref={input}
            className={styles.input}
            role="combobox"
            aria-expanded="true"
            aria-controls="palette-list"
            aria-activedescendant={shown[pick] ? `palette-${shown[pick].id}` : undefined}
            aria-label={t("palette.title")}
            placeholder={t("palette.placeholder")}
            value={query}
            onChange={(e) => {
              setQuery(e.target.value);
              setActive(0);
            }}
          />
        </div>
        <ul id="palette-list" className={styles.list} role="listbox" aria-label={t("palette.title")}>
          {shown.map((c, i) => (
            <li
              key={c.id}
              id={`palette-${c.id}`}
              role="option"
              aria-selected={i === pick}
              aria-disabled={!c.enabled}
              className={styles.item}
              onPointerMove={() => setActive(i)}
              onClick={() => exec(c)}
            >
              {t(c.label)}
              {!c.enabled && <span className={styles.off}>{t("palette.disabled")}</span>}
            </li>
          ))}
          {!shown.length && <li className={styles.empty}>{t("palette.empty")}</li>}
        </ul>
      </div>
    </div>
  );
}
