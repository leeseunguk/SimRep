import type { ReactNode } from "react";
import styles from "./SegmentedControl.module.css";

export interface SegmentOption<T extends string> {
  value: T;
  label: ReactNode;
  title?: string;
  icon?: ReactNode;
}

interface Props<T extends string> {
  options: readonly SegmentOption<T>[];
  value: T;
  onChange(value: T): void;
  ariaLabel: string;
  size?: "sm" | "md";
}

export function SegmentedControl<T extends string>({ options, value, onChange, ariaLabel, size = "sm" }: Props<T>) {
  return (
    <div className={`${styles.group} ${styles[size]}`} role="radiogroup" aria-label={ariaLabel}>
      {options.map((o) => (
        <button
          key={o.value}
          type="button"
          role="radio"
          aria-checked={o.value === value}
          title={o.title}
          className={styles.segment}
          onClick={() => onChange(o.value)}
        >
          {o.icon && <span className={styles.icon}>{o.icon}</span>}
          {o.label}
        </button>
      ))}
    </div>
  );
}
