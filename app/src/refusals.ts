import type { Message } from "./i18n";
import { reasonMessage, type Reason } from "./scene/backend";

/**
 * Every place SimRep refuses a request, or runs it differently from what was asked, says so
 * through this module, so nothing is substituted silently: `what` names what happened (or did
 * not), `why` gives the cause and ends with what to do. The texts are i18n keys; most `why`s are
 * the native core's coded reasons (`reason.<code>`, see backend.ts `reasonMessage`).
 */
export interface Refusal {
  what: Message;
  why: Message;
}

/**
 * Reason codes the native core reports when it ran a request on another device than asked
 * (fly.rs header `fallback`, live.rs start-up, refine.rs "device" messages).
 */
export const FALLBACK_REASONS = [
  "noVulkan",
  "gpuInit",
  "gpuStarting",
  "fieldUploading",
  "fieldReplaced",
  "fieldFailed",
  "gpuFlightFailed",
  "exactSurfaces",
  "cutsInvalid",
  "gpuSolveFailed",
] as const;

export const refusals = {
  /** A live (GPU f32) flight flew as CPU f64: exact surfaces, no GPU, the field not on it yet. */
  liveOnCpu: (reason: Reason): Refusal => ({ what: { key: "refusal.liveOnCpu" }, why: reasonMessage(reason) }),
  /** A refine on the GPU continues (or runs) on the CPU. `why`: the core's reason. */
  refineOnCpu: (why: Message): Refusal => ({ what: { key: "refusal.refineOnCpu" }, why }),
  /** No GPU for ion flight at start-up, so live re-fly uses the CPU. */
  computeOnCpu: (why: Message): Refusal => ({ what: { key: "refusal.computeOnCpu" }, why }),
  /** The Fly button is off. `why`: the blocker (scene/simulation.ts `flyBlocker`). */
  flyBlocked: (why: Message): Refusal => ({ what: { key: "refusal.flyBlocked" }, why }),
  unsupportedFile: (file: string): Refusal => ({ what: { key: "toast.unsupported.title", params: { file } }, why: { key: "toast.unsupported.body" } }),
  /** Browser preview: refining and flying need the native core. */
  desktopOnly: (): Refusal => ({ what: { key: "sim.desktopOnly.title" }, why: { key: "sim.desktopOnly" } }),
};

/** One line, "what. why", for hints, tooltips and the activity log. */
export function refusalLine(r: Refusal): Message {
  return { key: "refusal.line", params: { what: r.what, why: r.why } };
}
