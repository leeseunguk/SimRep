import { useEffect, useId, useRef, useState, type KeyboardEvent, type PointerEvent } from "react";
import fieldStyles from "./Field.module.css";
import styles from "./inputs.module.css";

interface Props {
  label: string;
  value: number;
  onChange(value: number): void;
  unit?: string;
  step?: number;
  min?: number;
  max?: number;
  precision?: number;
  scrubTitle?: string;
}

const clamp = (v: number, lo = -Infinity, hi = Infinity) => Math.min(hi, Math.max(lo, v));

/**
 * Numeric field: type a value (Enter/blur commits, Esc reverts), use arrow keys (Shift = x10),
 * or drag the label sideways to scrub, so values can be changed without typing.
 */
export function NumberField({ label, value, onChange, unit, step = 1, min, max, precision = 2, scrubTitle }: Props) {
  const id = useId();
  const [text, setText] = useState(() => value.toFixed(precision));
  const editing = useRef(false);
  const scrub = useRef<{ x: number; start: number } | null>(null);

  useEffect(() => {
    if (!editing.current) setText(value.toFixed(precision));
  }, [value, precision]);

  const commit = (raw: string) => {
    const v = Number(raw.replace(",", "."));
    if (Number.isFinite(v)) onChange(clamp(v, min, max));
    else setText(value.toFixed(precision));
  };

  const onKeyDown = (e: KeyboardEvent<HTMLInputElement>) => {
    if (e.key === "Enter") e.currentTarget.blur();
    else if (e.key === "Escape") {
      setText(value.toFixed(precision));
      editing.current = false;
      e.currentTarget.blur();
    } else if (e.key === "ArrowUp" || e.key === "ArrowDown") {
      e.preventDefault();
      const delta = (e.key === "ArrowUp" ? step : -step) * (e.shiftKey ? 10 : 1);
      onChange(clamp(value + delta, min, max));
    }
  };

  const onPointerDown = (e: PointerEvent<HTMLLabelElement>) => {
    if (e.button !== 0) return;
    e.currentTarget.setPointerCapture(e.pointerId);
    scrub.current = { x: e.clientX, start: value };
  };
  const onPointerMove = (e: PointerEvent<HTMLLabelElement>) => {
    if (!scrub.current) return;
    const px = e.clientX - scrub.current.x;
    const next = scrub.current.start + Math.round(px / 2) * step * (e.shiftKey ? 10 : 1);
    onChange(clamp(Number(next.toFixed(precision)), min, max));
  };
  const onPointerUp = () => {
    scrub.current = null;
  };

  return (
    <div className={fieldStyles.field}>
      <label
        htmlFor={id}
        className={`${fieldStyles.label} ${styles.scrub}`}
        title={scrubTitle}
        onPointerDown={onPointerDown}
        onPointerMove={onPointerMove}
        onPointerUp={onPointerUp}
        onPointerCancel={onPointerUp}
      >
        {label}
      </label>
      <div className={styles.numberWrap}>
        <input
          id={id}
          className={`${styles.input} ${styles.number}`}
          inputMode="decimal"
          value={text}
          onFocus={(e) => {
            editing.current = true;
            e.currentTarget.select();
          }}
          onChange={(e) => setText(e.target.value)}
          onBlur={(e) => {
            if (editing.current) commit(e.target.value);
            editing.current = false;
          }}
          onKeyDown={onKeyDown}
        />
        {unit && <span className={styles.unit}>{unit}</span>}
      </div>
    </div>
  );
}
