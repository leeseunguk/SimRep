import { useEffect, useRef, useState, type PointerEvent as ReactPointerEvent } from "react";
import { useResolvedTheme } from "../../state/theme";
import { drawChart, fromPx, readPlotTheme, type ChartSpec, type Frame } from "./chart";
import { CanvasPainter } from "./painters";
import { plotPerf } from "./perf";
import styles from "./Chart.module.css";

interface ChartProps {
  spec: ChartSpec;
  ariaLabel: string;
  /** Tooltip lines at data coordinates (x, y), or null for none. */
  readout?: (x: number, y: number) => string[] | null;
  /** Drag across the plot: zoom x to [lo, hi]. Double-click: null (reset). */
  onZoom?: (range: [number, number] | null) => void;
  /** Click (no drag) at data coordinates. */
  onPick?: (x: number, y: number, frame: Frame) => void;
}

const DRAG_PX = 4;

/** One chart on a hi-DPI canvas that fills its container, with hover readout and brush zoom. */
export function Chart({ spec, ariaLabel, readout, onZoom, onPick }: ChartProps) {
  const wrap = useRef<HTMLDivElement>(null);
  const canvas = useRef<HTMLCanvasElement>(null);
  const frame = useRef<Frame | null>(null);
  const [size, setSize] = useState<{ w: number; h: number } | null>(null);
  const [hover, setHover] = useState<{ x: number; y: number; lines: string[] } | null>(null);
  const [brush, setBrush] = useState<{ a: number; b: number } | null>(null);
  const drag = useRef<{ x: number; y: number; moved: boolean } | null>(null);
  const theme = useResolvedTheme();

  useEffect(() => {
    const el = wrap.current!;
    const ro = new ResizeObserver(([e]) => {
      if (!e) return;
      const w = Math.floor(e.contentRect.width);
      const h = Math.floor(e.contentRect.height);
      setSize((s) => (s && s.w === w && s.h === h ? s : { w, h }));
    });
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  // Passive effect: runs after App has applied data-theme, so the tokens read here are current.
  useEffect(() => {
    const c = canvas.current;
    if (!c || !size || size.w < 2 || size.h < 2) return;
    const dpr = devicePixelRatio;
    c.width = Math.round(size.w * dpr);
    c.height = Math.round(size.h * dpr);
    const ctx = c.getContext("2d")!;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    const t = readPlotTheme();
    frame.current = drawChart(new CanvasPainter(ctx, t), spec, size.w, size.h, t);
    plotPerf.drawn();
  }, [spec, size, theme]);

  const local = (e: { clientX: number; clientY: number }) => {
    const r = wrap.current!.getBoundingClientRect();
    return [e.clientX - r.left, e.clientY - r.top] as const;
  };
  const inside = (px: number, py: number) => {
    const f = frame.current;
    return !!f && px >= f.left && px <= f.left + f.width && py >= f.top && py <= f.top + f.height;
  };

  const onPointerDown = (e: ReactPointerEvent<HTMLDivElement>) => {
    if (e.button !== 0) return;
    const [px, py] = local(e);
    if (!inside(px, py)) return;
    e.currentTarget.setPointerCapture(e.pointerId);
    drag.current = { x: px, y: py, moved: false };
  };

  const onPointerMove = (e: ReactPointerEvent<HTMLDivElement>) => {
    const [px, py] = local(e);
    const f = frame.current;
    const d = drag.current;
    if (d) {
      if (Math.abs(px - d.x) > DRAG_PX) d.moved = true;
      if (d.moved && onZoom && f) setBrush({ a: Math.max(f.left, Math.min(d.x, px)), b: Math.min(f.left + f.width, Math.max(d.x, px)) });
    }
    if (!f || !inside(px, py) || !readout) {
      setHover(null);
      return;
    }
    const [x, y] = fromPx(f, px, py);
    const lines = readout(x, y);
    setHover(lines ? { x: px, y: py, lines } : null);
  };

  const onPointerUp = (e: ReactPointerEvent<HTMLDivElement>) => {
    const d = drag.current;
    drag.current = null;
    setBrush(null);
    const f = frame.current;
    if (!d || !f) return;
    const [px, py] = local(e);
    if (d.moved && onZoom) {
      const [a] = fromPx(f, Math.min(d.x, px), py);
      const [b] = fromPx(f, Math.max(d.x, px), py);
      if (b > a) onZoom([Math.max(a, f.x[0]), Math.min(b, f.x[1])]);
    } else if (!d.moved && onPick) {
      const [x, y] = fromPx(f, px, py);
      onPick(x, y, f);
    }
  };

  const f = frame.current;
  return (
    <div
      ref={wrap}
      className={styles.chart}
      data-zoom={!!onZoom}
      data-pick={!!onPick}
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onPointerUp={onPointerUp}
      onPointerCancel={() => {
        drag.current = null;
        setBrush(null);
      }}
      onPointerLeave={() => setHover(null)}
      onDoubleClick={() => onZoom?.(null)}
    >
      <canvas ref={canvas} className={styles.canvas} role="img" aria-label={ariaLabel} />
      {f && brush && <div className={styles.brush} style={{ left: brush.a, width: brush.b - brush.a, top: f.top, height: f.height }} />}
      {f && hover && !brush && (
        <>
          <div className={styles.crosshair} style={{ left: hover.x, top: f.top, height: f.height }} />
          <div className={styles.tooltip} style={{ left: hover.x, top: hover.y }} data-flip={hover.x > f.left + f.width * 0.6}>
            {hover.lines.map((l, i) => (
              <span key={i} className="num">
                {l}
              </span>
            ))}
          </div>
        </>
      )}
    </div>
  );
}
