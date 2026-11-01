import { Box, Grid3x3, Maximize } from "lucide-react";
import { useI18n } from "../i18n";
import type { MessageKey } from "../i18n/en";
import { useApp, type ViewPreset } from "../state/store";
import { useLayout } from "../state/layout";
import { Kbd } from "../components/ui/Kbd";
import inputs from "../components/ui/inputs.module.css";
import styles from "./ViewportToolbar.module.css";

const PRESETS: { view: ViewPreset; key: MessageKey; kbd: string }[] = [
  { view: "front", key: "viewport.front", kbd: "1" },
  { view: "right", key: "viewport.side", kbd: "2" },
  { view: "top", key: "viewport.top", kbd: "3" },
  { view: "iso", key: "viewport.iso", kbd: "4" },
];

export function ViewportToolbar({
  activePreset,
  onPreset,
  onFit,
}: {
  activePreset: ViewPreset | null;
  onPreset(view: ViewPreset): void;
  onFit(): void;
}) {
  const { t } = useI18n();
  const projection = useApp((s) => s.projection);
  const setProjection = useApp((s) => s.setProjection);
  const gridVisible = useApp((s) => s.gridVisible);
  const toggleGrid = useApp((s) => s.toggleGrid);
  const ortho = projection === "orthographic";
  const phone = useLayout((s) => s.mode === "phone");

  // Phone: one small row (view menu, fit, projection, grid) so the 3D view stays visible.
  if (phone)
    return (
      <div className={`${styles.toolbar} ${styles.compact}`} role="toolbar" aria-label={t("viewport.label")} data-inset="top" data-testid="viewport-toolbar">
        <div className={styles.group}>
          <select
            className={`${inputs.select} ${styles.viewSelect}`}
            aria-label={t("viewport.view")}
            title={t("viewport.view")}
            value={activePreset ?? ""}
            onChange={(e) => e.target.value && onPreset(e.target.value as ViewPreset)}
          >
            {activePreset === null && (
              <option value="" disabled hidden>
                {t("viewport.view")}
              </option>
            )}
            {PRESETS.map((p) => (
              <option key={p.view} value={p.view}>
                {t(p.key)}
              </option>
            ))}
          </select>
          <button type="button" className={styles.tool} aria-label={t("viewport.fit")} title={t("viewport.fit")} onClick={onFit}>
            <Maximize aria-hidden />
          </button>
          <button
            type="button"
            className={styles.tool}
            aria-pressed={ortho}
            aria-label={t("viewport.orthographic")}
            title={`${t("viewport.perspective")} / ${t("viewport.orthographic")}`}
            onClick={() => setProjection(ortho ? "perspective" : "orthographic")}
          >
            <Box aria-hidden />
          </button>
          <button type="button" className={styles.tool} aria-pressed={gridVisible} aria-label={t("viewport.grid")} title={t("viewport.grid")} onClick={toggleGrid}>
            <Grid3x3 aria-hidden />
          </button>
        </div>
      </div>
    );

  return (
    <div className={styles.toolbar} role="toolbar" aria-label={t("viewport.label")} data-inset="top">
      <div className={styles.group}>
        {PRESETS.map((p) => (
          <button
            key={p.view}
            type="button"
            className={styles.tool}
            aria-pressed={activePreset === p.view}
            title={t("viewport.viewTitle", { view: t(p.key) })}
            onClick={() => onPreset(p.view)}
          >
            {t(p.key)}
            <Kbd>{p.kbd}</Kbd>
          </button>
        ))}
      </div>
      <div className={styles.group}>
        <button type="button" className={styles.tool} title={t("viewport.fit")} onClick={onFit}>
          <Maximize aria-hidden />
          {t("viewport.fit")}
          <Kbd>F</Kbd>
        </button>
      </div>
      <div className={styles.group}>
        <button
          type="button"
          className={styles.tool}
          aria-pressed={ortho}
          title={`${t("viewport.perspective")} / ${t("viewport.orthographic")}`}
          onClick={() => setProjection(ortho ? "perspective" : "orthographic")}
        >
          <Box aria-hidden />
          {t(ortho ? "viewport.orthographic" : "viewport.perspective")}
          <Kbd>P</Kbd>
        </button>
        <button type="button" className={styles.tool} aria-pressed={gridVisible} aria-label={t("viewport.grid")} title={t("viewport.grid")} onClick={toggleGrid}>
          <Grid3x3 aria-hidden />
          <Kbd>G</Kbd>
        </button>
      </div>
    </div>
  );
}
