import { useCallback } from "react";
import { en, type MessageKey } from "./en";
import { ko } from "./ko";
import { useApp, type Lang } from "../state/store";

export type { MessageKey };

/** A translatable message: key + params. Params may nest other messages (e.g. an error reason). */
export interface Message {
  key: MessageKey;
  params?: MessageParams;
}
export type MessageParam = string | number | Message;
export type MessageParams = Record<string, MessageParam>;

const dictionaries: Record<Lang, Record<MessageKey, string>> = { en, ko };
const numberFormats: Record<Lang, Intl.NumberFormat> = {
  en: new Intl.NumberFormat("en-US", { maximumFractionDigits: 3 }),
  ko: new Intl.NumberFormat("ko-KR", { maximumFractionDigits: 3 }),
};

/**
 * Adds (or, for its own wording, replaces) messages for a host build of this UI (host.ts). Both
 * languages are required, so every text exists in Korean and English.
 */
export function addMessages(extra: { en: Record<string, string>; ko: Record<string, string> }): void {
  Object.assign(dictionaries.en, extra.en);
  Object.assign(dictionaries.ko, extra.ko);
}

export function translate(lang: Lang, key: MessageKey, params?: MessageParams): string {
  const template = dictionaries[lang][key];
  if (!params) return template;
  return template.replace(/\{(\w+)\}/g, (match, name: string) => {
    const value = params[name];
    if (value === undefined) return match;
    if (typeof value === "number") return numberFormats[lang].format(value);
    if (typeof value === "string") return value;
    return translate(lang, value.key, value.params);
  });
}

/** Fixed-digit formatters, built once: numbers re-render on every slider frame during live re-fly. */
const fixedFormats = new Map<string, Intl.NumberFormat>();

export function formatNumber(lang: Lang, value: number, fractionDigits?: number): string {
  if (fractionDigits === undefined) return numberFormats[lang].format(value);
  const key = `${lang}:${fractionDigits}`;
  let format = fixedFormats.get(key);
  if (!format) {
    format = new Intl.NumberFormat(lang === "ko" ? "ko-KR" : "en-US", { minimumFractionDigits: fractionDigits, maximumFractionDigits: fractionDigits });
    fixedFormats.set(key, format);
  }
  return format.format(value);
}

export function formatBytes(lang: Lang, bytes: number): string {
  const units = ["B", "KB", "MB", "GB"];
  let value = bytes;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit++;
  }
  return `${formatNumber(lang, value, unit === 0 ? 0 : 1)} ${units[unit]}`;
}

export function useI18n() {
  const lang = useApp((s) => s.lang);
  const t = useCallback(
    (key: MessageKey, params?: MessageParams) => translate(lang, key, params),
    [lang],
  );
  const tm = useCallback((m: Message) => translate(lang, m.key, m.params), [lang]);
  const num = useCallback((v: number, digits?: number) => formatNumber(lang, v, digits), [lang]);
  const bytes = useCallback((v: number) => formatBytes(lang, v), [lang]);
  return { lang, t, tm, num, bytes };
}
