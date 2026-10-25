import { useEffect, useRef, useState, type PointerEvent as ReactPointerEvent } from "react";
import { useI18n } from "../i18n";
import type { Vec3 } from "../scene/mesh";
import { updateSource } from "../scene/simulation";
import { useApp, type PaObject } from "../state/store";
import type { Renderer } from "./renderer";
import styles from "./Viewport.module.css";

/** The six faces of the box: axis and side (0 = min, 1 = max). */
const FACES = [0, 1, 2].flatMap((axis) => [0, 1].map((side) => ({ axis, side, label: `${side ? "+" : "-"}${"xyz"[axis]}` })));

/** Smallest box size along an axis (mm) a drag can leave. */
const MIN_SIZE = 1e-3;

function faceCenter(min: Vec3, max: Vec3, axis: number, side: number): Vec3 {
  const p = [0, 1, 2].map((a) => (min[a]! + max[a]!) / 2) as Vec3;
  p[axis] = side ? max[axis]! : min[axis]!;
  return p;
}

/** A round step for dragged values: about 1/200 of the box, on a 1-2-5 scale. */
function niceStep(size: number): number {
  const raw = Math.max(size, MIN_SIZE) / 200;
  const p = 10 ** Math.floor(Math.log10(raw));
  const m = raw / p;
  return (m >= 5 ? 5 : m >= 2 ? 2 : 1) * p;
}

interface Drag {
  pointer: number;
  axis: number;
  side: number;
  start: number;
  startX: number;
  startY: number;
  /** Screen direction of +axis (unit) and CSS px per mm along it. */
  dir: [number, number];
  pxPerMm: number;
  step: number;
}

/**
 * Drag handles on the faces of the selected array's workbench box: dragging a handle moves that
 * face along its axis (the box in the properties panel follows, ions re-fly). Positions follow the
 * camera through the renderer's frame listener, without React renders per frame.
 */
export function BenchHandles({ renderer }: { renderer: Renderer }) {
  const { t, num } = useI18n();
  const o = useApp((s) => {
    const x = s.objects.find((obj) => obj.id === s.selectedId);
    return x?.kind === "pa" && x.visible && x.source.bench.enabled ? x : null;
  });
  const refs = useRef<(HTMLButtonElement | null)[]>([]);
  const drag = useRef<Drag | null>(null);
  const [active, setActive] = useState<{ face: number; value: number; digits: number } | null>(null);
  const bench = o?.source.bench;

  useEffect(() => {
    if (!bench) return;
    const place = () => {
      const { width, height } = renderer.cssSize();
      FACES.forEach(({ axis, side }, i) => {
        const el = refs.current[i];
        if (!el) return;
        const p = renderer.project(faceCenter(bench.min, bench.max, axis, side));
        const inside = p !== null && p[0] >= 0 && p[1] >= 0 && p[0] <= width && p[1] <= height;
        el.style.visibility = inside ? "visible" : "hidden";
        // `translate` (not `transform`): the hover `scale` then grows the dot in place.
        if (p) el.style.translate = `${p[0]}px ${p[1]}px`;
      });
    };
    place();
    return renderer.addFrameListener(place);
  }, [renderer, bench]);

  if (!o || !bench) return null;

  const onDown = (i: number, e: ReactPointerEvent<HTMLButtonElement>) => {
    if (e.button !== 0) return;
    const { axis, side } = FACES[i]!;
    const at = faceCenter(bench.min, bench.max, axis, side);
    const size = bench.max[axis]! - bench.min[axis]!;
    const ahead = [...at] as Vec3;
    ahead[axis] = at[axis]! + Math.max(size, MIN_SIZE);
    const p0 = renderer.project(at);
    const p1 = renderer.project(ahead);
    if (!p0 || !p1) return;
    const len = Math.hypot(p1[0] - p0[0], p1[1] - p0[1]);
    // Looking straight along the axis: the face cannot be dragged from here.
    if (len < 4) return;
    e.preventDefault();
    e.stopPropagation();
    e.currentTarget.setPointerCapture(e.pointerId);
    drag.current = {
      pointer: e.pointerId,
      axis,
      side,
      start: at[axis]!,
      startX: e.clientX,
      startY: e.clientY,
      dir: [(p1[0] - p0[0]) / len, (p1[1] - p0[1]) / len],
      pxPerMm: len / Math.max(size, MIN_SIZE),
      step: niceStep(size),
    };
    setActive({ face: i, value: at[axis]!, digits: Math.max(0, -Math.floor(Math.log10(drag.current.step))) });
  };

  const onMove = (i: number, e: ReactPointerEvent<HTMLButtonElement>) => {
    const d = drag.current;
    if (!d || d.pointer !== e.pointerId) return;
    const along = (e.clientX - d.startX) * d.dir[0] + (e.clientY - d.startY) * d.dir[1];
    let value = Math.round((d.start + along / d.pxPerMm) / d.step) * d.step;
    value = Number(value.toPrecision(10));
    const current = useApp.getState().objects.find((x): x is PaObject => x.id === o.id && x.kind === "pa")?.source.bench;
    if (!current) return;
    // The face may not cross the opposite one.
    if (d.side) value = Math.max(value, current.min[d.axis]! + MIN_SIZE);
    else value = Math.min(value, current.max[d.axis]! - MIN_SIZE);
    const key = d.side ? "max" : "min";
    if (current[key][d.axis] === value) return;
    const next = [...current[key]] as Vec3;
    next[d.axis] = value;
    updateSource(o.id, { bench: { ...current, [key]: next } });
    setActive({ face: i, value, digits: Math.max(0, -Math.floor(Math.log10(d.step))) });
  };

  const onUp = (e: ReactPointerEvent<HTMLButtonElement>) => {
    if (drag.current?.pointer !== e.pointerId) return;
    drag.current = null;
    setActive(null);
  };

  return (
    <>
      {FACES.map((f, i) => (
        <button
          key={f.label}
          ref={(el) => {
            refs.current[i] = el;
          }}
          type="button"
          className={styles.benchHandle}
          data-active={active?.face === i}
          aria-label={t("bench.handle", { face: f.label })}
          title={t("bench.handle", { face: f.label })}
          onPointerDown={(e) => onDown(i, e)}
          onPointerMove={(e) => onMove(i, e)}
          onPointerUp={onUp}
          onPointerCancel={onUp}
        >
          {active?.face === i && (
            <span className={styles.benchValue}>
              {f.label} <span className="num">{num(active.value, active.digits)}</span> mm
            </span>
          )}
        </button>
      ))}
    </>
  );
}
