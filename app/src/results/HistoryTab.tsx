import { useState, type KeyboardEvent } from "react";
import { GitCompareArrows, HistoryIcon, Pencil, Pin, PinOff, StickyNote, Trash2 } from "lucide-react";
import { surfaceLabel } from "../components/simulationFormat";
import { Badge } from "../components/ui/Badge";
import { Button } from "../components/ui/Button";
import { useI18n } from "../i18n";
import type { Surface } from "../scene/flight";
import { deleteRun, pinRun, renameRun, selectedPair, setRunNote, setRunsConfig, toggleSelected, useRuns, type StoredRun } from "../state/runs";
import inputs from "../components/ui/inputs.module.css";
import { Empty } from "./common";
import { useResults } from "./store";
import styles from "./ResultsPanel.module.css";

/** "electrode:2" / "wall:+x" (analysis.surfaceKey) back to a surface, for its localized label. */
function surfaceOfKey(key: string): Surface {
  const [kind, rest = ""] = key.split(":");
  if (kind === "electrode") return rest.startsWith("V") ? { kind: "electrode", number: null, voltage: Number(rest.slice(1)) || null } : { kind: "electrode", number: Number(rest), voltage: null };
  if (kind === "wall") return { kind: "wall", face: rest as Extract<Surface, { kind: "wall" }>["face"] };
  return { kind: kind === "timeLimit" ? "timeLimit" : "stalled" };
}

export function useRunLabel() {
  const { t } = useI18n();
  return (r: Pick<StoredRun, "label" | "seq">) => r.label || t("runs.defaultLabel", { n: r.seq });
}

/** Every completed flight of the project: rename, pin, note, delete, pick two to compare. */
export function HistoryTab() {
  const { t, num, lang } = useI18n();
  const runs = useRuns((s) => s.runs);
  const selected = useRuns((s) => s.selected);
  const shown = useRuns((s) => s.shown);
  const stale = useRuns((s) => s.stale);
  const config = useRuns((s) => s.config);
  const saved = useRuns((s) => s.project !== null);
  const label = useRunLabel();
  const [editing, setEditing] = useState<{ id: string; field: "label" | "note"; value: string } | null>(null);
  const time = new Intl.DateTimeFormat(lang === "ko" ? "ko-KR" : "en-US", { month: "numeric", day: "numeric", hour: "2-digit", minute: "2-digit", second: "2-digit" });
  const showing = new Set(Object.values(shown));

  const commit = () => {
    if (!editing) return;
    void (editing.field === "label" ? renameRun(editing.id, editing.value) : setRunNote(editing.id, editing.value));
    setEditing(null);
  };
  const onKey = (e: KeyboardEvent<HTMLInputElement>) => {
    if (e.key === "Enter") commit();
    else if (e.key === "Escape") {
      e.stopPropagation();
      setEditing(null);
    }
  };
  const compare = () => {
    const pair = selectedPair();
    if (pair) useResults.getState().comparePair(pair);
  };
  /** What ran: "CPU · f64 · Precise · 40 ions". */
  const ranText = (r: StoredRun) =>
    t("runs.subtitle", {
      device: r.settings.executed.device.toUpperCase(),
      precision: r.settings.executed.precision,
      mode: t(r.settings.inputs.precision === "live" ? "ions.precision.live" : "ions.precision.precise"),
      ions: r.metrics.ions,
    });

  return (
    <>
      <div className={styles.toolbar}>
        <span className={styles.tool}>{t("runs.count", { n: runs.length, keep: config.keep })}</span>
        <label className={styles.tool}>
          {t("runs.keep")}
          <input
            className={`${inputs.input} ${styles.select} ${styles.numberInput}`}
            type="number"
            min={1}
            max={1000}
            value={config.keep}
            onChange={(e) => Number(e.target.value) >= 1 && setRunsConfig({ keep: Number(e.target.value) })}
          />
        </label>
        <label className={styles.tool} title={t("runs.capTitle")}>
          {t("runs.cap")}
          <input
            className={`${inputs.input} ${styles.select} ${styles.numberInput}`}
            type="number"
            min={1}
            value={config.capMb}
            onChange={(e) => Number(e.target.value) >= 1 && setRunsConfig({ capMb: Number(e.target.value) })}
          />
        </label>
        {!saved && runs.length > 0 && <span className={`${styles.note} ${styles.warn}`}>{t("runs.unsaved")}</span>}
        <span className={styles.toolbarSpacer} />
        <span className={styles.note} aria-live="polite">
          {t("runs.selected", { n: selected.length })}
        </span>
        <Button size="sm" variant="primary" icon={<GitCompareArrows />} disabled={selected.length !== 2} onClick={compare} data-testid="history-compare">
          {t("runs.compare")}
        </Button>
      </div>
      {runs.length === 0 ? (
        <Empty icon={<HistoryIcon />} title={t("runs.empty")} hint={t("runs.emptyHint")} />
      ) : (
        <div className={styles.historyScroll}>
          <table className={styles.history} aria-label={t("results.tab.history")} data-testid="history-table">
            <thead>
              <tr>
                <th scope="col">
                  <span className="visually-hidden">{t("runs.select")}</span>
                </th>
                <th scope="col">{t("runs.col.run")}</th>
                <th scope="col">{t("runs.col.time")}</th>
                <th scope="col" className={styles.numCol}>{t("runs.col.ions")}</th>
                <th scope="col" className={styles.numCol}>{t("runs.col.arrivals")}</th>
                <th scope="col" className={styles.numCol}>{t("runs.col.tof")}</th>
                <th scope="col">{t("runs.col.splats")}</th>
                <th scope="col" className={styles.numCol}>{t("runs.col.compute")}</th>
                <th scope="col">
                  <span className="visually-hidden">{t("runs.col.actions")}</span>
                </th>
              </tr>
            </thead>
            <tbody>
              {runs.map((r) => {
                const name = label(r);
                const isSel = selected.includes(r.id);
                const tof = r.metrics.tof;
                const edit = editing?.id === r.id ? editing : null;
                return (
                  <tr key={r.id} data-selected={isSel || undefined} data-run={r.id} data-testid="history-row">
                    <td>
                      <input type="checkbox" checked={isSel} onChange={() => toggleSelected(r.id)} aria-label={t("runs.selectRun", { name })} />
                    </td>
                    <td className={styles.historyName}>
                      {edit?.field === "label" ? (
                        <input
                          className={`${inputs.input} ${styles.select}`}
                          autoFocus
                          aria-label={t("runs.rename")}
                          value={edit.value}
                          placeholder={t("runs.defaultLabel", { n: r.seq })}
                          onChange={(e) => setEditing({ ...edit, value: e.target.value })}
                          onKeyDown={onKey}
                          onBlur={commit}
                        />
                      ) : (
                        <span className={styles.historyLabel}>
                          {r.pinned && <Pin className={styles.historyPin} aria-label={t("runs.pinned")} />}
                          <span title={r.array.path}>{name}</span>
                          {showing.has(r.id) && <Badge tone="info">{t("runs.shown")}</Badge>}
                          {stale[r.id] && (
                            <Badge tone="warning" title={t("runs.staleTitle")}>
                              {t("runs.stale")}
                            </Badge>
                          )}
                          {r.stored.dropped && (
                            <Badge tone="neutral" title={t(r.stored.dropped === "trajectories" ? "runs.droppedTrajectories" : "runs.droppedResults", { cap: config.capMb })}>
                              {t(r.stored.dropped === "trajectories" ? "runs.noTrajectories" : "runs.statsOnly")}
                            </Badge>
                          )}
                        </span>
                      )}
                      {edit?.field === "note" ? (
                        <input
                          className={`${inputs.input} ${styles.select}`}
                          autoFocus
                          aria-label={t("runs.note")}
                          value={edit.value}
                          onChange={(e) => setEditing({ ...edit, value: e.target.value })}
                          onKeyDown={onKey}
                          onBlur={commit}
                        />
                      ) : (
                        r.note && (
                          <span className={styles.historyNote} data-testid="history-note">
                            <StickyNote aria-hidden />
                            <span className="visually-hidden">{t("runs.note")}: </span>
                            {r.note}
                          </span>
                        )
                      )}
                      <span className={`${styles.historySub} tnum`} data-testid="history-subtitle">
                        {ranText(r)}
                      </span>
                    </td>
                    <td className="tnum">{time.format(new Date(r.createdAt))}</td>
                    <td className={`${styles.numCol} tnum`}>{num(r.metrics.ions)}</td>
                    <td className={`${styles.numCol} tnum`}>{num(r.metrics.arrivals)}</td>
                    <td className={`${styles.numCol} tnum`}>{tof.n ? `${num(tof.mean, 4)} ± ${num(tof.std, 4)} µs` : "–"}</td>
                    <td className={styles.historySplats}>
                      {Object.entries(r.metrics.splats)
                        .map(([k, n]) => `${t(surfaceLabel(surfaceOfKey(k)).key, surfaceLabel(surfaceOfKey(k)).params)} ${num(n)}`)
                        .join(" · ")}
                    </td>
                    <td className={`${styles.numCol} tnum`}>{num(r.metrics.computeMs, 1)} ms</td>
                    <td className={styles.historyActions}>
                      <Button size="sm" variant="ghost" iconOnly icon={<Pencil />} aria-label={`${t("runs.rename")}: ${name}`} title={t("runs.rename")} onClick={() => setEditing({ id: r.id, field: "label", value: r.label })} />
                      <Button size="sm" variant="ghost" iconOnly icon={<StickyNote />} aria-label={`${t("runs.note")}: ${name}`} title={t("runs.note")} onClick={() => setEditing({ id: r.id, field: "note", value: r.note })} />
                      <Button
                        size="sm"
                        variant="ghost"
                        iconOnly
                        icon={r.pinned ? <PinOff /> : <Pin />}
                        aria-label={`${t(r.pinned ? "runs.unpin" : "runs.pin")}: ${name}`}
                        title={t(r.pinned ? "runs.unpin" : "runs.pinTitle")}
                        onClick={() => void pinRun(r.id, !r.pinned)}
                      />
                      <Button size="sm" variant="ghost" iconOnly icon={<Trash2 />} aria-label={`${t("runs.delete")}: ${name}`} title={t("runs.delete")} onClick={() => void deleteRun(r.id)} />
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        </div>
      )}
    </>
  );
}
