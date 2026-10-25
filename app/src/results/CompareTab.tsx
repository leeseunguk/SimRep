import { useMemo, useRef, type CSSProperties } from "react";
import { Pin, X, ZoomOut } from "lucide-react";
import { Button } from "../components/ui/Button";
import { SegmentedControl } from "../components/ui/SegmentedControl";
import { Switch } from "../components/ui/Switch";
import { useI18n } from "../i18n";
import { paElectrodes } from "../scene/electrodes";
import type { PaObject } from "../state/store";
import { autoBins, extent, fwhm, histogram, padRange, QUANTITIES, resolvingPower, runOf, select, stats, UNITS, type Quantity, type RunData } from "./analysis";
import { digitsFor, Select, Tool, useBinOptions, useFigure, useFilterOptions, useQuantityLabel } from "./common";
import { Chart } from "./plot/ChartView";
import type { ChartSpec, Layer } from "./plot/chart";
import { timed } from "./plot/perf";
import { usePlotTheme } from "./plot/usePlotTheme";
import { MAX_SNAPSHOTS, useResults } from "./store";
import styles from "./ResultsPanel.module.css";

/** "#2 Focus 110 V · 10,000 ions · live": what a kept run was flown with. */
export function useRunDetail() {
  const { t, num } = useI18n();
  return (o: PaObject) => {
    const volts = paElectrodes(o)
      .filter((e) => e.kind === "adjustable" && e.number)
      .map((e) => `#${e.number} ${num(e.voltage ?? 0, 1)} V`)
      .join(", ");
    const h = o.flight.result!.header;
    return [volts, t("results.compare.ions", { n: h.ions }), t(h.precision === "live" ? "results.mode.live" : "results.mode.precise")].filter(Boolean).join(" · ");
  };
}

interface Series {
  key: string;
  label: string;
  detail: string;
  color: string;
  run: RunData;
  current: boolean;
  visible: boolean;
  snapshot?: number;
}

/** Overlaid histograms of the current run and kept runs (e.g. before / after a voltage change). */
export function CompareTab({ o }: { o: PaObject }) {
  const { t, num } = useI18n();
  const view = useResults((s) => s.compare);
  const setView = (patch: Partial<typeof view>) => useResults.getState().view("compare", patch);
  const snapshots = useResults((s) => s.snapshots);
  const theme = usePlotTheme();
  const label = useQuantityLabel();
  const detailOf = useRunDetail();
  const plotRef = useRef<HTMLDivElement>(null);
  const run = runOf(o.flight.result!);
  const filters = useFilterOptions(run);
  const q = view.quantity;
  const unit = UNITS[q];

  const series: Series[] = useMemo(
    () => [
      { key: "current", label: t("results.compare.current"), detail: detailOf(o), color: theme.primary, run, current: true, visible: true },
      ...snapshots.map((s) => ({ key: `s${s.id}`, label: s.label, detail: s.detail, color: theme.series[s.color] ?? theme.primary, run: s.run, current: false, visible: s.visible, snapshot: s.id })),
    ],
    [snapshots, run, theme, t, o], // detailOf follows t
  );

  const data = useMemo(
    () =>
      timed(() => {
        const shown = series.filter((s) => s.visible);
        const values = shown.map((s) => select(s.run, q, view.filter));
        let lo = Infinity;
        let hi = -Infinity;
        for (const v of values) {
          const e = extent(v);
          if (e) [lo, hi] = [Math.min(lo, e[0]), Math.max(hi, e[1])];
        }
        if (!(lo <= hi)) return null;
        const range = view.zoom ?? padRange([lo, hi]);
        const inView = values.map((v) => (view.zoom ? v.filter((x) => x >= range[0] && x <= range[1]) : v));
        const bins = view.bins || autoBins(Math.max(...inView.map((v) => v.length)));
        const hists = inView.map((v) => {
          const h = histogram(v, range[0], range[1], bins);
          if (view.normalize && v.length) for (let i = 0; i < h.counts.length; i++) h.counts[i]! /= v.length;
          return h;
        });
        return {
          range,
          bins,
          rows: shown.map((s, i) => {
            const st = stats(inView[i]!);
            const raw = histogram(inView[i]!, range[0], range[1], bins);
            return { s, h: hists[i]!, st, fw: fwhm(raw) };
          }),
        };
      }),
    [series, q, view.filter, view.zoom, view.bins, view.normalize],
  );

  const binOptions = useBinOptions(data ? data.bins : 0);
  const d = data ? digitsFor(data.range[1] - data.range[0]) : 3;

  const spec = useMemo<ChartSpec | null>(() => {
    if (!data) return null;
    let max = 0;
    for (const r of data.rows) for (const c of r.h.counts) max = Math.max(max, c);
    // Kept runs first, the current run on top.
    const layers: Layer[] = [...data.rows]
      .reverse()
      .map((r) => ({ kind: "hist", lo: r.h.lo, hi: r.h.hi, counts: r.h.counts, stroke: r.s.color, width: r.s.current ? theme.lineW * 1.5 : theme.lineW }) as Layer);
    const minY = view.normalize ? 1e-6 : 0.5;
    return {
      x: { label: label(q), domain: data.range },
      y: { label: t(view.normalize ? "results.axis.fraction" : "results.axis.ions"), domain: view.log ? [minY, max * 2] : [0, (max || 1) * 1.08], log: view.log },
      layers,
      legend: data.rows.map((r) => ({ label: r.s.label, color: r.s.color })),
    };
  }, [data, q, view.log, view.normalize, theme, label, t]);

  useFigure(`compare-${q}`, spec ? [{ spec, ref: plotRef }] : []);

  const currentMean = data?.rows.find((r) => r.s.current)?.st?.mean;
  const keep = () => useResults.getState().keep(o, run, detailOf(o));

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
        <Tool label={t("results.compare.normalize")}>
          <Switch checked={view.normalize} label={t("results.compare.normalize")} onChange={(normalize) => setView({ normalize })} />
        </Tool>
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
          {spec ? <Chart spec={spec} ariaLabel={t("results.tab.compare")} onZoom={(zoom) => setView({ zoom })} /> : <p className={styles.note}>{t("results.noIons")}</p>}
        </div>
        <aside className={styles.side}>
          <Button size="sm" variant="primary" icon={<Pin />} onClick={keep} title={t("results.compare.keepHint", { max: MAX_SNAPSHOTS })}>
            {t("results.compare.keep")}
          </Button>
          {snapshots.length === 0 && <p className={styles.note}>{t("results.compare.empty")}</p>}
          <div className={styles.runs}>
            {series.map((s) => {
              const row = data?.rows.find((r) => r.s.key === s.key);
              const delta = !s.current && row?.st && currentMean !== undefined ? row.st.mean - currentMean : null;
              const R = q === "tof" && row?.fw ? resolvingPower(row.fw) : null;
              return (
                <div key={s.key} className={styles.run} data-hidden={!s.visible} style={{ "--swatch": s.color } as CSSProperties}>
                  <button
                    type="button"
                    className={styles.runToggle}
                    disabled={s.current}
                    aria-pressed={s.visible}
                    aria-label={s.current ? `${t("results.compare.current")}: ${s.label}` : `${t(s.visible ? "results.compare.hide" : "results.compare.show")}: ${s.label}`}
                    title={s.current ? undefined : t(s.visible ? "results.compare.hide" : "results.compare.show")}
                    onClick={() => s.snapshot && useResults.getState().patchSnapshot(s.snapshot, { visible: !s.visible })}
                  >
                    <span className={styles.runSwatch} />
                  </button>
                  <span className={styles.runName} title={s.label}>
                    {s.label}
                  </span>
                  {s.snapshot ? (
                    <Button size="sm" variant="ghost" iconOnly icon={<X />} aria-label={t("results.compare.remove")} title={t("results.compare.remove")} onClick={() => useResults.getState().removeSnapshot(s.snapshot!)} />
                  ) : (
                    <span />
                  )}
                  <span className={styles.runDetail}>{s.detail}</span>
                  {row?.st && (
                    <span className={styles.runStats}>
                      μ {num(row.st.mean, d + 1)} · σ {num(row.st.std, d + 1)} {unit}
                      {row.fw && ` · FWHM ${num(row.fw.width, d + 1)}`}
                      {R !== null && Number.isFinite(R) && ` · R ${num(R, 0)}`}
                      {delta !== null && (
                        <span className={styles.delta}>{` · Δμ ${delta > 0 ? "+" : ""}${num(delta, d + 1)}`}</span>
                      )}
                    </span>
                  )}
                </div>
              );
            })}
          </div>
        </aside>
      </div>
    </>
  );
}
