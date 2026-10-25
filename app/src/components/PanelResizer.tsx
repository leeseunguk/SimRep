import type { KeyboardEvent, PointerEvent } from "react";
import styles from "./PanelResizer.module.css";

/**
 * Drag handle on a panel edge (`edge` = which edge of the panel it sits on).
 * Limits come from the --panel-min-w / --panel-max-w tokens.
 */
export function PanelResizer({ edge, onResize, label }: { edge: "left" | "right"; onResize(width: number): void; label: string }) {
  const limits = () => {
    const css = getComputedStyle(document.documentElement);
    return [parseFloat(css.getPropertyValue("--panel-min-w")), parseFloat(css.getPropertyValue("--panel-max-w"))] as const;
  };
  // Keyboard: arrows move the edge by 16px (Shift: 64px), like a drag.
  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    if (e.key !== "ArrowLeft" && e.key !== "ArrowRight") return;
    e.preventDefault();
    const [min, max] = limits();
    const width = e.currentTarget.parentElement!.getBoundingClientRect().width;
    const dx = (e.key === "ArrowRight" ? 1 : -1) * (e.shiftKey ? 64 : 16);
    onResize(Math.min(max, Math.max(min, width + (edge === "right" ? dx : -dx))));
  };
  const onPointerDown = (e: PointerEvent<HTMLDivElement>) => {
    const el = e.currentTarget;
    const panel = el.parentElement!;
    const [min, max] = limits();
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
      tabIndex={0}
      onKeyDown={onKeyDown}
      onPointerDown={onPointerDown}
    />
  );
}
