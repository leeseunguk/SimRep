import type { ReactNode } from "react";
import styles from "./Badge.module.css";

export function Badge({
  tone = "neutral",
  children,
  title,
}: {
  tone?: "neutral" | "accent" | "success" | "warning" | "danger" | "info";
  children: ReactNode;
  title?: string;
}) {
  return (
    <span className={`${styles.badge} ${styles[tone]}`} title={title}>
      {children}
    </span>
  );
}
