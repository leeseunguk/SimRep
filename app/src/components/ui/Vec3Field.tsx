import { useEffect, useRef, useState, type KeyboardEvent } from "react";
import fieldStyles from "./Field.module.css";
import styles from "./inputs.module.css";

type Vec3 = [number, number, number];
const AXES = ["X", "Y", "Z"] as const;

/** One compact numeric input (commit on Enter / blur, Esc reverts, arrows step). */
function Component({ axis, value, step, onCommit }: { axis: string; value: number; step: number; onCommit(v: number): void }) {
  const [text, setText] = useState(String(value));
  const editing = useRef(false);
  useEffect(() => {
    if (!editing.current) setText(String(value));
  }, [value]);
  const commit = () => {
    editing.current = false;
    const v = Number(text.replace(",", "."));
    if (Number.isFinite(v)) onCommit(v);
    else setText(String(value));
  };
  const onKeyDown = (e: KeyboardEvent<HTMLInputElement>) => {
    if (e.key === "Enter") e.currentTarget.blur();
    else if (e.key === "Escape") {
      editing.current = false;
      setText(String(value));
      e.currentTarget.blur();
    } else if (e.key === "ArrowUp" || e.key === "ArrowDown") {
      e.preventDefault();
      const next = Number((value + (e.key === "ArrowUp" ? step : -step) * (e.shiftKey ? 10 : 1)).toPrecision(12));
      setText(String(next));
      onCommit(next);
    }
  };
  return (
    <label className={styles.vecItem}>
      <span className={styles.vecAxis}>{axis}</span>
      <input
        className={`${styles.input} ${styles.number} ${styles.vecInput}`}
        inputMode="decimal"
        value={text}
        aria-label={axis}
        onFocus={(e) => {
          editing.current = true;
          e.currentTarget.select();
        }}
        onChange={(e) => setText(e.target.value)}
        onBlur={commit}
        onKeyDown={onKeyDown}
      />
    </label>
  );
}

/** Label + X/Y/Z inputs on one row (positions, box corners). */
export function Vec3Field({ label, value, onChange, step = 1, unit }: { label: string; value: Vec3; onChange(v: Vec3): void; step?: number; unit?: string }) {
  return (
    <div className={fieldStyles.field}>
      <span className={fieldStyles.label}>
        {label}
        {unit && <span className={styles.vecUnit}> ({unit})</span>}
      </span>
      <div className={styles.vec}>
        {AXES.map((axis, i) => (
          <Component
            key={axis}
            axis={axis}
            value={value[i]!}
            step={step}
            onCommit={(v) => {
              const next = [...value] as Vec3;
              next[i] = v;
              onChange(next);
            }}
          />
        ))}
      </div>
    </div>
  );
}
