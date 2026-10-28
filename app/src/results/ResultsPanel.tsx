import { useEffect, useRef, useState, type KeyboardEvent as ReactKeyboardEvent, type PointerEvent, type ReactNode, type RefObject } from "react";
import { Activity, BarChart3, Camera, ChevronDown, ChevronUp, Crosshair, Download, FileSpreadsheet, GitCompareArrows, HistoryIcon, Image, Play, Table2 } from "lucide-react";
import { RunBadge } from "../components/SimulationSections";
import { Badge } from "../components/ui/Badge";
import { Button } from "../components/ui/Button";
import { useI18n } from "../i18n";
import { backendError, errorMessage, flightVelocities } from "../scene/backend";
import type { MessageKey } from "../i18n/en";
import type { PaObject } from "../state/store";
import { useApp } from "../state/store";
import { runOf } from "./analysis";
import { CompareTab } from "./CompareTab";
import { HistoryTab } from "./HistoryTab";
import { PairCompare } from "./PairCompare";
import { flyNow } from "../scene/simulation";
import { useRuns } from "../state/runs";
import { DetectorTab } from "./DetectorTab";
import { currentFigure, endStatesCsv, exportStem, figurePng, figureSvg, saveFile, traceCsv, viewportPng } from "./exporters";
import { HistogramTab } from "./HistogramTab";
import { IonTab } from "./IonTab";
import { useActiveArray, useResults, type ResultsTab } from "./store";
import { TableTab } from "./TableTab";
import "./trace";
import styles from "./ResultsPanel.module.css";

const TABS: { id: ResultsTab; icon: ReactNode }[] = [
  { id: "hist", icon: <BarChart3 /> },
  { id: "map", icon: <Crosshair /> },
  { id: "ion", icon: <Activity /> },
  { id: "table", icon: <Table2 /> },
  { id: "compare", icon: <GitCompareArrows /> },
  { id: "history", icon: <HistoryIcon /> },
];

/** Bottom results workspace for the active array's last flight (hidden until something has flown). */
export function ResultsPanel() {
  const { t } = useI18n();
  const o = useActiveArray();
  const open = useResults((s) => s.open);
  const height = useResults((s) => s.height);
  const tab = useResults((s) => s.tab);
  const kept = useResults((s) => s.snapshots.length);
  const ion = useResults((s) => s.ion);
  const pair = useResults((s) => s.pair);
  const runCount = useRuns((s) => s.runs.length);
  const panel = useRef<HTMLElement>(null);
  if (!o) return null;
  const h = o.flight.result!.header;
  const ionHere = ion?.objectId === o.id ? ion.index : null;

  return (
    <section ref={panel} className={styles.panel} data-tour="results" data-open={open} style={open && height !== null ? { height } : undefined} aria-label={t("results.panel")}>
      {open && <HeightResizer panel={panel} />}
      <header className={styles.header}>
        <div className={styles.tabs} role="tablist" aria-label={t("results.panel")}>
          {TABS.map(({ id, icon }) => (
            <button
              key={id}
              type="button"
              role="tab"
              data-tour={`results-tab-${id}`}
              className={styles.tab}
              aria-selected={tab === id}
              onClick={() => (open && tab === id ? useResults.getState().setOpen(false) : useResults.getState().setTab(id))}
            >
              {icon}
              {t(`results.tab.${id}` as MessageKey)}
              {id === "ion" && ionHere !== null && <span className={styles.tabCount}>#{ionHere}</span>}
              {id === "compare" && (pair ? 2 : kept) > 0 && <span className={styles.tabCount}>{pair ? 2 : kept}</span>}
              {id === "history" && runCount > 0 && <span className={styles.tabCount}>{runCount}</span>}
            </button>
          ))}
        </div>
        {/* Badges first: on a narrow header the name and the count give way before them. */}
        <div className={styles.context}>
          {o.flight.stored && (
            <>
              <Badge tone="info" title={t("results.stored.title")}>
                <span data-testid="results-stored">{t("results.stored")}</span>
              </Badge>
              <Button size="sm" variant="ghost" icon={<Play />} disabled={o.flight.status === "running"} onClick={() => void flyNow(o.id)} data-testid="results-fly-again">
                {t("results.stored.flyAgain")}
              </Button>
            </>
          )}
          <RunBadge h={h} />
          {h.surface ? (
            <Badge
              tone={h.surface.accuracy === "source" ? "success" : "warning"}
              title={t(h.surface.accuracy === "source" ? "results.surfaces.exactTitle" : "results.surfaces.approximateTitle", { file: h.surface.file })}
            >
              {t(h.surface.accuracy === "source" ? "results.surfaces.exact" : "results.surfaces.approximate")}
            </Badge>
          ) : (
            o.info.cuts && (
              <Badge tone="neutral" title={t("results.surfaces.gridTitle")}>
                {t("results.surfaces.grid")}
              </Badge>
            )
          )}
          <span className={styles.contextName} title={o.path ?? o.fileName}>
            {o.customName ?? o.fileName}
          </span>
          <span className="tnum">{t("results.context.ions", { n: h.ions })}</span>
        </div>
        <div className={styles.headerActions}>
          <ExportMenu o={o} />
          <Button
            size="sm"
            variant="ghost"
            iconOnly
            icon={open ? <ChevronDown /> : <ChevronUp />}
            aria-label={t(open ? "results.collapse" : "results.expand")}
            title={t(open ? "results.collapse" : "results.expand")}
            onClick={() => useResults.getState().setOpen(!open)}
          />
        </div>
      </header>
      {open && (
        <div className={styles.body} role="tabpanel">
          {tab === "hist" && <HistogramTab o={o} />}
          {tab === "map" && <DetectorTab o={o} />}
          {tab === "ion" && <IonTab o={o} />}
          {tab === "table" && <TableTab o={o} />}
          {tab === "compare" && (pair ? <PairCompare pair={pair} /> : <CompareTab o={o} />)}
          {tab === "history" && <HistoryTab />}
        </div>
      )}
    </section>
  );
}

/** Top-edge drag handle; double-click restores the default height. */
function HeightResizer({ panel }: { panel: RefObject<HTMLElement | null> }) {
  const { t } = useI18n();
  const bounds = () => {
    const css = getComputedStyle(document.documentElement);
    const min = parseFloat(css.getPropertyValue("--results-min-h"));
    const keep = parseFloat(css.getPropertyValue("--viewport-min-h"));
    return [min, panel.current!.parentElement!.getBoundingClientRect().height - keep] as const;
  };
  // Keyboard: arrows move the edge by 16px (Shift: 64px), Home restores the default height.
  const onKeyDown = (e: ReactKeyboardEvent<HTMLDivElement>) => {
    if (e.key === "Home") {
      e.preventDefault();
      useResults.getState().setHeight(null);
      return;
    }
    if (e.key !== "ArrowUp" && e.key !== "ArrowDown") return;
    e.preventDefault();
    const [min, max] = bounds();
    const h = panel.current!.getBoundingClientRect().height + (e.key === "ArrowUp" ? 1 : -1) * (e.shiftKey ? 64 : 16);
    useResults.getState().setHeight(Math.round(Math.min(max, Math.max(min, h))));
  };
  const onPointerDown = (e: PointerEvent<HTMLDivElement>) => {
    const el = e.currentTarget;
    const box = panel.current!;
    const [min, maxH] = bounds();
    const startY = e.clientY;
    const start = box.getBoundingClientRect().height;
    el.setPointerCapture(e.pointerId);
    const move = (ev: globalThis.PointerEvent) => {
      useResults.getState().setHeight(Math.round(Math.min(maxH, Math.max(min, start - (ev.clientY - startY)))));
    };
    const up = () => {
      el.removeEventListener("pointermove", move);
      el.removeEventListener("pointerup", up);
      el.removeEventListener("pointercancel", up);
    };
    el.addEventListener("pointermove", move);
    el.addEventListener("pointerup", up);
    el.addEventListener("pointercancel", up);
  };
  return (
    <div
      className={styles.resizer}
      role="separator"
      aria-orientation="horizontal"
      aria-label={t("results.resize")}
      title={t("results.resize")}
      tabIndex={0}
      onKeyDown={onKeyDown}
      onPointerDown={onPointerDown}
      onDoubleClick={() => useResults.getState().setHeight(null)}
    />
  );
}

function ExportMenu({ o }: { o: PaObject }) {
  const { t } = useI18n();
  const [open, setOpen] = useState(false);
  const wrap = useRef<HTMLDivElement>(null);
  const tab = useResults((s) => s.tab);
  const panelOpen = useResults((s) => s.open);
  const trace = useResults((s) => (s.trace && s.ion && s.trace.objectId === s.ion.objectId && s.trace.index === s.ion.index && s.trace.objectId === o.id ? s.trace : null));
  const gpuReady = useApp((s) => s.gpu.state === "ready");

  useEffect(() => {
    if (!open) return;
    const onDown = (e: globalThis.PointerEvent) => {
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

  const stem = exportStem(o);
  const hasPlot = panelOpen && tab !== "table";
  const act = (fn: () => Promise<void>) => () => {
    setOpen(false);
    void fn().catch((err: unknown) => useApp.getState().log("error", { key: "log.exportFailed", params: { path: stem, detail: String(err) } }));
  };
  const plot = (kind: "png" | "svg") =>
    act(async () => {
      const fig = currentFigure();
      if (!fig) return;
      const name = `${stem}-${fig.name}.${kind}`;
      await saveFile(name, kind, kind === "png" ? await figurePng(fig) : figureSvg(fig), o);
    });

  return (
    <div ref={wrap} className={styles.menuWrap} data-tour="results-export">
      <Button size="sm" variant="ghost" icon={<Download />} aria-haspopup="menu" aria-expanded={open} onClick={() => setOpen(!open)}>
        {t("results.export")}
      </Button>
      {open && (
        <div className={styles.menu} role="menu">
          <span className={styles.menuLabel}>{t("results.export.data")}</span>
          <MenuItem icon={<FileSpreadsheet />} hint="CSV" onClick={act(async () => {
              const flight = o.flight.result!;
              // Flight packets leave the final velocities out; the native core keeps them per flight.
              const velocity = await flightVelocities(o.id, flight.header.token).catch((err: unknown) => {
                const m = errorMessage(backendError(err));
                throw new Error(t(m.key, m.params));
              });
              await saveFile(`${stem}-end-states.csv`, "csv", endStatesCsv(runOf(flight), velocity), o);
            })}>
            {t("results.export.endStates")}
          </MenuItem>
          <MenuItem icon={<FileSpreadsheet />} hint="CSV" disabled={!trace} onClick={act(async () => {
              if (trace) await saveFile(`${stem}-ion-${trace.index}.csv`, "csv", traceCsv(trace.data), o);
            })}>
            {trace ? t("results.export.trace", { n: trace.index }) : t("results.export.traceNone")}
          </MenuItem>
          <div className={styles.menuSep} />
          <span className={styles.menuLabel}>{t("results.export.images")}</span>
          <MenuItem icon={<Image />} hint="PNG" disabled={!hasPlot} onClick={plot("png")}>
            {t("results.export.plot")}
          </MenuItem>
          <MenuItem icon={<Image />} hint="SVG" disabled={!hasPlot} onClick={plot("svg")}>
            {t("results.export.plot")}
          </MenuItem>
          <MenuItem
            icon={<Camera />}
            hint="PNG"
            disabled={!gpuReady}
            onClick={act(async () => {
              const png = await viewportPng();
              if (png) await saveFile(`${stem}-3d.png`, "png", png, o);
            })}
          >
            {t("results.export.view3d")}
          </MenuItem>
        </div>
      )}
    </div>
  );
}

function MenuItem({ icon, hint, disabled, onClick, children }: { icon: ReactNode; hint: string; disabled?: boolean; onClick(): void; children: ReactNode }) {
  return (
    <button type="button" role="menuitem" className={styles.menuItem} disabled={disabled} onClick={onClick}>
      {icon}
      {children}
      <span className={styles.menuHint}>{hint}</span>
    </button>
  );
}
