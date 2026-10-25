import { useEffect, useLayoutEffect, useMemo, useRef, useState, type KeyboardEvent } from "react";
import { ArrowDown, ArrowUp } from "lucide-react";
import { surfaceLabel } from "../components/simulationFormat";
import { useI18n } from "../i18n";
import type { MessageKey } from "../i18n/en";
import type { PaObject } from "../state/store";
import { extent, indicesOf, QUANTITIES, runOf, UNITS } from "./analysis";
import { digitsFor, Select, useFilterOptions } from "./common";
import { timed } from "./plot/perf";
import { useResults, type TableColumn } from "./store";
import styles from "./ResultsPanel.module.css";

const COLUMNS: TableColumn[] = ["index", "surface", ...QUANTITIES];
const OVERSCAN = 8;

/** Every ion's end state: virtualized rows (100k+), sortable, filtered by surface, linked selection. */
export function TableTab({ o }: { o: PaObject }) {
  const { t, tm, num } = useI18n();
  const run = runOf(o.flight.result!);
  const view = useResults((s) => s.table);
  const setView = (patch: Partial<typeof view>) => useResults.getState().view("table", patch);
  const selected = useResults((s) => (s.ion?.objectId === o.id ? s.ion.index : null));
  const filters = useFilterOptions(run);
  const scroller = useRef<HTMLDivElement>(null);
  const [scroll, setScroll] = useState({ top: 0, height: 0 });
  const rowH = useMemo(() => parseFloat(getComputedStyle(document.documentElement).getPropertyValue("--table-row-h")) || 24, []);

  const order = useMemo(
    () =>
      timed(() => {
        const idx = Uint32Array.from(indicesOf(run, view.filter));
        if (view.sort === "surface") idx.sort((a, b) => run.surface[a]! - run.surface[b]! || a - b);
        else if (view.sort !== "index") {
          const col = run.cols[view.sort];
          idx.sort((a, b) => col[a]! - col[b]! || a - b);
        }
        if (view.desc) idx.reverse();
        return idx;
      }),
    [run, view.sort, view.desc, view.filter],
  );

  const digits = useMemo(() => Object.fromEntries(QUANTITIES.map((q) => [q, Math.min(5, digitsFor((extent(run.cols[q]) ?? [0, 1]).reduce((a, b) => b - a)) + 1)])), [run]);
  const labels = useMemo(() => run.surfaces.map((s) => tm(surfaceLabel(s.surface))), [run, tm]);

  useEffect(() => {
    const el = scroller.current!;
    const ro = new ResizeObserver(() => setScroll({ top: el.scrollTop, height: el.clientHeight }));
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  // Selection made elsewhere (3D view, detector map): bring its row into view.
  const position = useMemo(() => (selected === null ? -1 : order.indexOf(selected)), [order, selected]);
  useLayoutEffect(() => {
    const el = scroller.current;
    if (!el || position < 0) return;
    const top = position * rowH;
    if (top < el.scrollTop) el.scrollTop = top;
    else if (top + rowH > el.scrollTop + el.clientHeight) el.scrollTop = top + rowH - el.clientHeight;
  }, [position, rowH]);

  const select = (index: number) => useResults.getState().selectIon({ objectId: o.id, index });
  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    const page = Math.max(1, Math.floor(scroll.height / rowH) - 1);
    const step: Record<string, number> = { ArrowDown: 1, ArrowUp: -1, PageDown: page, PageUp: -page, Home: -Infinity, End: Infinity };
    const s = step[e.key];
    if (s === undefined || !order.length) return;
    e.preventDefault();
    const next = Math.max(0, Math.min(order.length - 1, (position < 0 ? (s > 0 ? -1 : order.length) : position) + s));
    select(order[next]!);
  };

  const first = Math.max(0, Math.floor(scroll.top / rowH) - OVERSCAN);
  const last = Math.min(order.length, Math.ceil((scroll.top + scroll.height) / rowH) + OVERSCAN);
  const rows = [];
  for (let k = first; k < last; k++) {
    const i = order[k]!;
    rows.push(
      <div key={i} className={styles.row} role="row" aria-selected={i === selected} onClick={() => select(i)}>
        <span className={styles.cell} role="cell">
          {i}
        </span>
        <span className={styles.cell} role="cell" data-col="surface">
          {labels[run.surface[i]!]}
        </span>
        {QUANTITIES.map((q) => (
          <span key={q} className={styles.cell} role="cell">
            {num(run.cols[q][i]!, digits[q])}
          </span>
        ))}
      </div>,
    );
  }

  const header = (c: TableColumn) =>
    c === "index" ? "#" : c === "surface" ? t("results.table.surface") : `${t(`results.q.${c}` as MessageKey)} (${UNITS[c]})`;

  return (
    <>
      <div className={styles.toolbar}>
        <Select label={t("results.filter")} value={view.filter} options={filters} onChange={(filter) => setView({ filter })} />
        <span className={styles.toolbarSpacer} />
        <span className={styles.note}>{t("results.table.hint")}</span>
      </div>
      <div className={styles.table} role="table" aria-label={t("results.tab.table")} aria-rowcount={order.length}>
        <div className={styles.thead} role="row">
          {COLUMNS.map((c) => (
            <button
              key={c}
              type="button"
              role="columnheader"
              className={styles.th}
              data-col={c}
              data-sorted={view.sort === c}
              aria-sort={view.sort === c ? (view.desc ? "descending" : "ascending") : "none"}
              onClick={() => setView(view.sort === c ? { desc: !view.desc } : { sort: c, desc: false })}
            >
              {header(c)}
              {view.sort === c && (view.desc ? <ArrowDown aria-hidden /> : <ArrowUp aria-hidden />)}
            </button>
          ))}
        </div>
        <div
          ref={scroller}
          className={styles.viewport}
          tabIndex={0}
          onKeyDown={onKeyDown}
          onScroll={(e) => setScroll({ top: e.currentTarget.scrollTop, height: e.currentTarget.clientHeight })}
        >
          <div style={{ height: order.length * rowH }} />
          <div className={styles.rows} style={{ top: first * rowH }}>
            {rows}
          </div>
        </div>
        <div className={styles.tableFoot}>{t("results.table.count", { shown: order.length, total: run.n })}</div>
      </div>
    </>
  );
}
