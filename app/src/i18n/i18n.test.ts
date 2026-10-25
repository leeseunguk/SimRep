import { describe, expect, it } from "vitest";
import { en } from "./en";
import { translate } from "./index";
import { ko } from "./ko";

const placeholders = (s: string) => [...s.matchAll(/\{(\w+)\}/g)].map((m) => m[1]).sort();

describe("dictionaries", () => {
  it("define the same keys", () => {
    expect(Object.keys(ko).sort()).toEqual(Object.keys(en).sort());
  });

  it("use the same placeholders in both languages", () => {
    const mismatched = Object.keys(en).filter((k) => placeholders(en[k as keyof typeof en]).join() !== placeholders(ko[k as keyof typeof ko]).join());
    expect(mismatched).toEqual([]);
  });
});

describe("translate", () => {
  it("fills nested messages and formats numbers for the language", () => {
    const inner = { key: "refine.cycle" as const, params: { n: 1234 } };
    expect(translate("en", "refusal.line", { what: inner, why: "x" })).toBe(`${translate("en", "refine.cycle", { n: 1234 })}. x`);
    expect(translate("en", "refine.cycle", { n: 1234 })).toContain("1,234");
    expect(translate("ko", "refine.cycle", { n: 1234 })).toContain("1,234");
  });
});
