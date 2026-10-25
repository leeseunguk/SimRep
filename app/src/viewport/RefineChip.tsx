import type { ReactNode } from "react";
import { Loader2, X } from "lucide-react";
import { useI18n } from "../i18n";
import { cancelRefine } from "../scene/simulation";
import { useApp, type PaObject, type SceneObject } from "../state/store";
import { refineProgress, stageLabel } from "../components/simulationFormat";
import styles from "./Viewport.module.css";

const refining = (o: SceneObject | undefined): o is PaObject => o?.kind === "pa" && (o.refine?.status === "running" || o.refine?.status === "starting");

/**
 * While an array is refining, its progress (and Cancel) takes the place of the mouse hints at the
 * bottom of the view, so the converging section stays unobstructed.
 */
export function RefineChip({ children }: { children: ReactNode }) {
  const { t, tm } = useI18n();
  const o = useApp((s) => {
    const selected = s.objects.find((x) => x.id === s.selectedId);
    return refining(selected) ? selected : (s.objects.find(refining) ?? null);
  });
  if (!o?.refine) return <>{children}</>;
  const r = o.refine;
  const progress = refineProgress(r);
  return (
    <div className={styles.chip} data-inset="bottom" role="status" aria-live="polite">
      <Loader2 className={styles.spin} aria-hidden />
      <span className={styles.chipTitle} title={t("refine.chip.title", { file: o.fileName })}>
        {t("refine.chip.title", { file: o.fileName })}
      </span>
      {r.stages.length > 0 && <span className={styles.chipMeta}>{tm(stageLabel(r))}</span>}
      <span className={`${styles.chipMeta} tnum`}>
        {t("refine.cycle", { n: r.cycle })} · {r.residual === null ? "…" : r.residual.toExponential(1)}
      </span>
      <span className={styles.chipBar} aria-hidden>
        <span style={{ width: `${Math.round(progress * 100)}%` }} />
      </span>
      <button type="button" className={styles.chipCancel} onClick={() => cancelRefine(o.id)} title={t("refine.cancel")}>
        <X aria-hidden />
        {t("refine.cancel")}
      </button>
    </div>
  );
}
