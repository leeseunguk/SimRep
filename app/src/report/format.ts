import { formatNumber } from "../i18n";
import type { Lang } from "../state/store";

/** Number formats of the report (locale grouping and decimal marks follow the UI language). */

const locale = (lang: Lang) => (lang === "ko" ? "ko-KR" : "en-US");
const sigFormats = new Map<string, Intl.NumberFormat>();

/** `sig` significant digits ("8.26305", "1,234.57"); "–" for missing or non-finite values. */
export function fmtSig(lang: Lang, v: number | null | undefined, sig = 6): string {
  if (v === null || v === undefined || Number.isNaN(v)) return "–";
  if (!Number.isFinite(v)) return v > 0 ? "∞" : "-∞";
  const key = `${lang}:${sig}`;
  let f = sigFormats.get(key);
  if (!f) {
    f = new Intl.NumberFormat(locale(lang), { maximumSignificantDigits: sig });
    sigFormats.set(key, f);
  }
  // Very large or small magnitudes read better in exponent notation.
  const a = Math.abs(v);
  if (a !== 0 && (a >= 1e9 || a < 1e-4)) return v.toExponential(Math.max(0, sig - 1)).replace(/\.?0+e/, "e").replace("e+", "e");
  return f.format(v);
}

/** Whole counts with grouping ("1,234"). */
export const fmtCount = (lang: Lang, v: number) => formatNumber(lang, Math.round(v), 0);

/** Fixed decimals. */
export const fmtFixed = (lang: Lang, v: number, digits: number) => (Number.isFinite(v) ? formatNumber(lang, v, digits) : "–");

/** Milliseconds: 2 decimals below 10 ms, 1 below 100 ms, whole above. */
export function fmtMs(lang: Lang, ms: number): string {
  return fmtFixed(lang, ms, ms < 10 ? 2 : ms < 100 ? 1 : 0);
}

/** Mass resolving power: whole from 100, one decimal below, ∞ for a zero-width peak. */
export function fmtResolution(lang: Lang, r: number): string {
  if (!Number.isFinite(r)) return "∞";
  return fmtFixed(lang, r, r >= 100 ? 0 : 1);
}

/** Percent of a total, one decimal. */
export const fmtPercent = (lang: Lang, part: number, total: number) => (total > 0 ? `${fmtFixed(lang, (100 * part) / total, 1)}%` : "–");

/** "1 × 2 × 3" style triples. */
export const fmtTriple = (lang: Lang, v: readonly number[], f: (lang: Lang, x: number) => string = (l, x) => fmtSig(l, x, 6)) => v.map((x) => f(lang, x)).join(" × ");

/** Report date: long date and time in the UI language. */
export function fmtDate(lang: Lang, d: Date): string {
  return new Intl.DateTimeFormat(lang === "ko" ? "ko-KR" : "en-GB", { dateStyle: "long", timeStyle: "short" }).format(d);
}
