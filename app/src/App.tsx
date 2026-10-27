import { lazy, Suspense, useEffect, useLayoutEffect, useState, type CSSProperties } from "react";
import { isTauri } from "@tauri-apps/api/core";
import { DropOverlay } from "./components/DropOverlay";
import { ExportDialog } from "./components/ExportDialog";
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
import { initTour } from "./state/tour";
import styles from "./App.module.css";
import { AiAccountsDialog } from "./components/ai/AiAccountsDialog";
import { AiPanel } from "./components/ai/AiPanel";
import { useAi } from "./ai/store";

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
  const aiPanel = useAi((s) => s.panelOpen);
  const aiSettings = useAi((s) => s.settingsOpen);
  const [leftWidth, setLeftWidth] = useState<number | null>(null);
  const [rightWidth, setRightWidth] = useState<number | null>(null);

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
    initTour(isTauri());
    void connectRuntime();
  }, []);

  // App-wide keys. View keys (1-4, F, P, G) live in the viewport.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const app = useApp.getState();
      const typing = isTextTarget(e.target);
      if ((e.ctrlKey || e.metaKey) && !e.altKey) {
        const k = e.key.toLowerCase();
        if ((k === "z" || k === "y") && !typing) {
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
      if (typing || app.busy || useExport.getState().objectId !== null || useAi.getState().settingsOpen) return;
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

  return (
    <div className={styles.app} style={layout}>
      <TopBar />
      <div className={styles.left}>
        <SceneTree />
        <PanelResizer edge="right" onResize={setLeftWidth} label={t("tree.title")} />
        {/* Notifications stack over the bottom of the scene panel, never over the 3D view. */}
        <Toaster />
      </div>
      <main className={styles.center} inert={home}>
        <Viewport theme={theme} />
        {hasResults && (
          <Suspense fallback={null}>
            <ResultsPanel />
          </Suspense>
        )}
      </main>
      <div className={styles.right} inert={home}>
        {wizard ? <ImportWizard /> : aiPanel ? <AiPanel /> : <PropertiesPanel />}
        <PanelResizer edge="left" onResize={setRightWidth} label={aiPanel && !wizard ? t("ai.panel.title") : t("props.title")} />
      </div>
      {home && <StartScreen />}
      <Tour />
      {logOpen && <LogDrawer />}
      <StatusBar />
      <DropOverlay />
      {helpOpen && <HelpOverlay />}
      {exporting && <ExportDialog />}
      {aiSettings && <AiAccountsDialog />}
      <BusyOverlay />
    </div>
  );
}
