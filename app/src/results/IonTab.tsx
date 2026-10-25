import { createRef, useMemo, useRef, type RefObject } from "react";
import { ChevronLeft, ChevronRight, Loader2, MousePointerClick } from "lucide-react";
import { isTauri } from "@tauri-apps/api/core";
import { Button } from "../components/ui/Button";
import { SegmentedControl } from "../components/ui/SegmentedControl";
import { surfaceLabel } from "../components/simulationFormat";
import inputs from "../components/ui/inputs.module.css";
import { useI18n } from "../i18n";
import type { MessageKey } from "../i18n/en";
import type { PaObject } from "../state/store";
import { extent, runOf } from "./analysis";
import { digitsFor, Empty, Tool, useFigure } from "./common";
import { Chart } from "./plot/ChartView";
import type { ChartSpec } from "./plot/chart";
import { usePlotTheme } from "./plot/usePlotTheme";
import { useResults } from "./store";
import { TRACE_QUANTITIES, TRACE_UNITS, type Trace, type TraceQuantity } from "./trace";
import styles from "./ResultsPanel.module.css";

/** One ion: x, y, z, |v|, kinetic energy and potential along its flight, against time or x. */
export function IonTab({ o }: { o: PaObject }) {
  const { t, tm, num } = useI18n();
  const flight = o.flight.result!;
  const run = runOf(flight);
  const selected = useResults((s) => (s.ion?.objectId === o.id ? s.ion.index : null));
  const trace = useResults((s) => (s.trace && s.trace.objectId === o.id && s.trace.index === selected ? s.trace.data : null));
  const status = useResults((s) => s.traceStatus);
  const error = useResults((s) => s.traceError);
  const view = useResults((s) => s.ionView);
  const setView = (patch: Partial<typeof view>) => useResults.getState().view("ionView", patch);
  const select = (index: number) => useResults.getState().selectIon({ objectId: o.id, index: Math.max(0, Math.min(run.n - 1, Math.round(index))) });
  const qLabel = (q: TraceQuantity) => t(`results.trace.${q}` as MessageKey);

  const toggle = (q: TraceQuantity) => {
    const has = view.quantities.includes(q);
    if (has && view.quantities.length === 1) return;
    setView({ quantities: has ? view.quantities.filter((x) => x !== q) : TRACE_QUANTITIES.filter((x) => x === q || view.quantities.includes(x)) });
  };

  const picker = (
    <span className={styles.ionPick}>
      <Button size="sm" variant="ghost" iconOnly icon={<ChevronLeft />} aria-label={t("results.ion.prev")} title={t("results.ion.prev")} disabled={selected === null || selected <= 0} onClick={() => select((selected ?? 1) - 1)} />
      <input
        className={`${inputs.input} num`}
        type="number"
        min={0}
        max={run.n - 1}
        value={selected ?? ""}
        placeholder="#"
        aria-label={t("results.ion.number")}
        onChange={(e) => e.target.value !== "" && select(Number(e.target.value))}
      />
      <Button size="sm" variant="ghost" iconOnly icon={<ChevronRight />} aria-label={t("results.ion.next")} title={t("results.ion.next")} disabled={selected !== null && selected >= run.n - 1} onClick={() => select((selected ?? -1) + 1)} />
    </span>
  );

  if (selected === null) {
    return (
      <Empty icon={<MousePointerClick />} title={t("results.ion.none")} hint={t("results.ion.noneHint")}>
        <Tool label={t("results.ion.number")}>{picker}</Tool>
      </Empty>
    );
  }

  const surface = run.surfaces[run.surface[selected]!]?.surface;
  const tof = run.cols.tof[selected]!;
  return (
    <>
      <div className={styles.ionHead}>
        <Tool label={t("results.ion.label")}>{picker}</Tool>
        {surface && (
          <span className={styles.fact}>
            {t("results.ion.ended")} <b>{tm(surfaceLabel(surface))}</b>
          </span>
        )}
        <span className={styles.fact}>
          TOF <b>{num(tof, digitsFor(tof))} µs</b>
        </span>
        <span className={styles.fact}>
          {t("results.ion.ke")} <b>{num(run.cols.ke[selected]!, 2)} eV</b>
        </span>
        {trace && (
          <span className={styles.fact} title={t(trace.header.field === "resident" ? "results.ion.fieldResident" : trace.header.field === "exact" ? "results.ion.fieldExact" : "results.ion.fieldArray")}>
            {t("results.ion.trace")} <b>{t("results.ion.traceValue", { n: trace.header.samples, ms: num(trace.header.ms, 1) })}</b>
          </span>
        )}
        {status === "loading" && <Loader2 className={styles.spin} aria-label={t("results.ion.loading")} />}
        <span className={styles.toolbarSpacer} />
        <div className={styles.chips} role="group" aria-label={t("results.ion.quantities")}>
          {TRACE_QUANTITIES.map((q) => (
            <button key={q} type="button" className={styles.chip} aria-pressed={view.quantities.includes(q)} onClick={() => toggle(q)}>
              {qLabel(q)}
            </button>
          ))}
        </div>
        <SegmentedControl<"t" | "x">
          ariaLabel={t("results.ion.against")}
          value={view.xAxis}
          onChange={(xAxis) => setView({ xAxis })}
          options={[
            { value: "t", label: t("results.ion.vsTime") },
            { value: "x", label: t("results.ion.vsX") },
          ]}
        />
      </div>
      {trace ? (
        <IonCharts trace={trace} quantities={view.quantities} xAxis={view.xAxis} ion={selected} qLabel={qLabel} />
      ) : status === "error" ? (
        <Empty icon={<MousePointerClick />} title={t("results.ion.failed")} hint={error ? tm(error) : undefined} />
      ) : (
        <Empty icon={<Loader2 className={styles.spin} />} title={isTauri() ? t("results.ion.loading") : t("sim.desktopOnly")} />
      )}
    </>
  );
}

function IonCharts({ trace, quantities, xAxis, ion, qLabel }: { trace: Trace; quantities: TraceQuantity[]; xAxis: "t" | "x"; ion: number; qLabel(q: TraceQuantity): string }) {
  const { t, num } = useI18n();
  const theme = usePlotTheme();
  const refs = useRef(new Map<TraceQuantity, RefObject<HTMLDivElement | null>>());
  const refOf = (q: TraceQuantity) => {
    let r = refs.current.get(q);
    if (!r) refs.current.set(q, (r = createRef<HTMLDivElement>()));
    return r;
  };
  const xs = xAxis === "t" ? trace.t : trace.x;
  const xLabel = xAxis === "t" ? `${t("results.trace.t")} (µs)` : `x (mm)`;

  const specs = useMemo(() => {
    const xr = extent(xs) ?? [0, 1];
    const pad = (r: [number, number]): [number, number] => {
      const m = (r[1] - r[0]) * 0.06 || Math.abs(r[0]) * 0.01 || 1;
      return [r[0] - m, r[1] + m];
    };
    const colors: Record<TraceQuantity, string> = { x: theme.xyz[0], y: theme.xyz[1], z: theme.xyz[2], speed: theme.primary, ke: theme.primary, potential: theme.series[0]! };
    // x against x says nothing: drop it while plotting against x (unless it is the only one).
    const shown = xAxis === "x" && quantities.length > 1 ? quantities.filter((q) => q !== "x") : quantities;
    return shown.map((q) => {
      const ys = trace[q];
      const spec: ChartSpec = {
        title: `${t(`results.trace.${q}` as MessageKey)} (${TRACE_UNITS[q]})`,
        x: { label: xLabel, domain: xr[1] > xr[0] ? xr : pad(xr) },
        y: { label: TRACE_UNITS[q], domain: pad(extent(ys) ?? [0, 1]) },
        layers: [{ kind: "line", xs, ys, color: colors[q] }],
      };
      return { q, spec };
    });
  }, [trace, quantities, xAxis, xs, xLabel, theme, t]);

  useFigure(`ion-${ion}-${xAxis === "t" ? "vs-time" : "vs-x"}`, specs.map(({ q, spec }) => ({ spec, ref: refOf(q) })));

  /** Nearest sample to x (time is sorted; x may not be). */
  const sampleAt = (x: number) => {
    let best = 0;
    for (let i = 1; i < xs.length; i++) if (Math.abs(xs[i]! - x) < Math.abs(xs[best]! - x)) best = i;
    return best;
  };

  return (
    <div className={styles.ionGrid}>
      {specs.map(({ q, spec }) => (
        <div key={q} className={styles.plot} ref={refOf(q)}>
          <Chart
            spec={spec}
            ariaLabel={spec.title!}
            readout={(x) => {
              const i = sampleAt(x);
              const d = digitsFor(Math.abs(trace[q][i]!) || 1);
              return [`t ${num(trace.t[i]!, 4)} µs · x ${num(trace.x[i]!, 3)} mm`, `${qLabel(q)} ${num(trace[q][i]!, d)} ${TRACE_UNITS[q]}`];
            }}
          />
        </div>
      ))}
    </div>
  );
}
