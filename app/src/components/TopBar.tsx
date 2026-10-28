import { useEffect, useRef, useState } from "react";
import { isTauri } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { Bot, ChevronDown, CircleHelp, Database, Download, FileText, FolderOpen, House, Monitor, Moon, Redo2, Save, Sun, Undo2, UserRound } from "lucide-react";
import { openPanel, openSettings, useAi } from "../ai/store";
import { useI18n } from "../i18n";
import { ACCEPT_ATTRIBUTE, FORMATS } from "../scene/formats";
import { importFiles } from "../scene/importFiles";
import { redo, undo } from "../state/history";
import { openPaths, PROJECT_EXTENSION, projectTitle, saveProject, showStart } from "../state/project";
import { useApp, type Lang, type ThemePref } from "../state/store";
import { useExport } from "../state/exportDialog";
import { useReportDialog } from "../state/reportDialog";
import { useWizard } from "../state/wizard";
import { exportTargetId } from "./CommandPalette";
import { Badge } from "./ui/Badge";
import { Button } from "./ui/Button";
import { SegmentedControl } from "./ui/SegmentedControl";
import { Logo } from "./Logo";
import styles from "./TopBar.module.css";

const MOD = navigator.platform.toLowerCase().includes("mac") ? "⌘" : "Ctrl";

/** Desktop: native dialog (project files open as the project). Browser preview: a file input. */
export async function pickAndOpen(title: string, filterName: string, allName: string, fileInput: HTMLInputElement | null) {
  if (!isTauri()) {
    fileInput?.click();
    return;
  }
  const picked = await open({
    multiple: true,
    title,
    filters: [
      { name: filterName, extensions: [PROJECT_EXTENSION, ...FORMATS.flatMap((f) => f.extensions.map((e) => e.slice(1)))] },
      { name: allName, extensions: ["*"] },
    ],
  });
  if (picked?.length) await openPaths(picked);
}

export function TopBar() {
  const { t, lang } = useI18n();
  const setLang = useApp((s) => s.setLang);
  const themePref = useApp((s) => s.themePref);
  const setThemePref = useApp((s) => s.setThemePref);
  const project = useApp((s) => s.project);
  const dirty = useApp((s) => s.dirty);
  const history = useApp((s) => s.history);
  const home = useApp((s) => s.home);
  const hasContent = useApp((s) => s.objects.length > 0);
  const wizardOpen = useWizard((s) => s.wizard !== null);
  // Export the selected array, else the newest one.
  const exportTarget = useApp(exportTargetId);
  const fileInput = useRef<HTMLInputElement>(null);
  const aiOpen = useAi((s) => s.panelOpen);
  const name = projectTitle(project, lang);

  return (
    <header className={styles.topbar}>
      <div className={styles.brand}>
        <Logo className={styles.logo} />
        <span className={styles.wordmark}>{t("app.name")}</span>
        <span className={styles.tagline}>{t("app.tagline")}</span>
      </div>
      <div className={styles.divider} />
      <div className={styles.project}>
        <span className={styles.projectLabel}>{t("topbar.projectLabel")}</span>
        <span className={styles.projectLine}>
          <span className={styles.projectName} title={project.path ?? name}>
            {name}
          </span>
          {dirty && <span className={styles.dirty} title={t("topbar.unsaved")} aria-label={t("topbar.unsaved")} />}
          {!project.path && hasContent && <Badge tone="warning">{t("topbar.notSaved")}</Badge>}
        </span>
      </div>
      <div className={styles.spacer} />
      <div className={styles.group}>
        <Button variant="ghost" iconOnly icon={<House />} data-tour="home" active={home} aria-label={t("topbar.home")} title={`${t("topbar.home")} (${MOD}+N)`} onClick={showStart} />
        <Button
          variant="ghost"
          iconOnly
          icon={<Undo2 />}
          disabled={!history.undo}
          aria-label={t("topbar.undo")}
          title={history.undo ? t("topbar.undoAction", { action: t(history.undo), keys: `${MOD}+Z` }) : t("topbar.nothingToUndo")}
          onClick={undo}
        />
        <Button
          variant="ghost"
          iconOnly
          icon={<Redo2 />}
          disabled={!history.redo}
          aria-label={t("topbar.redo")}
          title={history.redo ? t("topbar.redoAction", { action: t(history.redo), keys: `${MOD}+Shift+Z` }) : t("topbar.nothingToRedo")}
          onClick={redo}
        />
      </div>
      <div className={styles.divider} />
      {/* Prominent only while there is nothing to work on; with a scene or the import wizard, their own action leads. */}
      <Button
        variant={hasContent || wizardOpen ? "secondary" : "primary"}
        icon={<FolderOpen />}
        title={`${t("topbar.openTitle")} (${MOD}+O)`}
        onClick={() => void pickAndOpen(t("topbar.openTitle"), t("topbar.openFilter"), t("topbar.openAll"), fileInput.current)}
      >
        {t("topbar.open")}
      </Button>
      <Button icon={<Save />} title={`${t("topbar.saveTitle")} (${MOD}+S)`} disabled={!hasContent} onClick={() => void saveProject()}>
        {t("topbar.save")}
      </Button>
      <ExportMenu target={exportTarget} hasContent={hasContent} />
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
      <div className={styles.divider} />
      <Button icon={<Bot />} active={aiOpen} disabled={home} title={t("topbar.aiTitle")} onClick={() => openPanel(!aiOpen)} data-testid="ai-toggle">
        {t("topbar.ai")}
      </Button>
      <Button variant="ghost" iconOnly icon={<UserRound />} aria-label={t("topbar.aiAccounts")} title={t("topbar.aiAccounts")} onClick={() => openSettings(true)} data-testid="ai-accounts-open" />
      <div className={styles.divider} />
      <SegmentedControl<Lang>
        ariaLabel={t("topbar.language")}
        value={lang}
        onChange={setLang}
        options={[
          { value: "ko", label: "한국어", title: "한국어" },
          { value: "en", label: "English", title: "English" },
        ]}
      />
      <SegmentedControl<ThemePref>
        ariaLabel={t("topbar.theme")}
        value={themePref}
        onChange={setThemePref}
        options={[
          { value: "light", label: <span className="visually-hidden">{t("topbar.theme.light")}</span>, icon: <Sun />, title: t("topbar.theme.light") },
          { value: "dark", label: <span className="visually-hidden">{t("topbar.theme.dark")}</span>, icon: <Moon />, title: t("topbar.theme.dark") },
          { value: "system", label: <span className="visually-hidden">{t("topbar.theme.system")}</span>, icon: <Monitor />, title: t("topbar.theme.system") },
        ]}
      />
      <Button
        variant="ghost"
        iconOnly
        icon={<CircleHelp />}
        aria-label={t("topbar.help")}
        title={`${t("topbar.help")} (?)`}
        onClick={() => useApp.getState().setHelpOpen(true)}
      />
    </header>
  );
}

/** Export: data files of the selected array (export dialog) or the report (report dialog). */
function ExportMenu({ target, hasContent }: { target: string | null; hasContent: boolean }) {
  const { t } = useI18n();
  const [open, setOpen] = useState(false);
  const wrap = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!open) return;
    const onDown = (e: PointerEvent) => {
      if (!wrap.current?.contains(e.target as Node)) setOpen(false);
    };
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && setOpen(false);
    window.addEventListener("pointerdown", onDown);
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("pointerdown", onDown);
      window.removeEventListener("keydown", onKey);
    };
  }, [open]);
  const dataOk = !!target && isTauri();
  return (
    <div ref={wrap} className={styles.menuWrap} data-testid="topbar-export">
      <Button icon={<Download />} title={t("topbar.exportMenu")} aria-haspopup="menu" aria-expanded={open} disabled={!hasContent} onClick={() => setOpen(!open)}>
        {t("topbar.export")}
        <ChevronDown className={styles.menuChevron} aria-hidden />
      </Button>
      {open && (
        <div className={styles.menu} role="menu">
          <button
            type="button"
            role="menuitem"
            className={styles.menuItem}
            disabled={!dataOk}
            title={t("topbar.exportTitle")}
            onClick={() => {
              setOpen(false);
              if (target) useExport.getState().open(target);
            }}
          >
            <Database aria-hidden />
            <span className={styles.menuText}>
              <span>{t("topbar.export.data")}</span>
              <span className={styles.menuSub}>{t("topbar.export.dataSub")}</span>
            </span>
          </button>
          <button
            type="button"
            role="menuitem"
            className={styles.menuItem}
            onClick={() => {
              setOpen(false);
              useReportDialog.getState().show();
            }}
          >
            <FileText aria-hidden />
            <span className={styles.menuText}>
              <span>{t("topbar.export.report")}</span>
              <span className={styles.menuSub}>{t("topbar.export.reportSub")}</span>
            </span>
          </button>
        </div>
      )}
    </div>
  );
}
