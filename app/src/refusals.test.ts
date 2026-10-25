import { readdirSync, readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { en } from "./i18n/en";
import { translate } from "./i18n";
import { FALLBACK_REASONS, refusalLine, refusals } from "./refusals";

const rustDir = new URL("../src-tauri/src/", import.meta.url);
const rust = readdirSync(rustDir)
  .filter((f) => f.endsWith(".rs"))
  .map((f) => readFileSync(new URL(f, rustDir), "utf8"))
  .join("\n");
/** Every reason code the native core can send (`Reason::new("code")`). */
const rustCodes = [...new Set([...rust.matchAll(/Reason::new\("(\w+)"\)/g)].map((m) => m[1]!))];

describe("refusals", () => {
  it("have a message for every reason code the native core sends", () => {
    expect(rustCodes.length).toBeGreaterThan(20);
    expect(rustCodes.filter((code) => !(`reason.${code}` in en))).toEqual([]);
  });

  it("know the fallback reasons the native core uses", () => {
    expect(FALLBACK_REASONS.filter((code) => !rustCodes.includes(code))).toEqual([]);
  });

  it("say what happened, then why and what to do", () => {
    for (const lang of ["en", "ko"] as const) {
      for (const r of [
        refusals.liveOnCpu({ code: "exactSurfaces" }),
        refusals.refineOnCpu({ key: "reason.gpuSolveFailed", params: { detail: "lost" } }),
        refusals.computeOnCpu({ key: "reason.noVulkan" }),
        refusals.flyBlocked({ key: "fly.blocked.magnetic" }),
        refusals.unsupportedFile("a.xyz"),
        refusals.desktopOnly(),
      ]) {
        const what = translate(lang, r.what.key, r.what.params);
        const why = translate(lang, r.why.key, r.why.params);
        expect(what).not.toMatch(/\{\w+\}/);
        expect(why).not.toMatch(/\{\w+\}/);
        const line = refusalLine(r);
        expect(translate(lang, line.key, line.params)).toBe(`${what}. ${why}`);
      }
    }
  });

  it("show the core's reason for a live flight on the CPU", () => {
    expect(refusals.liveOnCpu({ code: "gpuFlightFailed", params: { detail: "device lost" } }).why).toEqual({
      key: "reason.gpuFlightFailed",
      params: { detail: "device lost" },
    });
    expect(refusals.liveOnCpu({ code: "somethingNew" }).why.key).toBe("reason.unknown");
  });
});
