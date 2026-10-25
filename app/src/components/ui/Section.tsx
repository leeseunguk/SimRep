import { useState, type ReactNode } from "react";
import { ChevronDown } from "lucide-react";
import styles from "./Section.module.css";

export function Section({ title, children, defaultOpen = true }: { title: string; children: ReactNode; defaultOpen?: boolean }) {
  const [open, setOpen] = useState(defaultOpen);
  return (
    <section className={styles.section} data-open={open}>
      <button type="button" className={styles.header} aria-expanded={open} onClick={() => setOpen(!open)}>
        <ChevronDown className={styles.chevron} aria-hidden />
        {title}
      </button>
      {open && <div className={styles.body}>{children}</div>}
    </section>
  );
}
