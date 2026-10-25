import { useEffect, useState } from "react";
import { useApp } from "./store";

export type ResolvedTheme = "light" | "dark";

const query = "(prefers-color-scheme: dark)";

export function resolveTheme(pref: "light" | "dark" | "system", systemDark: boolean): ResolvedTheme {
  return pref === "system" ? (systemDark ? "dark" : "light") : pref;
}

/** The theme actually shown: the user's choice, or the OS setting when set to "system". */
export function useResolvedTheme(): ResolvedTheme {
  const pref = useApp((s) => s.themePref);
  const [systemDark, setSystemDark] = useState(() => matchMedia(query).matches);
  useEffect(() => {
    const mq = matchMedia(query);
    const onChange = (e: MediaQueryListEvent) => setSystemDark(e.matches);
    mq.addEventListener("change", onChange);
    return () => mq.removeEventListener("change", onChange);
  }, []);
  return resolveTheme(pref, systemDark);
}
