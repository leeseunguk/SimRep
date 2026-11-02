import { useEffect, useRef, useState, type ReactNode } from "react";
import { isTauri } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { Bot, ChevronDown, CircleHelp, Database, Download, Ellipsis, FileText, FolderOpen, House, Monitor, Moon, PanelLeft, PanelRight, Redo2, Save, Sun, Undo2, UserRound } from "lucide-react";
import { openPanel, openSettings, useAi } from "../ai/store";
import { allows, host } from "../host";
import { useI18n } from "../i18n";
import { ACCEPT_ATTRIBUTE, FORMATS } from "../scene/formats";
import { importFiles } from "../scene/importFiles";
import { redo, undo } from "../state/history";
import { openPaths, PROJECT_EXTENSION, projectTitle, saveProject, showStart } from "../state/project";
import { useApp, type Lang, type ThemePref } from "../state/store";
import { useLayout } from "../state/layout";
import { drawerToggleId, ownEscape } from "../state/escape";
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
  const mode = useLayout((s) => s.mode);
  const drawer = useLayout((s) => s.drawer);
  const openFile = () => void pickAndOpen(t("topbar.openTitle"), t("topbar.openFilter"), t("topbar.openAll"), fileInput.current);
  const fileField = (
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
  );
  const langControl = (
    <SegmentedControl<Lang>
      ariaLabel={t("topbar.language")}
      className={styles.lang}
      value={lang}
      onChange={setLang}
      options={[
        { value: "ko", label: "한국어", title: "한국어" },
        { value: "en", label: "English", title: "English" },
      ]}
    />
  );
  const themeControl = (
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
  );

  // Tablet and phone: a condensed bar; the rest of the actions live in the More menu.
  if (mode !== "desktop") {
    const tablet = mode === "tablet";
    return (
      <header className={styles.topbar} data-compact={mode}>
        <div className={styles.brand}>
          <Logo className={styles.logo} />
          {tablet && <span className={styles.wordmark}>{t("app.name")}</span>}
        </div>
        {tablet && (
          <>
            {/* Disclosure buttons of the drawers (App: #panel-scene, #panel-simulation); Escape closes them. */}
            <Button variant="ghost" iconOnly icon={<PanelLeft />} active={drawer === "scene"} aria-pressed={undefined} aria-expanded={drawer === "scene"} aria-controls="panel-scene" id={drawerToggleId("scene")} aria-label={t("layout.scenePanel")} title={t("layout.scenePanel")} onClick={() => useLayout.getState().toggleDrawer("scene")} data-testid="drawer-scene" />
            <Button variant="ghost" iconOnly icon={<PanelRight />} active={drawer === "simulation"} aria-pressed={undefined} aria-expanded={drawer === "simulation"} aria-controls="panel-simulation" id={drawerToggleId("simulation")} aria-label={t("layout.propsPanel")} title={t("layout.propsPanel")} onClick={() => useLayout.getState().toggleDrawer("simulation")} data-testid="drawer-simulation" />
          </>
        )}
        <div className={styles.project}>
          <span className={styles.projectLine}>
            <span className={styles.projectName} title={project.path ?? name}>
              {name}
            </span>
            {dirty && <span className={styles.dirty} title={t("topbar.unsaved")} aria-label={t("topbar.unsaved")} />}
          </span>
        </div>
        <div className={styles.spacer} />
        <Button variant="ghost" iconOnly icon={<House />} data-tour="home" active={home} aria-label={t("topbar.home")} title={t("topbar.home")} onClick={showStart} />
        {tablet && (
          <>
            <Button variant="ghost" iconOnly icon={<Undo2 />} disabled={!history.undo} aria-label={t("topbar.undo")} title={t("topbar.undo")} onClick={undo} />
            <Button variant="ghost" iconOnly icon={<Redo2 />} disabled={!history.redo} aria-label={t("topbar.redo")} title={t("topbar.redo")} onClick={redo} />
            <Button variant={hasContent || wizardOpen ? "secondary" : "primary"} icon={<FolderOpen />} title={t("topbar.openTitle")} onClick={openFile}>
              {t("topbar.open")}
            </Button>
          </>
        )}
        <MoreMenu
          phone={!tablet}
          hasContent={hasContent}
          exportTarget={exportTarget}
          onOpen={openFile}
          lang={langControl}
          theme={themeControl}
        />
        {fileField}
      </header>
    );
  }

  return (
    <header className={styles.topbar}>
      <div className={styles.brand}>
        <Logo className={styles.logo} />
        <span className={styles.wordmark}>{t("app.name")}</span>
        <span className={styles.tagline} data-testid="topbar-tagline">{t("app.tagline")}</span>
      </div>
      <div className={styles.divider} />
      <div className={styles.project}>
        <span className={styles.projectLabel}>{t("topbar.projectLabel")}</span>
        <span className={styles.projectLine}>
          <span className={styles.projectName} title={project.path ?? name} data-testid="project-name">
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
        <span className={styles.label}>{t("topbar.open")}</span>
      </Button>
      <Button icon={<Save />} title={`${t("topbar.saveTitle")} (${MOD}+S)`} disabled={!hasContent} onClick={() => void saveProject()}>
        <span className={styles.label}>{t("topbar.save")}</span>
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
        <span className={styles.label}>{t("topbar.ai")}</span>
      </Button>
      <Button variant="ghost" iconOnly icon={<UserRound />} aria-label={t("topbar.aiAccounts")} title={t("topbar.aiAccounts")} onClick={() => openSettings(true)} data-testid="ai-accounts-open" />
      <div className={styles.divider} />
      <SegmentedControl<Lang>
        ariaLabel={t("topbar.language")}
        className={styles.lang}
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
  const dataOk = !!target && isTauri() && allows("exportData");
  const Extra = host()?.ExportMenuItems;
  return (
    <div ref={wrap} className={styles.menuWrap} data-testid="topbar-export">
      <Button icon={<Download />} title={t("topbar.exportMenu")} aria-haspopup="menu" aria-expanded={open} disabled={!hasContent} onClick={() => setOpen(!open)}>
        <span className={styles.label}>{t("topbar.export")}</span>
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
          {Extra && <Extra close={() => setOpen(false)} />}
        </div>
      )}
    </div>
  );
}

/** Tablet / phone: the top bar actions that do not fit (open on a phone, save, export, assistant, language, theme, help). */
function MoreMenu({
  phone,
  hasContent,
  exportTarget,
  onOpen,
  lang,
  theme,
}: {
  phone: boolean;
  hasContent: boolean;
  exportTarget: string | null;
  onOpen(): void;
  lang: ReactNode;
  theme: ReactNode;
}) {
  const { t } = useI18n();
  const [open, setOpen] = useState(false);
  const wrap = useRef<HTMLDivElement>(null);
  const trigger = useRef<HTMLButtonElement>(null);
  const history = useApp((s) => s.history);
  const home = useApp((s) => s.home);
  const aiOpen = useAi((s) => s.panelOpen);
  useEffect(() => {
    if (!open) return;
    const onDown = (e: PointerEvent) => {
      if (!wrap.current?.contains(e.target as Node)) setOpen(false);
    };
    // Capture phase: this Escape closes the menu (focus back on its button) and nothing else, so the
    // workspace Escape (App) does not also clear the selection.
    const onKey = (e: KeyboardEvent) =>
      ownEscape(e, () => {
        setOpen(false);
        trigger.current?.focus();
      });
    window.addEventListener("pointerdown", onDown);
    window.addEventListener("keydown", onKey, true);
    return () => {
      window.removeEventListener("pointerdown", onDown);
      window.removeEventListener("keydown", onKey, true);
    };
  }, [open]);
  const dataOk = !!exportTarget && isTauri() && allows("exportData");
  const Extra = host()?.ExportMenuItems;
  /** A menu action: closes the menu, then runs. */
  const item = (icon: ReactNode, label: string, run: () => void, opts: { disabled?: boolean; testId?: string; pressed?: boolean } = {}) => (
    <button
      type="button"
      className={styles.menuItem}
      disabled={opts.disabled}
      aria-pressed={opts.pressed}
      data-testid={opts.testId}
      onClick={() => {
        setOpen(false);
        run();
      }}
    >
      {icon}
      <span className={styles.menuText}>{label}</span>
    </button>
  );
  return (
    <div ref={wrap} className={styles.menuWrap}>
      <Button ref={trigger} variant="ghost" iconOnly icon={<Ellipsis />} aria-label={t("topbar.more")} title={t("topbar.more")} aria-expanded={open} onClick={() => setOpen(!open)} data-testid="topbar-more" />
      {open && (
        <div className={`${styles.menu} ${styles.moreMenu}`} aria-label={t("topbar.more")} data-testid="topbar-more-menu">
          {phone && (
            <>
              {item(<FolderOpen aria-hidden />, t("topbar.open"), onOpen)}
              {item(<Undo2 aria-hidden />, t("topbar.undo"), undo, { disabled: !history.undo })}
              {item(<Redo2 aria-hidden />, t("topbar.redo"), redo, { disabled: !history.redo })}
            </>
          )}
          {item(<Save aria-hidden />, t("topbar.save"), () => void saveProject(), { disabled: !hasContent })}
          {item(<Database aria-hidden />, t("topbar.export.data"), () => exportTarget && useExport.getState().open(exportTarget), { disabled: !hasContent || !dataOk })}
          {item(<FileText aria-hidden />, t("topbar.export.report"), () => useReportDialog.getState().show(), { disabled: !hasContent })}
          {Extra && <Extra close={() => setOpen(false)} />}
          {item(<Bot aria-hidden />, t("topbar.ai"), () => openPanel(!aiOpen), { disabled: home, pressed: aiOpen, testId: "ai-toggle" })}
          {item(<UserRound aria-hidden />, t("topbar.aiAccounts"), () => openSettings(true), { testId: "ai-accounts-open" })}
          {item(<CircleHelp aria-hidden />, t("topbar.help"), () => useApp.getState().setHelpOpen(true))}
          <div className={styles.menuControls}>
            {lang}
            {theme}
          </div>
        </div>
      )}
    </div>
  );
}
