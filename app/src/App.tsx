import { lazy, Suspense, useEffect, useLayoutEffect, useState, type CSSProperties, type ReactNode } from "react";
import { isTauri } from "@tauri-apps/api/core";
import { DropOverlay } from "./components/DropOverlay";
import { CommandPalette } from "./components/CommandPalette";
import { ExportDialog } from "./components/ExportDialog";
import { ReportDialog } from "./components/ReportDialog";
import { ImportWizard } from "./components/ImportWizard";
import { BusyOverlay, HelpOverlay } from "./components/HelpOverlay";
import { LogDrawer } from "./components/LogDrawer";
import { PanelResizer } from "./components/PanelResizer";
import { PropertiesPanel } from "./components/PropertiesPanel";
import { SceneTree } from "./components/SceneTree";
import { StartScreen } from "./components/StartScreen";
import { StatusBar } from "./components/StatusBar";
import { Tour } from "./components/Tour";
import { Toaster } from "./components/Toaster";
import { pickAndOpen, TopBar } from "./components/TopBar";
import { translate, useI18n } from "./i18n";
import { allows } from "./host";
import { connectRuntime } from "./runtime";
import { objectName } from "./scene/names";
import { redo, undo } from "./state/history";
import { initProjects, saveProject, showStart } from "./state/project";
import { useApp } from "./state/store";
import { useResolvedTheme } from "./state/theme";
import { Viewport } from "./viewport/Viewport";
import { useResults } from "./results/store";
import { useWizard } from "./state/wizard";
import { useExport } from "./state/exportDialog";
import { useReportDialog } from "./state/reportDialog";
import { initTour } from "./state/tour";
import styles from "./App.module.css";
import { AiAccountsDialog } from "./components/ai/AiAccountsDialog";
import { AiPanel } from "./components/ai/AiPanel";
import { useAi } from "./ai/store";
import { layoutFor, shownTab, useLayout, type PhoneTab } from "./state/layout";
import { Layers, SlidersHorizontal, BarChart3 } from "lucide-react";

// Plots, table and exports load with the first flight result (own chunk).
const ResultsPanel = lazy(() => import("./results/ResultsPanel").then((m) => ({ default: m.ResultsPanel })));

let started = false;

/** Text entry has its own undo and keys; sliders, switches and buttons do not. */
function isTextTarget(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  if (target.isContentEditable || target.tagName === "TEXTAREA" || target.tagName === "SELECT") return true;
  return target instanceof HTMLInputElement && !["range", "checkbox", "radio", "button", "file"].includes(target.type);
}

export function App() {
  const { t, lang } = useI18n();
  const theme = useResolvedTheme();
  const logOpen = useApp((s) => s.logOpen);
  const home = useApp((s) => s.home);
  const helpOpen = useApp((s) => s.helpOpen);
  const hasResults = useApp((s) => s.objects.some((o) => o.kind === "pa" && o.flight.result !== null));
  const wizard = useWizard((s) => s.wizard !== null);
  const exporting = useExport((s) => s.objectId !== null);
  const reporting = useReportDialog((s) => s.open);
  const palette = useReportDialog((s) => s.paletteOpen);
  const aiPanel = useAi((s) => s.panelOpen);
  const aiSettings = useAi((s) => s.settingsOpen);
  const [leftWidth, setLeftWidth] = useState<number | null>(null);
  const [rightWidth, setRightWidth] = useState<number | null>(null);
  const mode = useLayout((s) => s.mode);
  const drawer = useLayout((s) => s.drawer);
  const phoneTab = useLayout((s) => shownTab(s.tab, hasResults));

  // Responsive layout (state/layout.ts): the stylesheets key on <html data-layout>.
  useLayoutEffect(() => {
    document.documentElement.dataset.layout = mode;
  }, [mode]);

  useEffect(() => {
    const onResize = () => useLayout.getState().setMode(layoutFor(window.innerWidth));
    onResize();
    window.addEventListener("resize", onResize);
    return () => window.removeEventListener("resize", onResize);
  }, []);

  // The wizard and the assistant open in the right column: bring it into view on a tablet or phone.
  useEffect(() => {
    if (wizard || aiPanel) useLayout.getState().show("simulation");
  }, [wizard, aiPanel]);

  useLayoutEffect(() => {
    document.documentElement.dataset.theme = theme;
  }, [theme]);

  useEffect(() => {
    document.documentElement.lang = lang;
  }, [lang]);

  useEffect(() => {
    if (started) return; // StrictMode mounts twice in dev
    started = true;
    initProjects();
    initTour(isTauri() && allows("tour"));
    void connectRuntime();
  }, []);

  // App-wide keys. View keys (1-4, F, P, G) live in the viewport.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const app = useApp.getState();
      const typing = isTextTarget(e.target);
      if ((e.ctrlKey || e.metaKey) && !e.altKey) {
        const k = e.key.toLowerCase();
        if (k === "k" || (k === "p" && e.shiftKey)) {
          e.preventDefault();
          const r = useReportDialog.getState();
          if (!r.open && useExport.getState().objectId === null) r.setPalette(!r.paletteOpen);
        } else if ((k === "z" || k === "y") && !typing) {
          e.preventDefault();
          if (k === "y" || e.shiftKey) redo();
          else undo();
        } else if (k === "s") {
          e.preventDefault();
          void saveProject(e.shiftKey);
        } else if (k === "o") {
          e.preventDefault();
          const l = app.lang;
          void pickAndOpen(translate(l, "topbar.openTitle"), translate(l, "topbar.openFilter"), translate(l, "topbar.openAll"), null);
        } else if (k === "n") {
          e.preventDefault();
          showStart();
        }
        return;
      }
      // The export dialog is modal and owns Escape; no second modal (help) on top of it.
      const report = useReportDialog.getState();
      if (typing || app.busy || useExport.getState().objectId !== null || report.open || report.paletteOpen || useAi.getState().settingsOpen) return;
      if (e.key === "?" || e.key === "F1") {
        e.preventDefault();
        app.setHelpOpen(!app.helpOpen);
      } else if (e.key === "Escape") {
        // An open menu (e.g. results export) takes this Escape for itself.
        if (document.querySelector('[role="menu"]')) return;
        if (app.helpOpen) app.setHelpOpen(false);
        else if (app.home && app.objects.length) app.setHome(false);
        else if (useResults.getState().ion) useResults.getState().selectIon(null);
        else app.select(null);
      } else if ((e.key === "Delete" || e.key === "Backspace") && !app.home && app.selectedElectrode === null) {
        const o = app.objects.find((x) => x.id === app.selectedId);
        if (o) {
          app.removeObject(o.id);
          app.log("info", { key: "log.removed", params: { name: objectName(o) } });
        }
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  const layout = {
    ...(leftWidth !== null && { "--panel-left-w": `${leftWidth}px` }),
    ...(rightWidth !== null && { "--panel-right-w": `${rightWidth}px` }),
    // The import wizard needs room for its parts table.
    ...(wizard && { "--panel-right-w": `max(${rightWidth ?? 0}px, var(--panel-wizard-w))` }),
    // The assistant takes the properties column while it is open, at least its own width.
    ...(aiPanel && !wizard && { "--panel-right-w": `max(${rightWidth ?? 0}px, var(--panel-ai-w))` }),
  } as CSSProperties;

  const desktop = mode === "desktop";
  const phone = mode === "phone";
  // Tablet: closed drawers are hidden. Phone: only the selected tab's panel is shown.
  const leftShown = desktop || (phone ? phoneTab === "scene" : drawer === "scene");
  const rightShown = desktop || (phone ? phoneTab === "simulation" : drawer === "simulation");
  const results = hasResults && (
    <Suspense fallback={null}>
      <ResultsPanel />
    </Suspense>
  );

  return (
    <div className={styles.app} style={layout}>
      <TopBar />
      <div className={styles.left} data-shown={leftShown}>
        <SceneTree />
        {desktop && <PanelResizer edge="right" onResize={setLeftWidth} label={t("tree.title")} />}
        {/* Notifications stack over the bottom of the scene panel, never over the 3D view. */}
        {desktop && <Toaster />}
      </div>
      <main className={styles.center} inert={home}>
        <Viewport theme={theme} />
        {!phone && results}
        {/* Tablet and phone: the side panels can be hidden, so notifications use the 3D view's bottom. */}
        {!desktop && <Toaster />}
      </main>
      {phone && <PhoneTabs tab={phoneTab} hasResults={hasResults} />}
      {phone && phoneTab === "results" && <div className={styles.phoneResults}>{results}</div>}
      <div className={styles.right} data-shown={rightShown} inert={home}>
        {wizard ? <ImportWizard /> : aiPanel ? <AiPanel /> : <PropertiesPanel />}
        {desktop && <PanelResizer edge="left" onResize={setRightWidth} label={aiPanel && !wizard ? t("ai.panel.title") : t("props.title")} />}
      </div>
      {home && <StartScreen />}
      <Tour />
      {logOpen && <LogDrawer />}
      <StatusBar />
      <DropOverlay />
      {helpOpen && <HelpOverlay />}
      {exporting && <ExportDialog />}
      {reporting && <ReportDialog />}
      {palette && <CommandPalette />}
      {aiSettings && <AiAccountsDialog />}
      <BusyOverlay />
    </div>
  );
}

const PHONE_TABS: { id: PhoneTab; key: "layout.tab.scene" | "layout.tab.simulation" | "layout.tab.results"; icon: ReactNode }[] = [
  { id: "scene", key: "layout.tab.scene", icon: <Layers aria-hidden /> },
  { id: "simulation", key: "layout.tab.simulation", icon: <SlidersHorizontal aria-hidden /> },
  { id: "results", key: "layout.tab.results", icon: <BarChart3 aria-hidden /> },
];

/** Phone: the panels under the 3D view, one at a time. */
function PhoneTabs({ tab, hasResults }: { tab: PhoneTab; hasResults: boolean }) {
  const { t } = useI18n();
  const home = useApp((s) => s.home);
  return (
    <div className={styles.phoneTabs} role="tablist" aria-label={t("layout.tabs")} inert={home} data-testid="phone-tabs">
      {PHONE_TABS.map(({ id, key, icon }) => (
        <button
          key={id}
          type="button"
          role="tab"
          className={styles.phoneTab}
          aria-selected={tab === id}
          disabled={id === "results" && !hasResults}
          data-testid={`phone-tab-${id}`}
          onClick={() => {
            useLayout.getState().setTab(id);
            if (id === "results") useResults.getState().setOpen(true);
          }}
        >
          {icon}
          {t(key)}
        </button>
      ))}
    </div>
  );
}
