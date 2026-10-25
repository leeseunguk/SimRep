import { Fragment, useEffect, useRef, type ReactNode, type RefObject } from "react";
import { surfaceLabel } from "../components/simulationFormat";
import inputs from "../components/ui/inputs.module.css";
import { useI18n } from "../i18n";
import type { MessageKey } from "../i18n/en";
import { landed, surfaceKey, UNITS, type Quantity, type RunData } from "./analysis";
import { registerFigure } from "./exporters";
import type { ChartSpec, Panel } from "./plot/chart";
import styles from "./ResultsPanel.module.css";

/** Decimal places that resolve ~1/1000 of a span. */
export function digitsFor(span: number): number {
  return Math.min(6, Math.max(0, 3 - Math.floor(Math.log10(Math.abs(span) || 1))));
}

export function useQuantityLabel() {
  const { t } = useI18n();
  return (q: Quantity, withUnit = true) => (withUnit ? `${t(`results.q.${q}` as MessageKey)} (${UNITS[q]})` : t(`results.q.${q}` as MessageKey));
}

export interface Option<T extends string> {
  value: T;
  label: string;
}

export function Select<T extends string>({ label, value, options, onChange }: { label: string; value: T; options: Option<T>[]; onChange(v: T): void }) {
  return (
    <label className={styles.tool}>
      {label}
      <select className={`${inputs.input} ${inputs.select} ${styles.select}`} value={value} onChange={(e) => onChange(e.target.value as T)}>
        {options.map((o) => (
          <option key={o.value} value={o.value}>
            {o.label}
          </option>
        ))}
      </select>
    </label>
  );
}

export function Tool({ label, children }: { label: string; children: ReactNode }) {
  return (
    <span className={styles.tool}>
      {label}
      {children}
    </span>
  );
}

/** Ion filter choices: landed, all, then every surface of the run with its count. */
export function useFilterOptions(run: RunData | null, onlyLanded = false): Option<string>[] {
  const { t, tm, num } = useI18n();
  if (!run) return [];
  const surfaces = [...run.surfaces].filter((s) => !onlyLanded || landed(s.surface)).sort((a, b) => b.count - a.count);
  return [
    ...(onlyLanded
      ? []
      : [
          { value: "landed", label: t("results.filter.landed") },
          { value: "all", label: t("results.filter.all") },
        ]),
    ...surfaces.map((s) => ({ value: surfaceKey(s.surface), label: `${tm(surfaceLabel(s.surface))} · ${num(s.count)}` })),
  ];
}

export const BIN_CHOICES = [0, 20, 50, 100, 200, 500];

export function useBinOptions(auto: number): Option<string>[] {
  const { t, num } = useI18n();
  return BIN_CHOICES.map((b) => ({ value: String(b), label: b === 0 ? t("results.bins.auto", { n: auto }) : num(b) }));
}

export function Headline({ label, value, sub, title }: { label: string; value: string; sub?: string; title?: string }) {
  return (
    <div className={styles.headline} title={title}>
      <span className={styles.headlineLabel}>{label}</span>
      <span className={styles.headlineValue}>{value}</span>
      {sub && <span className={styles.headlineSub}>{sub}</span>}
    </div>
  );
}

export function StatList({ rows }: { rows: [string, string][] }) {
  return (
    <dl className={styles.stats}>
      {rows.map(([k, v]) => (
        <Fragment key={k}>
          <dt>{k}</dt>
          <dd title={v}>{v}</dd>
        </Fragment>
      ))}
    </dl>
  );
}

export function Empty({ icon, title, hint, children }: { icon: ReactNode; title: string; hint?: string; children?: ReactNode }) {
  return (
    <div className={styles.empty}>
      {icon}
      <span>{title}</span>
      {hint && <span className={styles.emptyHint}>{hint}</span>}
      {children}
    </div>
  );
}

/**
 * Registers what the tab shows for "export plot": each chart's spec with its on-screen box, laid
 * out relative to the first box's offset parent (so a grid of charts exports as one figure).
 */
export function useFigure(name: string, items: { spec: ChartSpec; ref: RefObject<HTMLElement | null> }[]) {
  const latest = useRef({ name, items });
  latest.current = { name, items };
  useEffect(
    () =>
      registerFigure(() => {
        const { name: n, items: list } = latest.current;
        const boxes = list.map((i) => ({ spec: i.spec, r: i.ref.current?.getBoundingClientRect() })).filter((b) => b.r && b.r.width > 0);
        if (!boxes.length) return null;
        const x0 = Math.min(...boxes.map((b) => b.r!.left));
        const y0 = Math.min(...boxes.map((b) => b.r!.top));
        const panels: Panel[] = boxes.map((b) => ({ spec: b.spec, x: b.r!.left - x0, y: b.r!.top - y0, w: b.r!.width, h: b.r!.height }));
        return {
          name: n,
          width: Math.max(...panels.map((p) => p.x + p.w)),
          height: Math.max(...panels.map((p) => p.y + p.h)),
          panels,
        };
      }),
    [],
  );
}
