import { useMemo, useRef } from "react";
import { ZoomOut } from "lucide-react";
import { Button } from "../components/ui/Button";
import { useI18n } from "../i18n";
import type { PaObject } from "../state/store";
import { autoBins, extent, fwhm, histogram, padRange, QUANTITIES, resolvingPower, runOf, select, stats, UNITS, type Quantity } from "./analysis";
import { digitsFor, Headline, Select, StatList, Tool, useBinOptions, useFigure, useFilterOptions, useQuantityLabel } from "./common";
import { usePlotTheme } from "./plot/usePlotTheme";
import { Chart } from "./plot/ChartView";
import type { ChartSpec, Layer } from "./plot/chart";
import { timed } from "./plot/perf";
import { SegmentedControl } from "../components/ui/SegmentedControl";
import { Switch } from "../components/ui/Switch";
import { useResults } from "./store";
import styles from "./ResultsPanel.module.css";

/** Distribution of one end-state quantity, with FWHM and (for TOF) mass resolving power. */
export function HistogramTab({ o }: { o: PaObject }) {
  const { t, num } = useI18n();
  const flight = o.flight.result!;
  const view = useResults((s) => s.hist);
  const setView = (patch: Partial<typeof view>) => useResults.getState().view("hist", patch);
  const ion = useResults((s) => (s.ion?.objectId === o.id ? s.ion.index : null));
  const label = useQuantityLabel();
  const run = runOf(flight);
  const filters = useFilterOptions(run);
  const plotRef = useRef<HTMLDivElement>(null);
  const theme = usePlotTheme();
  const q = view.quantity;

  const data = useMemo(
    () =>
      timed(() => {
        const all = select(run, q, view.filter);
        const full = extent(all);
        if (!full) return null;
        const range = view.zoom ?? padRange(full);
        const values = view.zoom ? select(run, q, view.filter, range) : all;
        const bins = view.bins || autoBins(values.length);
        const h = histogram(values, range[0], range[1], bins);
        return { range, values, bins, h, st: stats(values), fw: fwhm(h), total: all.length };
      }),
    [run, q, view.filter, view.zoom, view.bins],
  );

  const binOptions = useBinOptions(data ? autoBins(data.values.length) : 0);
  const d = data ? digitsFor(data.range[1] - data.range[0]) : 3;
  const unit = UNITS[q];

  const spec = useMemo<ChartSpec | null>(() => {
    if (!data) return null;
    let max = 0;
    for (const c of data.h.counts) max = Math.max(max, c);
    const layers: Layer[] = [];
    if (data.fw) layers.push({ kind: "band", x0: data.fw.left, x1: data.fw.right, color: theme.band });
    layers.push({ kind: "hist", lo: data.h.lo, hi: data.h.hi, counts: data.h.counts, fill: theme.primaryFill, stroke: theme.primary });
    if (data.st) layers.push({ kind: "vline", x: data.st.mean, color: theme.textStrong, dash: [4, 3] });
    if (ion !== null && ion < run.n) layers.push({ kind: "vline", x: run.cols[q][ion]!, color: theme.select, width: theme.selectW });
    return {
      x: { label: label(q), domain: data.range },
      y: { label: t("results.axis.ions"), domain: view.log ? [0.5, max * 2] : [0, Math.max(1, max) * 1.08], log: view.log },
      layers,
    };
  }, [data, ion, run, q, view.log, theme, label, t]);

  useFigure(`${q}-histogram`, spec ? [{ spec, ref: plotRef }] : []);

  const readout = (x: number) => {
    if (!data) return null;
    const { h } = data;
    const w = (h.hi - h.lo) / h.counts.length;
    const k = Math.floor((x - h.lo) / w);
    if (k < 0 || k >= h.counts.length) return null;
    return [`${num(h.lo + k * w, d)} – ${num(h.lo + (k + 1) * w, d)} ${unit}`, t("results.readout.ions", { n: h.counts[k]! })];
  };

  const R = q === "tof" && data?.fw ? resolvingPower(data.fw) : null;
  return (
    <>
      <div className={styles.toolbar}>
        <SegmentedControl<Quantity>
          ariaLabel={t("results.quantity")}
          value={q}
          onChange={(quantity) => setView({ quantity, zoom: null })}
          options={QUANTITIES.map((v) => ({ value: v, label: label(v, false), title: label(v) }))}
        />
        <Select label={t("results.filter")} value={view.filter} options={filters} onChange={(filter) => setView({ filter, zoom: null })} />
        <Select label={t("results.bins")} value={String(view.bins)} options={binOptions} onChange={(b) => setView({ bins: Number(b) })} />
        <Tool label={t("results.logScale")}>
          <Switch checked={view.log} label={t("results.logScale")} onChange={(log) => setView({ log })} />
        </Tool>
        {view.zoom && (
          <Button size="sm" variant="ghost" icon={<ZoomOut />} onClick={() => setView({ zoom: null })}>
            {t("results.zoomReset")}
          </Button>
        )}
      </div>
      <div className={styles.main}>
        <div className={styles.plot} ref={plotRef}>
          {spec ? (
            <Chart spec={spec} ariaLabel={label(q)} readout={readout} onZoom={(zoom) => setView({ zoom })} />
          ) : (
            <p className={styles.note}>{t("results.noIons")}</p>
          )}
        </div>
        <aside className={styles.side}>
          {data?.fw &&
            (R !== null ? (
              <Headline title={t("results.fwhmNote")} label={t("results.resolution")} value={Number.isFinite(R) ? num(R, R >= 100 ? 0 : 1) : "∞"} sub={t("results.resolutionSub", { fwhm: num(data.fw.width, d + 1), unit })} />
            ) : (
              <Headline title={t("results.fwhmNote")} label={t("results.fwhm")} value={`${num(data.fw.width, d + 1)} ${unit}`} sub={t("results.peakAt", { x: num(data.fw.peak, d), unit })} />
            ))}
          {data?.st && (
            <StatList
              rows={[
                [t("results.stat.n"), view.zoom ? `${num(data.st.n)} / ${num(data.total)}` : num(data.st.n)],
                [t("results.stat.mean"), `${num(data.st.mean, d + 1)} ${unit}`],
                [t("results.stat.std"), `${num(data.st.std, d + 1)} ${unit}`],
                [t("results.stat.min"), `${num(data.st.min, d + 1)} ${unit}`],
                [t("results.stat.max"), `${num(data.st.max, d + 1)} ${unit}`],
                ...(data.fw ? ([[t("results.stat.peak"), `${num(data.fw.peak, d + 1)} ${unit}`]] as [string, string][]) : []),
              ]}
            />
          )}
          <p className={styles.note}>{view.zoom ? t("results.zoomed") : t("results.zoomHint")}</p>
        </aside>
      </div>
    </>
  );
}
