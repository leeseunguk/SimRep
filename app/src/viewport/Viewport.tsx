import { useCallback, useEffect, useRef, useState, type PointerEvent as ReactPointerEvent } from "react";
import { Loader2 } from "lucide-react";
import { useI18n } from "../i18n";
import { paKind } from "../scene/pa";
import { isRenderable, objectScale, useApp, type PaObject, type ViewPreset } from "../state/store";
import { AxesGizmo } from "./AxesGizmo";
import { BenchHandles } from "./BenchHandles";
import { PRESET_ANGLES } from "./camera";
import { readViewportTheme, type ViewportTheme } from "./color";
import { pick } from "./picking";
import type { PaDrawItem, ProbeHit } from "./paLayer";
import { rendererName } from "./rendererBase";
import { createRenderer, RendererCancelledError, RendererUnavailableError, type DrawItem, type Insets, type Renderer } from "./renderer";
import { RefineChip } from "./RefineChip";
import { SliceLegend } from "./SliceLegend";
import type { Highlight, TrajectoryItem } from "./trajectoryLayer";
import { useResults } from "../results/store";
import { useWizard } from "../state/wizard";
import type { Bounds } from "../scene/mesh";
import { registerCamera, registerCapture } from "./viewState";
import { ViewportToolbar } from "./ViewportToolbar";
import { WebGpuFallback } from "./WebGpuFallback";
import styles from "./Viewport.module.css";

type ViewportState =
  | { kind: "loading" }
  | { kind: "ready" }
  | { kind: "unavailable"; detail: string }
  | { kind: "lost"; detail: string };

const CLICK_SLOP_PX = 4;
/** How close (CSS px) a click must be to an ion's end point or drawn path to pick it. */
const ION_PICK_PX = 6;

function buildDrawItems(theme: ViewportTheme): DrawItem[] {
  const { objects, selectedId } = useApp.getState();
  const items: DrawItem[] = objects
    .filter(isRenderable)
    .map((o) => ({ id: o.id, mesh: o.mesh, color: theme.meshImported, visible: o.visible, selected: o.id === selectedId, scale: objectScale(o) }));
  // Import wizard preview: one colour per electrode number, until the built array replaces it.
  const w = useWizard.getState().wizard;
  if (w && !w.objectId)
    w.meshes.forEach((mesh, i) => {
      const number = w.parts[i]?.number ?? i + 1;
      items.push({ id: `wizard:${i}`, mesh, color: theme.parts[(number - 1) % theme.parts.length]!, visible: true, selected: w.highlight === i, scale: 1 });
    });
  return items;
}

/** Field-free workbench boxes of visible arrays that fly with one. */
function buildBenchBoxes(): Bounds[] {
  return useApp
    .getState()
    .objects.filter((o): o is PaObject => o.kind === "pa" && o.visible && o.source.bench.enabled)
    .map((o) => ({ min: [...o.source.bench.min], max: [...o.source.bench.max] }));
}

function buildPaItems(): PaDrawItem[] {
  const { objects, selectedId, selectedElectrode } = useApp.getState();
  return objects
    .filter((o): o is PaObject => o.kind === "pa")
    .map((o) => ({
      id: o.id,
      info: o.info,
      grid: o.grid,
      surface: o.surface,
      visible: o.visible,
      selected: o.id === selectedId && selectedElectrode === null,
      selectedElectrode: o.id === selectedId ? selectedElectrode : null,
      hiddenElectrodes: o.hiddenElectrodes,
      labels: o.electrodeSet?.labels ?? null,
      showMirrors: o.showMirrors,
      slice: o.slice,
      placement: o.source.bench.placement ?? null,
    }));
}

function buildTrajectoryItems(): TrajectoryItem[] {
  return useApp
    .getState()
    .objects.filter((o): o is PaObject => o.kind === "pa" && o.flight.result !== null)
    .map((o) => ({ id: o.id, flight: o.flight.result!, visible: o.visible && o.showTrajectories }));
}

/** The ion selected in the results panel: its re-traced path when available, else its drawn one. */
function buildHighlight(): Highlight | null {
  const { ion, trace } = useResults.getState();
  if (!ion) return null;
  const o = useApp.getState().objects.find((x) => x.id === ion.objectId);
  const f = o?.kind === "pa" && o.visible ? o.flight.result : null;
  if (!f || ion.index >= f.header.ions) return null;
  const i = ion.index;
  if (trace && trace.objectId === ion.objectId && trace.index === i && trace.data.t.length > 1) {
    const d = trace.data;
    const n = d.t.length;
    const points = new Float32Array(n * 3);
    for (let k = 0; k < n; k++) {
      points[k * 3] = d.x[k]!;
      points[k * 3 + 1] = d.y[k]!;
      points[k * 3 + 2] = d.z[k]!;
    }
    return { points, end: [d.x[n - 1]!, d.y[n - 1]!, d.z[n - 1]!] };
  }
  const a = f.offsets[i]!;
  const b = f.offsets[i + 1]!;
  return { points: b - a > 1 ? f.points.slice(a * 3, b * 3) : null, end: [f.ends[i * 4]!, f.ends[i * 4 + 1]!, f.ends[i * 4 + 2]!] };
}

/** Gap kept between an overlay and the framed scene, CSS px. */
const INSET_GAP = 12;

/** Space taken by overlays marked data-inset="top|right|bottom|left" (CSS px from each edge). */
function measureInsets(section: HTMLElement): Insets {
  const box = section.getBoundingClientRect();
  const insets: Insets = { left: 0, right: 0, top: 0, bottom: 0 };
  section.querySelectorAll<HTMLElement>("[data-inset]").forEach((el) => {
    const r = el.getBoundingClientRect();
    if (r.width === 0 || r.height === 0) return;
    const side = el.dataset.inset as keyof Insets;
    const size =
      side === "top" ? r.bottom - box.top : side === "bottom" ? box.bottom - r.top : side === "left" ? r.right - box.left : box.right - r.left;
    insets[side] = Math.max(insets[side], size + INSET_GAP);
  });
  return insets;
}

/** Decimal places that resolve a tenth of the finest step (spacing in mm, or a potential range). */
function digitsFor(scale: number): number {
  return Math.min(4, Math.max(0, 1 - Math.floor(Math.log10(Math.max(scale, 1e-9)))));
}

function isTypingTarget(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  return target.isContentEditable || ["INPUT", "TEXTAREA", "SELECT"].includes(target.tagName);
}

export function Viewport({ theme }: { theme: "light" | "dark" }) {
  const { t } = useI18n();
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const sectionRef = useRef<HTMLElement>(null);
  const rendererRef = useRef<Renderer | null>(null);
  const themeRef = useRef<ViewportTheme | null>(null);
  const [state, setState] = useState<ViewportState>({ kind: "loading" });
  const [activePreset, setActivePreset] = useState<ViewPreset | null>("iso");
  const [hover, setHover] = useState<{ x: number; y: number; flipX: boolean; flipY: boolean; hit: ProbeHit } | null>(null);
  const projection = useApp((s) => s.projection);
  const gridVisible = useApp((s) => s.gridVisible);
  const drag = useRef<{ x: number; y: number; startX: number; startY: number; mode: "orbit" | "pan"; moved: boolean } | null>(null);

  const sync = useCallback(() => {
    const r = rendererRef.current;
    if (r && themeRef.current) {
      r.setItems(buildDrawItems(themeRef.current));
      r.setPaItems(buildPaItems());
      r.setBenchBoxes(buildBenchBoxes());
      r.setTrajectories(buildTrajectoryItems());
      r.setHighlight(buildHighlight());
    }
  }, []);

  const fit = useCallback((ids?: string[], animate = true) => {
    const r = rendererRef.current;
    if (!r) return;
    const bounds = r.bounds(ids);
    if (!bounds) return;
    const target = r.camera.fitState(bounds, r.aspect(), r.safe);
    if (animate) r.camera.animateTo(target);
    else r.camera.jumpTo(target);
    r.requestRender();
  }, []);

  const applyPreset = useCallback((view: ViewPreset, withFit = false) => {
    const r = rendererRef.current;
    if (!r) return;
    const bounds = withFit ? r.bounds() : null;
    r.camera.animateTo({ ...PRESET_ANGLES[view], ...(bounds && r.camera.fitState(bounds, r.aspect(), r.safe)) });
    r.requestRender();
    setActivePreset(view);
  }, []);

  // Create the renderer once.
  useEffect(() => {
    const canvas = canvasRef.current!;
    let disposed = false;
    let observer: ResizeObserver | null = null;
    let unregisterCamera: (() => void) | null = null;
    let unregisterCapture: (() => void) | null = null;
    const { setGpu, log } = useApp.getState();

    createRenderer(canvas, () => disposed).then(
      ({ renderer: r, webgpuReason }) => {
        if (disposed) {
          r.destroy();
          return;
        }
        rendererRef.current = r;
        r.onDeviceLost = (reason) => {
          setState({ kind: "lost", detail: reason });
          useApp.getState().setGpu({ state: "lost", api: r.api, reason });
          useApp.getState().log("error", { key: "log.rendererLost", params: { api: rendererName(r.api), reason } });
        };
        const resize = (w: number, h: number) => {
          canvas.width = Math.max(1, Math.min(r.maxTextureSize, Math.round(w)));
          canvas.height = Math.max(1, Math.min(r.maxTextureSize, Math.round(h)));
          r.requestRender();
        };
        const rect = canvas.getBoundingClientRect();
        resize(rect.width * devicePixelRatio, rect.height * devicePixelRatio);
        observer = new ResizeObserver(([entry]) => {
          if (!entry) return;
          const box = entry.devicePixelContentBoxSize?.[0];
          if (box) resize(box.inlineSize, box.blockSize);
          else resize(entry.contentRect.width * devicePixelRatio, entry.contentRect.height * devicePixelRatio);
        });
        observer.observe(canvas);

        themeRef.current = readViewportTheme();
        r.setTheme(themeRef.current);
        r.camera.orthographic = useApp.getState().projection === "orthographic";
        r.gridVisible = useApp.getState().gridVisible;
        r.setItems(buildDrawItems(themeRef.current));
        r.setPaItems(buildPaItems());
        r.setBenchBoxes(buildBenchBoxes());
        r.setTrajectories(buildTrajectoryItems());
        r.setHighlight(buildHighlight());
        fit(undefined, false);
        unregisterCamera = registerCamera(() => r.camera.state());
        unregisterCapture = registerCapture(() => {
          const css = getComputedStyle(document.documentElement);
          return r.capture({ center: css.getPropertyValue("--vp-bg-center").trim(), edge: css.getPropertyValue("--vp-bg-edge").trim() });
        });
        setState({ kind: "ready" });
        setGpu({ state: "ready", api: r.api, adapter: r.adapterLabel, fallbackReason: webgpuReason });
        // WebGL2 draws the same passes; the log keeps why WebGPU was not used.
        if (webgpuReason) log("warning", { key: "log.webgpuFallback", params: { reason: webgpuReason } });
        log("success", { key: "log.rendererReady", params: { api: rendererName(r.api), adapter: r.adapterLabel } });
      },
      (err: unknown) => {
        if (disposed || err instanceof RendererCancelledError) return;
        const detail = err instanceof Error ? err.message : String(err);
        setState({ kind: "unavailable", detail });
        setGpu({ state: "unavailable", reason: detail });
        log(err instanceof RendererUnavailableError ? "warning" : "error", {
          key: "log.webgpuUnavailable",
          params: { reason: detail },
        });
      },
    );
    return () => {
      disposed = true;
      observer?.disconnect();
      unregisterCamera?.();
      unregisterCapture?.();
      rendererRef.current?.destroy();
      rendererRef.current = null;
    };
  }, [fit]);

  // Overlays (toolbar, legend, hints, progress) reserve their edges: the scene is framed in the
  // remaining area, so "fit" never puts the array under the legend.
  useEffect(() => {
    const section = sectionRef.current;
    if (state.kind !== "ready" || !section) return;
    const update = () => {
      const r = rendererRef.current;
      const box = section.getBoundingClientRect();
      r?.setInsets(measureInsets(section), box.width, box.height);
    };
    const resize = new ResizeObserver(update);
    const observeAll = () => {
      resize.disconnect();
      resize.observe(section);
      section.querySelectorAll("[data-inset]").forEach((el) => resize.observe(el));
      update();
    };
    const mutations = new MutationObserver(observeAll);
    mutations.observe(section, { childList: true, subtree: true });
    observeAll();
    return () => {
      resize.disconnect();
      mutations.disconnect();
    };
  }, [state.kind]);

  // Scene and selection changes -> GPU.
  useEffect(() => useApp.subscribe((s, prev) => {
    if (s.objects !== prev.objects || s.selectedId !== prev.selectedId || s.selectedElectrode !== prev.selectedElectrode) sync();
  }), [sync]);

  // Import wizard preview (parts, colours, highlighted row).
  useEffect(() => useWizard.subscribe((s, prev) => {
    if (s.wizard?.meshes !== prev.wizard?.meshes || s.wizard?.parts !== prev.wizard?.parts || s.wizard?.highlight !== prev.wizard?.highlight || s.wizard?.objectId !== prev.wizard?.objectId || !s.wizard !== !prev.wizard) sync();
  }), [sync]);

  // The ion selected in the results panel (and its re-traced path).
  useEffect(() => useResults.subscribe((s, prev) => {
    if (s.ion !== prev.ion || s.trace !== prev.trace) rendererRef.current?.setHighlight(buildHighlight());
  }), []);

  // Theme switch: colours come from CSS tokens, re-read after data-theme changes.
  useEffect(() => {
    const r = rendererRef.current;
    if (!r) return;
    themeRef.current = readViewportTheme();
    r.setTheme(themeRef.current);
    sync();
  }, [theme, sync, state.kind]);

  useEffect(() => {
    const r = rendererRef.current;
    if (!r) return;
    r.camera.orthographic = projection === "orthographic";
    r.gridVisible = gridVisible;
    r.requestRender();
  }, [projection, gridVisible, state.kind]);

  // Imperative camera commands from elsewhere in the UI (tree double-click, import, properties).
  useEffect(() => useApp.subscribe((s, prev) => {
    if (!s.viewCommand || s.viewCommand === prev.viewCommand) return;
    const { cmd } = s.viewCommand;
    if (cmd.type === "fit") fit(cmd.ids);
    else if (cmd.type === "preset") applyPreset(cmd.view, cmd.fit);
    else {
      const r = rendererRef.current;
      r?.camera.jumpTo(cmd.state);
      r?.requestRender();
      setActivePreset(null);
    }
  }), [fit, applyPreset]);

  // Keyboard shortcuts (CAD-style view presets).
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.ctrlKey || e.metaKey || e.altKey || isTypingTarget(e.target)) return;
      const app = useApp.getState();
      const presets: Record<string, ViewPreset> = { "1": "front", "2": "right", "3": "top", "4": "iso", "0": "iso" };
      const preset = presets[e.key];
      if (preset) applyPreset(preset);
      else if (e.key === "f" || e.key === "F") fit();
      else if (e.key === "p" || e.key === "P")
        app.setProjection(app.projection === "perspective" ? "orthographic" : "perspective");
      else if (e.key === "g" || e.key === "G") app.toggleGrid();
      else return;
      e.preventDefault();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [applyPreset, fit]);

  // Wheel zoom towards the cursor (needs a non-passive listener to stop page scrolling).
  useEffect(() => {
    const canvas = canvasRef.current!;
    const onWheel = (e: WheelEvent) => {
      const r = rendererRef.current;
      if (!r) return;
      e.preventDefault();
      const rect = canvas.getBoundingClientRect();
      const ndcX = ((e.clientX - rect.left) / rect.width) * 2 - 1;
      const ndcY = 1 - ((e.clientY - rect.top) / rect.height) * 2;
      const lines = e.deltaMode === 1 ? 16 : e.deltaMode === 2 ? rect.height : 1;
      r.camera.zoomAt(Math.exp(e.deltaY * lines * 0.0015), ndcX, ndcY, r.aspect());
      r.requestRender();
    };
    canvas.addEventListener("wheel", onWheel, { passive: false });
    return () => canvas.removeEventListener("wheel", onWheel);
  }, []);

  const toNdc = (e: { clientX: number; clientY: number }) => {
    const rect = canvasRef.current!.getBoundingClientRect();
    return [((e.clientX - rect.left) / rect.width) * 2 - 1, 1 - ((e.clientY - rect.top) / rect.height) * 2] as const;
  };

  const pickAt = (e: { clientX: number; clientY: number }): string | null => {
    const r = rendererRef.current;
    if (!r) return null;
    const [x, y] = toNdc(e);
    const ray = r.camera.ray(x, y, r.aspect());
    return pick(r.drawItems(), ray.origin, ray.dir, r.paPickables());
  };

  const onPointerDown = (e: ReactPointerEvent<HTMLCanvasElement>) => {
    if (!rendererRef.current) return;
    e.currentTarget.setPointerCapture(e.pointerId);
    e.currentTarget.focus();
    const pan = e.button === 1 || e.button === 2 || (e.button === 0 && e.shiftKey);
    drag.current = { x: e.clientX, y: e.clientY, startX: e.clientX, startY: e.clientY, mode: pan ? "pan" : "orbit", moved: false };
  };

  const onPointerMove = (e: ReactPointerEvent<HTMLCanvasElement>) => {
    const d = drag.current;
    const r = rendererRef.current;
    if (r && !d) {
      const [nx, ny] = toNdc(e);
      const hit = r.probe(nx, ny);
      const rect = e.currentTarget.getBoundingClientRect();
      const css = getComputedStyle(e.currentTarget);
      const x = e.clientX - rect.left;
      const y = e.clientY - rect.top;
      const flipX = x > rect.width - parseFloat(css.getPropertyValue("--readout-w"));
      const flipY = y > rect.height - parseFloat(css.getPropertyValue("--readout-h"));
      setHover(hit ? { x, y, flipX, flipY, hit } : null);
    }
    if (!d || !r) return;
    if (hover) setHover(null);
    const dx = e.clientX - d.x;
    const dy = e.clientY - d.y;
    d.x = e.clientX;
    d.y = e.clientY;
    if (!d.moved && Math.hypot(e.clientX - d.startX, e.clientY - d.startY) < CLICK_SLOP_PX) return;
    d.moved = true;
    if (d.mode === "orbit") {
      r.camera.orbit(dx, dy);
      setActivePreset(null);
    } else {
      r.camera.pan(dx, dy, e.currentTarget.getBoundingClientRect().height);
    }
    r.requestRender();
  };

  const onPointerUp = (e: ReactPointerEvent<HTMLCanvasElement>) => {
    const d = drag.current;
    drag.current = null;
    if (!d || d.moved || e.button !== 0) return;
    // An ion (end point or drawn path) wins over the array it flew through: open its plots.
    const r = rendererRef.current;
    const rect = e.currentTarget.getBoundingClientRect();
    const hit = r?.pickIon(e.clientX - rect.left, e.clientY - rect.top, ION_PICK_PX);
    const results = useResults.getState();
    if (hit) {
      results.selectIon({ objectId: hit.id, index: hit.index });
      if (!results.open || results.tab === "hist" || results.tab === "map" || results.tab === "compare") results.setTab("ion");
      return;
    }
    const id = pickAt(e);
    if (id?.startsWith("wizard:")) {
      useWizard.getState().set({ highlight: Number(id.slice(7)) });
      return;
    }
    if (!id) results.selectIon(null);
    useApp.getState().select(id);
  };

  const onDoubleClick = (e: React.MouseEvent<HTMLCanvasElement>) => {
    const id = pickAt(e);
    if (id && !id.startsWith("wizard:")) useApp.getState().select(id);
    fit(id ? [id] : undefined);
  };

  return (
    <section ref={sectionRef} className={styles.viewport} aria-label={t("viewport.label")}>
      <canvas
        ref={canvasRef}
        className={styles.canvas}
        tabIndex={0}
        onPointerDown={onPointerDown}
        onPointerMove={onPointerMove}
        onPointerUp={onPointerUp}
        onPointerCancel={() => (drag.current = null)}
        onPointerLeave={() => setHover(null)}
        onDoubleClick={onDoubleClick}
        onContextMenu={(e) => e.preventDefault()}
      />
      {state.kind === "ready" && (
        <>
          <ViewportToolbar activePreset={activePreset} onPreset={applyPreset} onFit={() => fit()} />
          <AxesGizmo renderer={rendererRef.current!} onPreset={applyPreset} />
          <SliceLegend />
          {hover && <Readout {...hover} />}
          <BenchHandles renderer={rendererRef.current!} />
          <RefineChip>
          <div className={styles.hint} aria-hidden data-inset="bottom">
            <span><b>{t("viewport.hint.leftDrag")}</b> {t("viewport.hint.orbit")}</span>
            <span><b>{t("viewport.hint.rightDrag")}</b> {t("viewport.hint.pan")}</span>
            <span><b>{t("viewport.hint.wheel")}</b> {t("viewport.hint.zoom")}</span>
            <span><b>{t("viewport.hint.doubleClick")}</b> {t("viewport.hint.focus")}</span>
          </div>
          </RefineChip>
        </>
      )}
      {state.kind === "loading" && (
        <div className={styles.loading}>
          <Loader2 className={styles.spin} aria-hidden />
          {t("viewport.loading")}
        </div>
      )}
      {(state.kind === "unavailable" || state.kind === "lost") && (
        <WebGpuFallback lost={state.kind === "lost"} detail={state.detail} />
      )}
    </section>
  );
}

/** Position (mm) and potential (V) under the cursor on a section. */
function Readout({ x, y, flipX, flipY, hit }: { x: number; y: number; flipX: boolean; flipY: boolean; hit: ProbeHit }) {
  const { t, num } = useI18n();
  const o = useApp((s) => s.objects.find((obj) => obj.id === hit.id));
  if (!o || o.kind !== "pa") return null;
  const { info } = o;
  const pd = digitsFor(Math.min(info.dx, info.dy, info.dz));
  const vd = digitsFor((info.potentialMax - info.potentialMin) / 1000);
  const [px, py, pz] = hit.point;
  const coords: [string, number][] =
    paKind(info) === "cylindrical" ? [["x", px], ["r", Math.hypot(py, pz)]] : paKind(info) === "planar2d" ? [["x", px], ["y", py]] : [["x", px], ["y", py], ["z", pz]];
  const numbered = o.formatId === "paHash" && hit.electrode && !o.refine?.original;
  return (
    <div className={styles.readout} style={{ left: x, top: y }} data-flip-x={flipX} data-flip-y={flipY} aria-live="off">
      <div className={styles.readoutCoords}>
        {coords.map(([axis, value]) => (
          <span key={axis}>
            <b>{axis}</b> <span className="num">{num(value, pd)}</span>
          </span>
        ))}
        <span className={styles.readoutUnit}>mm</span>
      </div>
      <div className={styles.readoutValue} data-electrode={hit.electrode}>
        <span className={styles.readoutLabel}>{t(hit.electrode ? "viewport.readout.electrode" : "viewport.readout.field")}</span>
        <span className="num">
          {numbered ? t("viewport.readout.electrodeNumber", { n: Math.round(hit.value) }) : `${num(hit.value, vd)} ${t("units.V")}`}
        </span>
      </div>
    </div>
  );
}
