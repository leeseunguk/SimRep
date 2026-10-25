import { useI18n } from "../i18n";
import { contourStep } from "../scene/pa";
import { objectName } from "../scene/names";
import { useApp, type PaObject, type SceneObject } from "../state/store";
import styles from "./Viewport.module.css";

const shownSection = (o: SceneObject | undefined): o is PaObject => o?.kind === "pa" && o.visible && o.slice.visible;

/** Colour bar for the section of the selected (else first visible) potential array. */
export function SliceLegend() {
  const { t, num } = useI18n();
  const o = useApp((s) => {
    const selected = s.objects.find((x) => x.id === s.selectedId);
    return shownSection(selected) ? selected : (s.objects.find(shownSection) ?? null);
  });
  if (!o) return null;
  const { potentialMin: min, potentialMax: max } = o.info;
  const range = Math.max(max - min, 1e-9);
  const digits = Math.min(4, Math.max(0, 2 - Math.floor(Math.log10(range))));
  // A uniform field (e.g. every electrode at 0 V) has one value, not a scale.
  const ticks = max > min ? [max, (min + max) / 2, min] : [max];
  const step = contourStep(min, max, o.slice.density);
  return (
    <div className={styles.legend} aria-label={t("viewport.legend.title")} data-inset="right">
      <p className={styles.legendTitle}>
        {t("viewport.legend.title")} <span className={styles.legendUnit}>({t("units.V")})</span>
      </p>
      <p className={styles.legendName} title={objectName(o)}>
        {objectName(o)}
      </p>
      <div className={styles.legendScale}>
        <div className={styles.legendBar} />
        <div className={styles.legendTicks}>
          {ticks.map((v, i) => (
            <span key={i} className="num">
              {num(Number(v.toFixed(digits)), digits)}
            </span>
          ))}
        </div>
      </div>
      {o.slice.contours && step > 0 && (
        <p className={styles.legendItem}>
          <i className={styles.legendLine} aria-hidden />
          {t("viewport.legend.lines", { step })}
        </p>
      )}
      <p className={styles.legendItem}>
        <i className={styles.legendElectrode} aria-hidden />
        {t("viewport.legend.electrode")}
      </p>
    </div>
  );
}
