import type { ReactNode } from "react";
import styles from "./Field.module.css";

/** Label/value row used throughout the properties panel. */
export function Field({ label, children, hint, htmlFor }: { label: ReactNode; children: ReactNode; hint?: string; htmlFor?: string }) {
  return (
    <div className={styles.field}>
      <label className={styles.label} htmlFor={htmlFor}>
        {label}
      </label>
      <div className={styles.control}>{children}</div>
      {hint && <p className={styles.hint}>{hint}</p>}
    </div>
  );
}

export function ReadOnlyValue({ children, mono }: { children: ReactNode; mono?: boolean }) {
  return <span className={`${styles.readonly} ${mono ? "num" : ""}`}>{children}</span>;
}
