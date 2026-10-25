import { useEffect, useRef, useState, type PointerEvent, type ReactNode, type RefObject } from "react";
import { Activity, BarChart3, Camera, ChevronDown, ChevronUp, Crosshair, Download, FileSpreadsheet, GitCompareArrows, Image, Table2 } from "lucide-react";
import { Badge } from "../components/ui/Badge";
import { Button } from "../components/ui/Button";
import { useI18n } from "../i18n";
import { backendError, errorMessage, flightVelocities } from "../scene/backend";
import type { MessageKey } from "../i18n/en";
import type { PaObject } from "../state/store";
import { useApp } from "../state/store";
import { runOf } from "./analysis";
import { CompareTab } from "./CompareTab";
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
              {id === "compare" && kept > 0 && <span className={styles.tabCount}>{kept}</span>}
            </button>
          ))}
        </div>
        <div className={styles.context}>
          <span className={styles.contextName} title={o.path ?? o.fileName}>
            {o.customName ?? o.fileName}
          </span>
          <span className="num">{t("results.context.ions", { n: h.ions })}</span>
          <Badge tone={h.precision === "live" ? "info" : "success"}>{t(h.precision === "live" ? "results.mode.live" : "results.mode.precise")}</Badge>
          {h.surface && (
            <Badge
              tone={h.surface.accuracy === "source" ? "success" : "warning"}
              title={t(h.surface.accuracy === "source" ? "results.surfaces.exactTitle" : "results.surfaces.approximateTitle", { file: h.surface.file })}
            >
              {t(h.surface.accuracy === "source" ? "results.surfaces.exact" : "results.surfaces.approximate")}
            </Badge>
          )}
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
          {tab === "compare" && <CompareTab o={o} />}
        </div>
      )}
    </section>
  );
}

/** Top-edge drag handle; double-click restores the default height. */
function HeightResizer({ panel }: { panel: RefObject<HTMLElement | null> }) {
  const { t } = useI18n();
  const onPointerDown = (e: PointerEvent<HTMLDivElement>) => {
    const el = e.currentTarget;
    const box = panel.current!;
    const css = getComputedStyle(document.documentElement);
    const min = parseFloat(css.getPropertyValue("--results-min-h"));
    const keep = parseFloat(css.getPropertyValue("--viewport-min-h"));
    const parent = box.parentElement!.getBoundingClientRect().height;
    const startY = e.clientY;
    const start = box.getBoundingClientRect().height;
    el.setPointerCapture(e.pointerId);
    const move = (ev: globalThis.PointerEvent) => {
      useResults.getState().setHeight(Math.round(Math.min(parent - keep, Math.max(min, start - (ev.clientY - startY)))));
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
