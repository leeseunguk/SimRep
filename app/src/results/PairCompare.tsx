import { useEffect, useMemo, useRef, useState, type CSSProperties } from "react";
import { X } from "lucide-react";
import { Button } from "../components/ui/Button";
import { SegmentedControl } from "../components/ui/SegmentedControl";
import { Switch } from "../components/ui/Switch";
import { useI18n } from "../i18n";
import { loadRun, useRuns, type LoadedRun } from "../state/runs";
import { settingsDiff } from "../state/runFormat";
import { autoBins, extent, extentAt, heat, histogram, histYRange, indicesOf, landed, mapWindow, padRange, planeAxes, QUANTITIES, select, stats, surfaceKey, UNITS, type Quantity } from "./analysis";
import { digitsFor, Select, Tool, useFigure, useQuantityLabel } from "./common";
import { useRunLabel } from "./HistoryTab";
import { Chart } from "./plot/ChartView";
import { colorRamp, heatRaster, type ChartSpec, type Layer } from "./plot/chart";
import { usePlotTheme } from "./plot/usePlotTheme";
import { useResults } from "./store";
import { en, type MessageKey } from "../i18n/en";

/** Run B's outline: dashed, so it stays visible where it lies on top of run A. */
const DASH = [6, 4];

/** A settings key in words ("inputs.durationUs" -> "Time limit (µs)"); the raw key is the tooltip. */
export function useSettingLabel() {
  const { t } = useI18n();
  return (key: string) => {
    const volt = /^inputs\.voltages\.(\d+)$/.exec(key);
    if (volt) return t("runs.key.voltage", { n: Number(volt[1]) });
    const k = `runs.key.${key}`;
    return k in en ? t(k as MessageKey) : key;
  };
}

/** Same ions per bin (and so the same distribution) in both runs. */
const sameCounts = (a: ArrayLike<number>, b: ArrayLike<number>) => a.length === b.length && Array.prototype.every.call(a, (v: number, i: number) => v === b[i]);
import styles from "./ResultsPanel.module.css";

const MAP_BINS = 60;

/** Two runs from the history: overlaid histograms, both detector maps on one scale, and a settings diff. */
export function PairCompare({ pair }: { pair: [string, string] }) {
  const { t, num } = useI18n();
  const [runs, setRuns] = useState<[LoadedRun | null, LoadedRun | null] | null>(null);
  const records = useRuns((s) => s.runs);
  const label = useRunLabel();
  const theme = usePlotTheme();
  const qLabel = useQuantityLabel();
  const settingLabel = useSettingLabel();
  const view = useResults((s) => s.compare);
  const setView = (patch: Partial<typeof view>) => useResults.getState().view("compare", patch);
  const [show, setShow] = useState<"plots" | "settings">("plots");
  const histRef = useRef<HTMLDivElement>(null);
  const mapRefs = [useRef<HTMLDivElement>(null), useRef<HTMLDivElement>(null)];

  useEffect(() => {
    let live = true;
    setRuns(null);
    void Promise.all(pair.map((id) => loadRun(id).catch(() => null))).then((r) => live && setRuns([r[0] ?? null, r[1] ?? null]));
    return () => {
      live = false;
    };
  }, [pair, records.length]);

  const q = view.quantity;
  const colors = [theme.primary, theme.series[1] ?? theme.primary];
  const data = runs?.map((r) => r?.run ?? null) ?? [null, null];

  const hist = useMemo(() => {
    const values = data.map((d) => (d ? select(d, q, view.filter === "all" ? "all" : "landed") : null));
    let lo = Infinity;
    let hi = -Infinity;
    for (const v of values) {
      const e = v && extent(v);
      if (e) [lo, hi] = [Math.min(lo, e[0]), Math.max(hi, e[1])];
    }
    if (!(lo <= hi)) return null;
    const range = padRange([lo, hi]);
    const bins = view.bins || autoBins(Math.max(...values.map((v) => v?.length ?? 0)));
    const hs = values.map((v) => {
      if (!v) return null;
      const h = histogram(v, range[0], range[1], bins);
      if (view.normalize && v.length) for (let i = 0; i < h.counts.length; i++) h.counts[i]! /= v.length;
      return h;
    });
    return { range, hs, st: values.map((v) => (v ? stats(v) : null)) };
  }, [data[0], data[1], q, view.filter, view.bins, view.normalize]);

  const names = runs?.map((r, i) => (r ? label(r.record) : pair[i]!)) ?? pair;

  const identical = Boolean(hist?.hs[0] && hist.hs[1] && sameCounts(hist.hs[0].counts, hist.hs[1].counts));
  const histSpec = useMemo<ChartSpec | null>(() => {
    if (!hist) return null;
    const layers: Layer[] = hist.hs.flatMap((h, i) => (h ? [{ kind: "hist", lo: h.lo, hi: h.hi, counts: h.counts, stroke: colors[i]!, width: theme.lineW * 1.5, ...(i === 1 && { dash: DASH }) } as Layer] : []));
    const yr = histYRange(hist.hs.filter((h) => h !== null).map((h) => h!.counts), view.log, view.normalize);
    return {
      x: { label: qLabel(q), domain: hist.range },
      y: { label: t(view.normalize ? "results.axis.fraction" : "results.axis.ions"), domain: yr.domain, log: view.log, floor: yr.floor },
      layers,
      legend: [
        ...names.map((n, i) => ({ label: n, color: colors[i]!, dash: i === 1 })),
        ...(identical ? [{ label: t("runs.identical"), color: theme.text }] : []),
      ],
    };
  }, [hist, identical, view.log, view.normalize, theme, q, names[0], names[1]]);

  // Detector maps: the surface most ions of the first run landed on, both runs on the same axes and scale.
  const maps = useMemo(() => {
    const first = data.find((d) => d) ?? null;
    const best = first && [...first.surfaces].filter((s) => landed(s.surface)).sort((a, b) => b.count - a.count)[0];
    if (!best) return null;
    const key = surfaceKey(best.surface);
    const [ax, ay] = planeAxes("auto", best.surface);
    const idx = data.map((d) => (d ? indicesOf(d, key) : null));
    // One window for both runs (same axes and scale), wide enough on a flat pattern to be seen.
    const extents = data.flatMap((d, i) => (d && idx[i]!.length ? [{ x: extentAt(d.cols[ax], idx[i]!)!, y: extentAt(d.cols[ay], idx[i]!)! }] : []));
    const w = mapWindow(extents, MAP_BINS);
    if (!w) return null;
    const { x, y } = w;
    const lut = colorRamp(theme.heat);
    const cells = data.map((d, i) => (d && idx[i]!.length ? heat(d.cols[ax], d.cols[ay], idx[i]!, x, y, w.nx, w.ny) : null));
    const specs = cells.map((c, i): ChartSpec | null => {
      if (!c) return null;
      const raster = heatRaster(c, w.nx, w.ny, lut, true);
      return { x: { label: qLabel(ax), domain: x }, y: { label: qLabel(ay), domain: y }, layers: [{ kind: "raster", raster, x, y }], equal: true, title: names[i] };
    });
    // Ions inside the window per map (the e2e test checks the maps are not empty).
    const ions = cells.map((c) => (c ? c.reduce((a, b) => a + b, 0) : 0));
    const filled = cells.map((c) => (c ? c.reduce((a, b) => a + (b > 0 ? 1 : 0), 0) : 0));
    return { key, specs, ions, filled };
  }, [data[0], data[1], theme, names[0], names[1]]);

  useFigure(`compare-runs-${q}`, [
    ...(histSpec && show === "plots" ? [{ spec: histSpec, ref: histRef }] : []),
    ...(show === "plots" ? (maps?.specs ?? []).flatMap((s, i) => (s ? [{ spec: s, ref: mapRefs[i]! }] : [])) : []),
  ]);

  const diff = useMemo(() => (runs?.[0] && runs[1] ? settingsDiff(runs[0].settings, runs[1].settings) : []), [runs]);
  const differing = diff.filter((r) => !r.same).length;
  const d = hist ? digitsFor(hist.range[1] - hist.range[0]) : 3;
  const missing = runs?.some((r) => !r?.run);

  return (
    <>
      <div className={styles.toolbar}>
        <SegmentedControl<"plots" | "settings">
          ariaLabel={t("runs.compareView")}
          value={show}
          onChange={setShow}
          options={[
            { value: "plots", label: t("runs.comparePlots") },
            { value: "settings", label: t("runs.compareSettings", { n: differing }) },
          ]}
        />
        {show === "plots" && (
          <>
            <SegmentedControl<Quantity> ariaLabel={t("results.quantity")} value={q} onChange={(quantity) => setView({ quantity, zoom: null })} options={QUANTITIES.map((v) => ({ value: v, label: qLabel(v, false), title: qLabel(v) }))} />
            <Select label={t("results.filter")} value={view.filter === "all" ? "all" : "landed"} options={[{ value: "landed", label: t("results.filter.landed") }, { value: "all", label: t("results.filter.all") }]} onChange={(filter) => setView({ filter })} />
            <Tool label={t("results.compare.normalize")}>
              <Switch checked={view.normalize} label={t("results.compare.normalize")} onChange={(normalize) => setView({ normalize })} />
            </Tool>
            <Tool label={t("results.logScale")}>
              <Switch checked={view.log} label={t("results.logScale")} onChange={(log) => setView({ log })} />
            </Tool>
          </>
        )}
        <span className={styles.toolbarSpacer} />
        <Button size="sm" variant="ghost" icon={<X />} onClick={() => useResults.getState().comparePair(null)} data-testid="compare-pair-close">
          {t("runs.compareClose")}
        </Button>
      </div>
      {!runs ? (
        <p className={styles.note}>{t("runs.loading")}</p>
      ) : show === "settings" ? (
        <div className={styles.historyScroll}>
          <table className={`${styles.history} ${styles.diffTable}`} aria-label={t("runs.settingsDiff")} data-testid="settings-diff">
            <thead>
              <tr>
                <th scope="col" className={styles.diffKeyCol}>{t("runs.setting")}</th>
                {names.map((n, i) => (
                  <th key={i} scope="col" className={styles.diffRunCol} style={{ "--swatch": colors[i] } as CSSProperties}>
                    <span className={styles.diffRunHead}>
                      <span className={i === 1 ? styles.dashSwatch : styles.swatchLine} aria-hidden />
                      {n}
                    </span>
                  </th>
                ))}
                <th scope="col" className={styles.diffSameCol}>
                  <span className="visually-hidden">{t("runs.diffState")}</span>
                </th>
              </tr>
            </thead>
            <tbody>
              {diff.map((row) => (
                <tr key={row.key} data-differs={!row.same || undefined} data-key={row.key}>
                  <th scope="row" className={styles.historyKey} title={row.key}>
                    {settingLabel(row.key)}
                  </th>
                  <td className="num" title={row.a}>{row.a}</td>
                  <td className="num" title={row.b}>{row.b}</td>
                  <td className={styles.diffState}>{row.same ? <span className="visually-hidden">{t("runs.same")}</span> : t("runs.differs")}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      ) : (
        <>
          {/* Per-run figures as a strip, so the three plots get the panel's whole width. */}
          <ul className={styles.pairRuns} aria-label={t("results.tab.compare")}>
            {runs.map((r, i) =>
              r ? (
                <li key={r.record.id} style={{ "--swatch": colors[i] } as CSSProperties}>
                  <span className={i === 1 ? styles.dashSwatch : styles.swatchLine} aria-hidden />
                  <b>{names[i]}</b>
                  <span className="tnum">
                    {t("results.compare.ions", { n: r.metrics.ions })} · {t("runs.col.arrivals")} {num(r.metrics.arrivals)} · {num(r.metrics.computeMs, 1)} ms
                    {hist?.st[i] && ` · μ ${num(hist.st[i]!.mean, d + 1)} · σ ${num(hist.st[i]!.std, d + 1)} ${UNITS[q]}`}
                  </span>
                </li>
              ) : null,
            )}
            {identical && (
              <li data-testid="compare-identical">
                <b>{t("runs.identical")}</b>
              </li>
            )}
            {missing && <li className={styles.warn}>{t("runs.compareMissing")}</li>}
          </ul>
          <div className={styles.pairPlots}>
            <div className={styles.plot} ref={histRef} data-testid="compare-hist" data-identical={identical} data-dashed={histSpec?.layers.some((l) => l.kind === "hist" && l.dash) ?? false}>
              {histSpec ? <Chart spec={histSpec} ariaLabel={`${t("results.tab.compare")}: ${qLabel(q)}`} /> : <p className={styles.note}>{t("results.noIons")}</p>}
            </div>
            {maps?.specs.map((s, i) => (
              <div key={i} className={styles.plot} ref={mapRefs[i]} data-testid="compare-map" data-ions={maps.ions[i]} data-cells={maps.filled[i]}>
                {s ? <Chart spec={s} ariaLabel={`${t("results.tab.map")}: ${names[i]}`} /> : <p className={styles.note}>{t("results.noIons")}</p>}
              </div>
            ))}
          </div>
        </>
      )}
    </>
  );
}
