import { useRef, type CSSProperties } from "react";
import { isTauri } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { ArrowLeft, Clock, FileStack, FolderOpen, GraduationCap, History, LifeBuoy, Trash2, X } from "lucide-react";
import { allows, host, unavailable } from "../host";
import { useI18n } from "../i18n";
import { RefusalNotice } from "./SimulationSections";
import { ACCEPT_ATTRIBUTE, FORMATS } from "../scene/formats";
import { importFiles } from "../scene/importFiles";
import { TEMPLATES, type TemplateDef } from "../scene/templates";
import {
  discardRecovery,
  dirname,
  forgetRecent,
  newFromFiles,
  newFromTemplate,
  openProject,
  projectTitle,
  restoreRecovery,
} from "../state/project";
import { useApp } from "../state/store";
import { useTour } from "../state/tour";
import { TourOffer } from "./Tour";
import { Button } from "./ui/Button";
import { Kbd } from "./ui/Kbd";
import { TemplateArt } from "./TemplateArt";
import styles from "./StartScreen.module.css";

/** Start screen: template gallery, open files / project, recent projects, autosave recovery. */
export function StartScreen() {
  const { t, lang } = useI18n();
  const recent = useApp((s) => s.recent);
  const recovery = useApp((s) => s.recovery);
  const hasScene = useApp((s) => s.objects.length > 0);
  const desktop = isTauri();
  const tourOffer = useTour((s) => s.mode === "offer");
  const fileInput = useRef<HTMLInputElement>(null);
  const templatesOff = unavailable("templates");
  const HostPanel = host()?.StartPanel;
  const when = (ms: number) =>
    new Intl.DateTimeFormat(lang === "ko" ? "ko-KR" : "en-US", { dateStyle: "medium", timeStyle: "short" }).format(new Date(ms));

  const openFiles = async () => {
    if (!desktop) {
      fileInput.current?.click();
      return;
    }
    const picked = await open({
      multiple: true,
      title: t("start.openFilesTitle"),
      filters: [{ name: t("topbar.openFilter"), extensions: FORMATS.flatMap((f) => f.extensions.map((e) => e.slice(1))) }],
    });
    if (picked?.length) await newFromFiles(picked);
  };

  return (
    <div className={styles.start} role="dialog" aria-modal="false" aria-label={t("start.title")}>
      <div className={styles.inner}>
        <header className={styles.head}>
          <div>
            <p className={styles.kicker}>{t("start.kicker")}</p>
            <h1 className={styles.title}>{t("start.title")}</h1>
            <p className={styles.lead}>{t("start.lead")}</p>
          </div>
          {hasScene && (
            <Button icon={<ArrowLeft />} onClick={() => useApp.getState().setHome(false)} title={t("start.backTitle")}>
              {t("start.back")} <Kbd>Esc</Kbd>
            </Button>
          )}
        </header>

        <div className={styles.columns}>
          <section className={styles.gallery} aria-label={t("start.templates")}>
            <h2 className={styles.sectionTitle}>{t("start.templates")}</h2>
            <div className={styles.grid}>
              {TEMPLATES.map((def, i) => (
                <TemplateCard key={def.id} def={def} index={i} disabled={!desktop || templatesOff !== null} />
              ))}
            </div>
            {!desktop && <p className={styles.note}>{t("template.desktopOnly")}</p>}
            {desktop && templatesOff && (
              <div className={styles.note}>
                <RefusalNotice refusal={templatesOff} />
              </div>
            )}
          </section>

          <aside className={styles.side}>
            {HostPanel && <HostPanel />}
            {tourOffer && <TourOffer inline />}
            {recovery && (
              <div className={styles.recovery} role="status">
                <LifeBuoy className={styles.recoveryIcon} aria-hidden />
                <div className={styles.recoveryText}>
                  <p className={styles.recoveryTitle}>{t("start.recovery.title")}</p>
                  <p className={styles.recoveryBody}>{t("start.recovery.body", { name: projectTitle(recovery.meta, lang), time: when(recovery.savedAt) })}</p>
                  <div className={styles.row}>
                    <Button size="sm" variant="primary" icon={<History />} onClick={() => void restoreRecovery()}>
                      {t("start.recovery.restore")}
                    </Button>
                    <Button size="sm" variant="ghost" icon={<Trash2 />} onClick={() => void discardRecovery()}>
                      {t("start.recovery.discard")}
                    </Button>
                  </div>
                </div>
              </div>
            )}

            <h2 className={styles.sectionTitle}>{t("start.open")}</h2>
            <div className={styles.openButtons}>
              <Button icon={<FolderOpen />} className={styles.wide} onClick={() => void openFiles()}>
                {t("start.openFiles")}
              </Button>
              <Button icon={<FileStack />} className={styles.wide} onClick={() => void openProject()} disabled={!desktop}>
                {t("start.openProject")}
              </Button>
              <input
                ref={fileInput}
                type="file"
                multiple
                accept={ACCEPT_ATTRIBUTE}
                hidden
                onChange={(e) => {
                  if (e.target.files) {
                    useApp.getState().setHome(false);
                    void importFiles(Array.from(e.target.files));
                  }
                  e.target.value = "";
                }}
              />
            </div>
            <p className={styles.dropHint}>{t("start.dropHint")}</p>
            {desktop && allows("tour") && (
              <Button variant="ghost" icon={<GraduationCap />} className={styles.wide} title={t("tour.openTitle")} onClick={() => useTour.getState().start()}>
                {t("tour.open")}
              </Button>
            )}

            <h2 className={styles.sectionTitle}>{t("start.recent")}</h2>
            {recent.length === 0 ? (
              <p className={styles.empty}>{t("start.recentEmpty")}</p>
            ) : (
              <ul className={styles.recent}>
                {recent.map((r) => (
                  <li key={r.path} className={styles.recentItem}>
                    <button type="button" className={styles.recentOpen} onClick={() => void openProject(r.path)} title={r.path}>
                      <span className={styles.recentName}>{r.name}</span>
                      <span className={styles.recentPath}>{dirname(r.path)}</span>
                      <span className={styles.recentTime}>
                        <Clock aria-hidden /> {when(r.savedAt)}
                      </span>
                    </button>
                    <button
                      type="button"
                      className={styles.recentForget}
                      aria-label={t("start.recentForget", { name: r.name })}
                      title={t("start.recentForget", { name: r.name })}
                      onClick={() => forgetRecent(r.path)}
                    >
                      <X aria-hidden />
                    </button>
                  </li>
                ))}
              </ul>
            )}
          </aside>
        </div>
      </div>
    </div>
  );
}

function TemplateCard({ def, index, disabled }: { def: TemplateDef; index: number; disabled: boolean }) {
  const { t } = useI18n();
  return (
    <button
      type="button"
      className={styles.card}
      data-tour={`template-${def.id}`}
      style={{ "--i": index } as CSSProperties}
      disabled={disabled}
      onClick={() => void newFromTemplate(def.id)}
    >
      <span className={styles.art}>
        <TemplateArt id={def.id} />
      </span>
      <span className={styles.cardBody}>
        <span className={styles.cardTitle}>{t(def.titleKey)}</span>
        <span className={styles.cardDesc}>{t(def.descKey)}</span>
        <span className={styles.cardMeta}>{t(`template.${def.id}.meta`)}</span>
      </span>
    </button>
  );
}
