import { AppWindow, BarChart3, Cpu, Globe, ScrollText, Zap } from "lucide-react";
import { host } from "../host";
import { useI18n } from "../i18n";
import { sceneTriangles, useApp } from "../state/store";
import { useResults } from "../results/store";
import { rendererName } from "../viewport/rendererBase";
import styles from "./StatusBar.module.css";

export function StatusBar() {
  const { t, tm, num } = useI18n();
  const gpu = useApp((s) => s.gpu);
  const compute = useApp((s) => s.compute);
  const runtime = useApp((s) => s.runtime);
  const objects = useApp((s) => s.objects);
  const logOpen = useApp((s) => s.logOpen);
  const setLogOpen = useApp((s) => s.setLogOpen);
  const logCount = useApp((s) => s.logs.length);
  const hasErrors = useApp((s) => s.logs.some((l) => l.level === "error"));
  const resultsOpen = useResults((s) => s.open);
  const flown = objects.some((o) => o.kind === "pa" && o.flight.result !== null);
  const triangles = sceneTriangles(objects, true);
  const ions = objects.reduce((n, o) => n + (o.kind === "pa" && o.visible && o.showTrajectories && o.flight.result ? o.flight.result.header.ions : 0), 0);
  const computeText =
    compute.state === "gpu" ? t("status.compute.gpu", { name: compute.name }) : compute.state === "cpu" ? t("status.compute.cpu") : t("status.compute.pending");
  const computeTitle =
    compute.state === "gpu" ? t("status.compute.gpuTitle", { name: compute.name, ms: Math.round(compute.initMs) }) : compute.state === "cpu" ? tm(compute.reason) : computeText;

  const gpuText =
    gpu.state === "ready"
      ? t("status.renderer.ready", { api: rendererName(gpu.api) })
      : gpu.state === "pending"
        ? t("status.webgpu.pending")
        : gpu.state === "lost"
          ? t("status.renderer.lost", { api: rendererName(gpu.api) })
          : t("status.webgpu.unavailable");
  const HostItems = host()?.StatusItems;
  const gpuTitle =
    gpu.state === "unavailable" || gpu.state === "lost"
      ? gpu.reason
      : gpu.state === "ready"
        ? gpu.fallbackReason
          ? t("status.renderer.fallbackTitle", { adapter: gpu.adapter, reason: gpu.fallbackReason })
          : t("status.renderer.title", { api: rendererName(gpu.api), adapter: gpu.adapter })
        : gpuText;

  return (
    <footer className={styles.statusbar}>
      <span className={styles.item} title={gpuTitle} data-testid="status-renderer" data-renderer={gpu.state === "ready" ? gpu.api : undefined}>
        <span className={styles.dot} data-state={gpu.state} />
        {gpuText}
      </span>
      {HostItems ? (
        <HostItems />
      ) : runtime.kind === "tauri" && (
        <span className={styles.item} title={computeTitle} data-compute={compute.state}>
          {compute.state === "gpu" ? <Zap aria-hidden /> : <Cpu aria-hidden />}
          {computeText}
        </span>
      )}
      {!HostItems && (
        <span className={styles.item}>
          {runtime.kind === "tauri" ? <AppWindow aria-hidden /> : <Globe aria-hidden />}
          {runtime.kind === "tauri"
            ? t("status.runtime.tauri", { version: runtime.version ?? "…" })
            : t("status.runtime.browser")}
        </span>
      )}
      <span className={styles.spacer} />
      <span className={styles.item}>{t("status.objects", { count: objects.length })}</span>
      <span className={`${styles.item} tnum`}>{t("status.triangles", { count: triangles })}</span>
      {ions > 0 && <span className={`${styles.item} tnum`}>{t("status.trajectories", { count: ions })}</span>}
      <span className={styles.item}>{t("status.unitsMm")}</span>
      {flown && (
        <button
          type="button"
          className={styles.logButton}
          aria-pressed={resultsOpen}
          title={t("status.resultsTitle")}
          onClick={() => useResults.getState().setOpen(!resultsOpen)}
        >
          <BarChart3 aria-hidden />
          {t("status.results")}
        </button>
      )}
      <button
        type="button"
        className={styles.logButton}
        aria-pressed={logOpen}
        aria-expanded={logOpen}
        title={t("status.logTitle")}
        onClick={() => setLogOpen(!logOpen)}
      >
        <ScrollText aria-hidden />
        {t("status.log")}
        <span className={styles.logCount} data-error={hasErrors}>
          {num(logCount)}
        </span>
      </button>
    </footer>
  );
}
