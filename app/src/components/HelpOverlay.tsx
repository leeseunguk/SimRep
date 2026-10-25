import { useEffect, useRef } from "react";
import { isTauri } from "@tauri-apps/api/core";
import { GraduationCap, Keyboard, X } from "lucide-react";
import { useI18n } from "../i18n";
import type { MessageKey } from "../i18n/en";
import { useApp } from "../state/store";
import { useTour } from "../state/tour";
import { Button } from "./ui/Button";
import { Kbd } from "./ui/Kbd";
import styles from "./Dialog.module.css";

const MOD = navigator.platform.toLowerCase().includes("mac") ? "⌘" : "Ctrl";

/** Shortcut groups: each entry is a list of alternative key combos + what it does. */
const GROUPS: { title: MessageKey; items: { keys: string[][]; label: MessageKey }[] }[] = [
  {
    title: "help.group.file",
    items: [
      { keys: [[MOD, "N"]], label: "help.new" },
      { keys: [[MOD, "O"]], label: "help.open" },
      { keys: [[MOD, "S"]], label: "help.save" },
      { keys: [[MOD, "Shift", "S"]], label: "help.saveAs" },
    ],
  },
  {
    title: "help.group.edit",
    items: [
      { keys: [[MOD, "Z"]], label: "help.undo" },
      { keys: [[MOD, "Shift", "Z"], [MOD, "Y"]], label: "help.redo" },
      { keys: [["F2"]], label: "help.rename" },
      { keys: [["Delete"]], label: "help.delete" },
      { keys: [["Esc"]], label: "help.deselect" },
      { keys: [["↑"], ["↓"]], label: "help.treeNav" },
    ],
  },
  {
    title: "help.group.view",
    items: [
      { keys: [["1"]], label: "help.front" },
      { keys: [["2"]], label: "help.side" },
      { keys: [["3"]], label: "help.top" },
      { keys: [["4"], ["0"]], label: "help.iso" },
      { keys: [["F"]], label: "help.fit" },
      { keys: [["P"]], label: "help.projection" },
      { keys: [["G"]], label: "help.grid" },
    ],
  },
  {
    title: "help.group.mouse",
    items: [
      { keys: [["help.key.leftDrag"]], label: "help.orbit" },
      { keys: [["help.key.rightDrag"], ["Shift", "help.key.leftDrag"]], label: "help.pan" },
      { keys: [["help.key.wheel"]], label: "help.zoom" },
      { keys: [["help.key.doubleClick"]], label: "help.focus" },
      { keys: [["?"], ["F1"]], label: "help.toggle" },
    ],
  },
];

export function HelpOverlay() {
  const { t } = useI18n();
  const close = () => useApp.getState().setHelpOpen(false);
  const button = useRef<HTMLButtonElement>(null);
  useEffect(() => button.current?.focus(), []);
  const key = (k: string) => (k.startsWith("help.key.") ? t(k as MessageKey) : k);

  return (
    <div className={styles.scrim} onClick={close}>
      <div className={styles.dialog} role="dialog" aria-modal="true" aria-labelledby="help-title" onClick={(e) => e.stopPropagation()}>
        <header className={styles.header}>
          <Keyboard className={styles.headerIcon} aria-hidden />
          <h2 id="help-title" className={styles.title}>
            {t("help.title")}
          </h2>
          <button ref={button} type="button" className={styles.close} onClick={close} aria-label={t("help.close")} title={t("help.close")}>
            <X aria-hidden />
          </button>
        </header>
        <div className={styles.columns}>
          {GROUPS.map((g) => (
            <section key={g.title} className={styles.group}>
              <h3 className={styles.groupTitle}>{t(g.title)}</h3>
              <dl className={styles.list}>
                {g.items.map((item) => (
                  <div key={item.label} className={styles.item}>
                    <dt className={styles.keys}>
                      {item.keys.map((combo, i) => (
                        <span key={i} className={styles.combo}>
                          {i > 0 && <span className={styles.or}>{t("help.or")}</span>}
                          {combo.map((k, j) => (
                            <span key={j} className={styles.combo}>
                              {j > 0 && <span className={styles.plus}>+</span>}
                              <Kbd word={k.startsWith("help.key.")}>{key(k)}</Kbd>
                            </span>
                          ))}
                        </span>
                      ))}
                    </dt>
                    <dd className={styles.label}>{t(item.label)}</dd>
                  </div>
                ))}
              </dl>
            </section>
          ))}
        </div>
        <div className={styles.footer}>
          <p className={styles.footerText}>{t("help.footer")}</p>
          {isTauri() && (
            <Button size="sm" variant="primary" icon={<GraduationCap />} title={t("tour.openTitle")} onClick={() => useTour.getState().start()}>
              {t("tour.open")}
            </Button>
          )}
        </div>
      </div>
    </div>
  );
}

/** Blocking progress card while a template is generated or a project opens. */
export function BusyOverlay() {
  const { tm } = useI18n();
  const busy = useApp((s) => s.busy);
  if (!busy) return null;
  return (
    <div className={styles.scrim} data-busy>
      <div className={styles.busy} role="status" aria-live="polite">
        <span className={styles.spinner} aria-hidden />
        <p className={styles.busyText}>{tm(busy)}</p>
      </div>
    </div>
  );
}
