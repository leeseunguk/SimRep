import type { TranscriptItem } from "./model";
import { redact } from "../ai/history";

/**
 * Appendix slot of the report: whoever owns the AI chat registers a source of transcript items
 * ({role, text, tool}); the export dialog offers the appendix while one is registered and non-empty.
 */

let source: (() => TranscriptItem[] | null) | null = null;

export function registerTranscript(get: () => TranscriptItem[] | null): () => void {
  source = get;
  return () => {
    if (source === get) source = null;
  };
}

export function reportTranscript(): TranscriptItem[] | null {
  const items = source?.() ?? null;
  return items?.length ? redact(items) : null;
}
