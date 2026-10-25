import styles from "./Kbd.module.css";

/** A key cap. `word`: a spelled-out input ("Left-drag") in the UI font, so Hangul reads normally. */
export function Kbd({ children, word }: { children: string; word?: boolean }) {
  return <kbd className={word ? `${styles.kbd} ${styles.word}` : styles.kbd}>{children}</kbd>;
}
