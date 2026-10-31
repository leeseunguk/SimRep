import { afterEach, describe, expect, it } from "vitest";
import { allows, host, registerHost, unavailable, type HostFeature } from "./host";
import type { Refusal } from "./refusals";

const off: Refusal = { what: { key: "sim.desktopOnly.title" }, why: { key: "sim.desktopOnly" } };

describe("host", () => {
  afterEach(() => registerHost(null));

  it("allows every feature without a host (desktop app, demo mode)", () => {
    registerHost(null);
    expect(host()).toBeNull();
    for (const f of ["templates", "refine", "fly", "gpu", "ai", "cad", "folders", "tour"] as HostFeature[]) {
      expect(allows(f)).toBe(true);
      expect(unavailable(f)).toBeNull();
    }
  });

  it("reports the refusal of a registered host for the features it switches off", () => {
    registerHost({ unavailable: (f) => (f === "ai" || f === "cad" ? off : null) });
    expect(unavailable("ai")).toBe(off);
    expect(allows("cad")).toBe(false);
    expect(allows("refine")).toBe(true);
  });
});
