import { useMemo, useRef } from "react";
import { SegmentedControl } from "../components/ui/SegmentedControl";
import { Switch } from "../components/ui/Switch";
import { surfaceLabel } from "../components/simulationFormat";
import { useI18n } from "../i18n";
import type { PaObject } from "../state/store";
import { extentAt, heat, indicesOf, landed, nearestIon, PLANES, planeAxes, runOf, statsAt, surfaceKey, UNITS, type MapPlane, type Quantity } from "./analysis";
import { digitsFor, Headline, Select, StatList, Tool, useFigure, useFilterOptions, useQuantityLabel } from "./common";
import { colorRamp, heatRaster, pointRaster, type ChartSpec, type Layer } from "./plot/chart";
import { Chart } from "./plot/ChartView";
import { timed } from "./plot/perf";
import { usePlotTheme } from "./plot/usePlotTheme";
import { useResults } from "./store";
import styles from "./ResultsPanel.module.css";

/** Raster size of the point view (px); drawn scaled into the plot. */
const POINT_RASTER = 640;
const POINT_SIZE = 3;
const PICK_PX = 8;

/** Where ions hit one end surface: density (2D histogram) or points coloured by TOF / energy. */
export function DetectorTab({ o }: { o: PaObject }) {
  const { t, tm, num } = useI18n();
  const flight = o.flight.result!;
  const view = useResults((s) => s.map);
  const setView = (patch: Partial<typeof view>) => useResults.getState().view("map", patch);
  const ion = useResults((s) => (s.ion?.objectId === o.id ? s.ion.index : null));
  const label = useQuantityLabel();
  const theme = usePlotTheme();
  const plotRef = useRef<HTMLDivElement>(null);
  const run = runOf(flight);
  const options = useFilterOptions(run, true);
  // Default: the surface most ions landed on.
  const best = [...run.surfaces].filter((s) => landed(s.surface)).sort((a, b) => b.count - a.count)[0];
  const key = view.surface && run.surfaces.some((s) => surfaceKey(s.surface) === view.surface) ? view.surface : best ? surfaceKey(best.surface) : null;
  const surface = run.surfaces.find((s) => surfaceKey(s.surface) === key)?.surface ?? null;
  const [ax, ay] = planeAxes(view.plane, surface);
  const lut = useMemo(() => colorRamp(theme.heat), [theme]);

  const data = useMemo(
    () =>
      timed(() => {
        if (!key) return null;
        const idx = indicesOf(run, key);
        if (!idx.length) return null;
        const xs = run.cols[ax];
        const ys = run.cols[ay];
        const ex = extentAt(xs, idx)!;
        const ey = extentAt(ys, idx)!;
        // Margins scale with the larger extent, so a flat pattern (all z = 0 on a 2D array) keeps a
        // visible width; density cells are square in mm.
        const big = Math.max(ex[1] - ex[0], ey[1] - ey[0], Math.abs(ex[0]) * 1e-4, Math.abs(ey[0]) * 1e-4, 1e-3);
        const pad = (r: [number, number]) => [r[0] - big * 0.04, r[1] + big * 0.04] as [number, number];
        const xr = pad(ex);
        const yr = pad(ey);
        const xw = xr[1] - xr[0];
        const yw = yr[1] - yr[0];
        const aspect = yw / xw;
        let raster;
        let color: [number, number] | null = null;
        if (view.mode === "density") {
          const cell = Math.max(xw, yw) / view.bins;
          const nx = Math.max(1, Math.round(xw / cell));
          const ny = Math.max(1, Math.round(yw / cell));
          raster = heatRaster(heat(xs, ys, idx, xr, yr, nx, ny), nx, ny, lut, view.log);
        } else {
          const by = run.cols[view.colorBy];
          color = extentAt(by, idx);
          const [c0, c1] = color ?? [0, 1];
          const span = c1 - c0 || 1;
          const w = Math.max(POINT_SIZE * 2, Math.round(POINT_RASTER * Math.min(1, 1 / aspect)));
          const h = Math.max(POINT_SIZE * 2, Math.round(POINT_RASTER * Math.min(1, aspect)));
          raster = pointRaster(xs, ys, idx, xr, yr, w, h, POINT_SIZE, lut, (i) => (by[i]! - c0) / span);
        }
        return { idx, xs, ys, xr, yr, raster, color, sx: statsAt(xs, idx), sy: statsAt(ys, idx) };
      }),
    [run, key, ax, ay, view.mode, view.bins, view.log, view.colorBy, lut],
  );

  const spec = useMemo<ChartSpec | null>(() => {
    if (!data) return null;
    const layers: Layer[] = [{ kind: "raster", raster: data.raster, x: data.xr, y: data.yr }];
    if (ion !== null && ion < run.n && data.idx.includes(ion)) layers.push({ kind: "marker", x: data.xs[ion]!, y: data.ys[ion]!, color: theme.select });
    return { x: { label: label(ax), domain: data.xr }, y: { label: label(ay), domain: data.yr }, layers, equal: true };
  }, [data, ion, run, ax, ay, theme, label]);

  useFigure(`detector-${ax}${ay}`, spec ? [{ spec, ref: plotRef }] : []);

  const d = data ? digitsFor(Math.max(data.xr[1] - data.xr[0], data.yr[1] - data.yr[0])) : 3;
  const surfaceName = surface ? tm(surfaceLabel(surface)) : "";
  const planeLabel = (p: Exclude<MapPlane, "auto">) => `${p[0]}–${p[1]}`;

  return (
    <>
      <div className={styles.toolbar}>
        <Select label={t("results.map.surface")} value={key ?? ""} options={options} onChange={(surface) => setView({ surface })} />
        <Select<MapPlane>
          label={t("results.map.plane")}
          value={view.plane}
          options={[{ value: "auto", label: t("results.map.planeAuto", { plane: `${ax}–${ay}` }) }, ...PLANES.map((p) => ({ value: p, label: planeLabel(p) }))]}
          onChange={(plane) => setView({ plane })}
        />
        <SegmentedControl<"density" | "points">
          ariaLabel={t("results.map.mode")}
          value={view.mode}
          onChange={(mode) => setView({ mode })}
          options={[
            { value: "density", label: t("results.map.density") },
            { value: "points", label: t("results.map.points") },
          ]}
        />
        {view.mode === "density" ? (
          <>
            <Select
              label={t("results.bins")}
              value={String(view.bins)}
              options={[40, 80, 160, 320].map((b) => ({ value: String(b), label: num(b) }))}
              onChange={(b) => setView({ bins: Number(b) })}
            />
            <Tool label={t("results.logScale")}>
              <Switch checked={view.log} label={t("results.logScale")} onChange={(log) => setView({ log })} />
            </Tool>
          </>
        ) : (
          <Select<Quantity>
            label={t("results.map.colorBy")}
            value={view.colorBy}
            options={(["tof", "ke"] as const).map((q) => ({ value: q, label: label(q) }))}
            onChange={(colorBy) => setView({ colorBy })}
          />
        )}
        <span className={styles.toolbarSpacer} />
        <span className={styles.note}>{t("results.map.pickHint")}</span>
      </div>
      <div className={styles.main}>
        <div className={styles.plot} ref={plotRef}>
          {spec && data ? (
            <Chart
              spec={spec}
              ariaLabel={t("results.tab.map")}
              readout={(x, y) => [`${ax} ${num(x, d)} · ${ay} ${num(y, d)} mm`]}
              onPick={(x, y, f) => {
                const sx = f.width / (f.x[1] - f.x[0]);
                const sy = f.height / (f.y[1] - f.y[0]);
                const i = nearestIon(data.xs, data.ys, data.idx, x, y, sx, sy, PICK_PX);
                if (i !== null) useResults.getState().selectIon({ objectId: o.id, index: i });
              }}
            />
          ) : (
            <p className={styles.note}>{t("results.noIons")}</p>
          )}
        </div>
        <aside className={styles.side}>
          {data && <Headline label={surfaceName} value={num(data.idx.length)} sub={t("results.map.share", { pct: num((100 * data.idx.length) / run.n, 1) })} />}
          {data?.sx && data.sy && (
            <StatList
              rows={[
                [t("results.map.centroid"), `${num(data.sx.mean, d)}, ${num(data.sy.mean, d)} mm`],
                [t("results.map.rms", { axis: ax }), `${num(data.sx.std, d + 1)} mm`],
                [t("results.map.rms", { axis: ay }), `${num(data.sy.std, d + 1)} mm`],
                [t("results.map.span", { axis: ax }), `${num(data.sx.max - data.sx.min, d)} mm`],
                [t("results.map.span", { axis: ay }), `${num(data.sy.max - data.sy.min, d)} mm`],
              ]}
            />
          )}
          {data && (
            <div className={styles.ramp}>
              <span className={styles.note}>
                {view.mode === "density" ? t(view.log ? "results.map.rampLog" : "results.map.ramp") : `${label(view.colorBy)}`}
              </span>
              <div className={styles.rampBar} />
              <div className={styles.rampLabels}>
                <span>{view.mode === "density" ? "0" : data.color ? num(data.color[0], digitsFor(data.color[1] - data.color[0])) : ""}</span>
                <span>
                  {view.mode === "density"
                    ? t("results.map.max")
                    : data.color
                      ? `${num(data.color[1], digitsFor(data.color[1] - data.color[0]))} ${UNITS[view.colorBy]}`
                      : ""}
                </span>
              </div>
            </div>
          )}
        </aside>
      </div>
    </>
  );
}
