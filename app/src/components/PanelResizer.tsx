import type { PointerEvent } from "react";
import styles from "./PanelResizer.module.css";

/**
 * Drag handle on a panel edge (`edge` = which edge of the panel it sits on).
 * Limits come from the --panel-min-w / --panel-max-w tokens.
 */
export function PanelResizer({ edge, onResize, label }: { edge: "left" | "right"; onResize(width: number): void; label: string }) {
  const onPointerDown = (e: PointerEvent<HTMLDivElement>) => {
    const el = e.currentTarget;
    const panel = el.parentElement!;
    const css = getComputedStyle(document.documentElement);
    const min = parseFloat(css.getPropertyValue("--panel-min-w"));
    const max = parseFloat(css.getPropertyValue("--panel-max-w"));
    const startX = e.clientX;
    const startWidth = panel.getBoundingClientRect().width;
    el.setPointerCapture(e.pointerId);
    const move = (ev: globalThis.PointerEvent) => {
      const dx = ev.clientX - startX;
      onResize(Math.min(max, Math.max(min, startWidth + (edge === "right" ? dx : -dx))));
    };
    const up = () => {
      el.removeEventListener("pointermove", move);
      el.removeEventListener("pointerup", up);
      el.removeEventListener("pointercancel", up);
    };
    el.addEventListener("pointermove", move);
    el.addEventListener("pointerup", up);
    el.addEventListener("pointercancel", up);
  };

  return (
    <div
      className={styles.resizer}
      data-edge={edge}
      role="separator"
      aria-orientation="vertical"
      aria-label={label}
      onPointerDown={onPointerDown}
    />
  );
}
