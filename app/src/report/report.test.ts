import { describe, expect, it } from "vitest";
import { runOf } from "../results/analysis";
import type { Flight, FlightHeader, IonSource } from "../scene/flight";
import { FLIGHT_TOLERANCES, flatten } from "../state/runFormat";
import { fmtCount, fmtMs, fmtResolution, fmtSig } from "./format";
import { differingSettings, esc, renderReport } from "./html";
import type { ReportModel, RunColumn } from "./model";
import { summarize, surfaceFromKey } from "./summary";

// A deterministic flight: 200 ions, 180 on electrode 3 (TOF a narrow ramp around 8.2 µs), 20 on +x.
function flight(shift = 0): Flight {
  const n = 200;
  const ends = new Float64Array(n * 4);
  const ke = new Float64Array(n);
  const surface = new Uint32Array(n);
  for (let i = 0; i < n; i++) {
    const u = ((i * 37) % 100) / 100;
    ends[i * 4] = 100;
    ends[i * 4 + 1] = (u - 0.5) * 4;
    ends[i * 4 + 2] = ((i * 13) % 50) / 25 - 1;
    ends[i * 4 + 3] = 8.2 + shift + (u - 0.5) * 0.1 * (1 - Math.abs(u - 0.5));
    ke[i] = 100 + u;
    surface[i] = i < 180 ? 0 : 1;
  }
  const header: FlightHeader = {
    ions: n,
    drawn: 0,
    points: 0,
    surfaces: [
      { surface: { kind: "electrode", number: 3, voltage: 0 }, count: 180, tofMean: 8.2 },
      { surface: { kind: "wall", face: "+x" }, count: 20, tofMean: 8.2 },
    ],
    tof: { n, min: 8.15, max: 8.25, mean: 8.2 + shift, std: 0.0123456 },
    keMean: 100.5,
    device: { kind: "cpu", name: "CPU · 32 threads", threads: 32 },
    precision: "live",
    executed: { device: "cpu", precision: "f64", surfaces: "grid" },
    fallback: { code: "gpuStarting" },
    surface: null,
    fieldMs: 0,
    prepareMs: 0,
    flyMs: 12.3456,
    transferMs: 0,
    pathsMs: 0,
    pathsGpu: false,
    totalMs: 20,
    thinned: false,
    gpuBytes: 0,
    dispatchIons: 0,
    surfaceBytes: 1,
    token: 1,
  };
  return { header, offsets: new Uint32Array(n + 1), points: new Float32Array(0), ends, surface, ke, roundTripMs: 0 };
}

const source: IonSource = {
  mode: "beam",
  count: 200,
  massAmu: 100,
  chargeE: 1,
  energyEv: 100,
  position: [1, 0, 0],
  azimuthDeg: 0,
  elevationDeg: 0,
  coneDeg: 0,
  shape: "line",
  widthMm: 14,
  file: null,
  bench: { enabled: false, min: [0, 0, 0], max: [1, 1, 1] },
  durationUs: 100,
  precision: "live",
  collision: "allCorners",
  drawCount: 300,
  exactSurfaces: true,
};

function model(lang: "en" | "ko", patch: Partial<ReportModel> = {}): ReportModel {
  const f = flight();
  return {
    lang,
    generatedAt: new Date(Date.UTC(2026, 9, 28, 9, 30)),
    simrepVersion: "0.1.0",
    projectName: "Einzel <lens> & co",
    projectPath: "C:\\Users\\demo\\Documents\\SimRep\\einzel.simrep",
    array: {
      name: "einzel.pa0",
      fileName: "einzel.pa0",
      path: "C:\\Users\\demo\\einzel.pa0",
      format: "pa0",
      kind: "cylindrical",
      dims: [201, 41, 1],
      spacing: [0.5, 0.5, 0.5],
      sizeMm: [100, 20, 0],
      mirror: { x: false, y: true, z: false },
      antiMirror: { x: false, y: false, z: false },
      field: "electrostatic",
      points: 8241,
      electrodePoints: 655,
      cuts: null,
      electrodes: [
        { name: "Entrance", number: 1, voltage: 0 },
        { name: "Focus", number: 2, voltage: 1234.5 },
        { name: "Exit", number: 3, voltage: 0 },
      ],
    },
    source,
    settings: {
      refine: { device: "CPU · 32 threads", tolerance: 1e-10, cycles: 29, residual: 7.49e-11 },
      precision: "live",
      executed: f.header.executed,
      deviceName: "CPU · 32 threads",
      collision: "allCorners",
      exactSurfaces: true,
      relativistic: true,
      durationUs: 100,
      drawn: 300,
      tolerances: { ...FLIGHT_TOLERANCES },
    },
    results: summarize(f.header, runOf(f), null, { kind: "current" }),
    view3d: { unavailable: { key: "report.view3d.notReady", params: { detail: "no WebGPU adapter" } } },
    runs: [],
    warnings: [{ key: "refusal.line", params: { what: { key: "refusal.liveOnCpu" }, why: { key: "reason.gpuStarting", params: { detail: "" } } } }],
    transcript: null,
    ...patch,
  };
}

/** Every URL-bearing attribute and CSS url() of the document. */
function references(html: string): string[] {
  return [...html.matchAll(/\b(?:src|href|xlink:href|data|poster|action)\s*=\s*"([^"]*)"/g), ...html.matchAll(/url\(\s*['"]?([^'")]+)/g)].map((m) => m[1]!);
}

describe("report HTML", () => {
  for (const lang of ["en", "ko"] as const) {
    it(`is self-contained (${lang}): no network URLs, no scripts, every image a data: URI`, () => {
      const html = renderReport(model(lang, { view3d: { dataUri: "data:image/png;base64,iVBORw0KGgo=" } }));
      expect(html.startsWith("<!DOCTYPE html>")).toBe(true);
      expect(html).not.toMatch(/https?:\/\//i);
      expect(html).not.toMatch(/<script|<link\b|@import|<iframe/i);
      const imgs = [...html.matchAll(/<img\b[^>]*\bsrc="([^"]*)"/g)].map((m) => m[1]!);
      // TOF, energy, detector map, 3D snapshot.
      expect(imgs).toHaveLength(4);
      for (const src of imgs) expect(src).toMatch(/^data:image\/(png|svg\+xml);base64,[A-Za-z0-9+/=]+$/);
      // Only the inline SVG clip references (url(#c1)) and data: URIs remain.
      for (const ref of references(html)) expect(ref.startsWith("data:") || ref.startsWith("#")).toBe(true);
      // The embedded SVG figures reference nothing outside themselves either.
      for (const src of imgs.filter((s) => s.startsWith("data:image/svg+xml"))) {
        const svg = atob(src.split(",")[1]!);
        expect(svg).toMatch(/^<svg xmlns="http:\/\/www\.w3\.org\/2000\/svg"/);
        expect(svg.replace('xmlns="http://www.w3.org/2000/svg"', "")).not.toMatch(/https?:|<image\b|href=/);
      }
      expect(html).toContain("@page { size: A4;");
      expect(html).toContain("@media print");
    });
  }

  it("follows the UI language", () => {
    const en = renderReport(model("en"));
    const ko = renderReport(model("ko"));
    expect(en).toContain('<html lang="en">');
    expect(ko).toContain('<html lang="ko">');
    expect(en).toContain("<h1>Simulation report</h1>");
    expect(ko).toContain("<h1>시뮬레이션 보고서</h1>");
    expect(en).toContain("Mass resolving power m/Δm");
    expect(ko).toContain("질량 분해능 m/Δm");
    expect(ko).toContain("계산만 포함");
    expect(en).toContain("Compute only");
    expect(ko).toContain("2026년 10월 28일");
  });

  it("escapes project text and keeps the warnings, the 3D note and the transcript appendix", () => {
    const html = renderReport(
      model("en", {
        transcript: [
          { role: "user", text: "Focus <b>this</b> lens" },
          { role: "assistant", text: "Running a sweep." },
          { role: "assistant", text: '{"ok":true}', tool: "fly" },
        ],
      }),
    );
    expect(html).toContain("Einzel &lt;lens&gt; &amp; co");
    expect(html).not.toContain("<b>this</b>");
    expect(html).toContain("Focus &lt;b&gt;this&lt;/b&gt; lens");
    expect(html).toContain("Assistant · fly");
    expect(html).toContain("Appendix: AI assistant transcript");
    expect(html).toContain("Live preview flew on the CPU (f64), not the GPU (f32)");
    expect(html).toContain("The 3D view snapshot is not included: the WebGPU 3D view is not running (no WebGPU adapter)");
    expect(renderReport(model("en"))).not.toContain("Appendix: AI assistant transcript");
  });

  it("lists geometry, electrodes with names and voltages, and solver/flight settings", () => {
    const html = renderReport(model("en"));
    expect(html).toContain('<td class="num">201 × 41 × 1</td>');
    expect(html).toContain('<td class="num">0.5 × 0.5 × 0.5</td>');
    expect(html).toContain("2D cylindrical array");
    expect(html).toMatch(/<td>Focus<\/td><td class="num">2<\/td><td class="num">1,234\.5<\/td>/);
    expect(html).toContain("CPU · CPU · 32 threads");
    expect(html).toContain("relative 1e-8; position 1e-8 mm");
    expect(html).toContain("V-cycles, residual 7.49e-11");
  });

  it("compares selected runs with a settings diff", () => {
    const a = flight();
    const b = flight(0.5);
    const col = (label: string, f: Flight, voltage: number): RunColumn => ({
      label,
      createdAt: "2026-10-28T09:00:00.000Z",
      summary: summarize(f.header, runOf(f), null, { kind: "run", label }),
      settings: flatten({ inputs: { voltages: { "2": voltage }, precision: "live" }, simrepVersion: "0.1.0" }),
    });
    const runs = [col("Run 1", a, 80), col("Run 2", b, 95)];
    expect(differingSettings(runs)).toEqual(["inputs.voltages.2"]);
    const html = renderReport(model("en", { runs }));
    expect(html).toContain("Run comparison");
    expect(html).toContain("<th class=\"num\">Run 1</th><th class=\"num\">Run 2</th>");
    expect(html).toMatch(/<td><code>inputs\.voltages\.2<\/code><\/td><td>80<\/td><td>95<\/td>/);
    expect(html).toContain("TOF histograms of the compared runs");
    expect(renderReport(model("en", { runs: [runs[0]!] }))).toContain("Only one run selected");
  });

  it("states that nothing flew instead of inventing results", () => {
    const html = renderReport(model("en", { results: null, view3d: null }));
    expect(html).toContain("Nothing has flown yet: no results.");
    expect(html).not.toMatch(/<img\b/);
  });
});

describe("report metrics", () => {
  it("summarizes a flight: arrivals, end states, TOF, resolving power, compute_ms", () => {
    const f = flight();
    const s = summarize(f.header, runOf(f), null, { kind: "current" });
    expect(s.ions).toBe(200);
    expect(s.arrivals).toBe(200);
    expect(s.splats.map((x) => x.count)).toEqual([180, 20]);
    expect(s.tof).toEqual({ mean: 8.2, std: 0.0123456, min: 8.15, max: 8.25 });
    expect(s.computeMs).toBe(12.3456);
    expect(s.resolution!.value).toBeGreaterThan(0);
    expect(s.resolution!.left).toBeLessThan(s.resolution!.right);
    expect(s.map!.counts.reduce((a, b) => a + b, 0)).toBe(180);
    const html = renderReport(model("en", { results: s }));
    expect(html).toContain('<tr><td>TOF std dev (population), µs</td><td class="num">0.0123456</td></tr>');
    expect(html).toContain('<tr><td>Compute time, ms (compute_ms)</td><td class="num">12.3</td></tr>');
    expect(html).toContain('<tr><td>End states: Electrode 3</td><td class="num">180 (90.0%)</td></tr>');
  });

  it("uses a stored run's plot summaries when its per-ion results were not stored", () => {
    const f = flight();
    const s = summarize(f.header, null, { histogram: { lo: 8, hi: 9, counts: [0, 5, 10, 5, 0] }, detectorMap: { surface: "electrode:3", axes: ["x", "r"], x: [0, 1], y: [0, 1], nx: 2, ny: 2, counts: [1, 2, 3, 4] } }, { kind: "run", label: "Run 7" });
    expect(s.tofHist!.counts).toEqual(Float64Array.from([0, 5, 10, 5, 0]));
    expect(s.keHist).toBeNull();
    expect(s.map!.surface).toEqual({ key: "fly.surface.electrode", params: { n: 3 } });
    expect(renderReport(model("en", { results: s }))).toContain("From stored run Run 7 (no flight in memory).");
  });

  it("parses surface keys back", () => {
    expect(surfaceFromKey("electrode:2")).toEqual({ kind: "electrode", number: 2, voltage: null });
    expect(surfaceFromKey("electrode:V-10")).toEqual({ kind: "electrode", number: null, voltage: -10 });
    expect(surfaceFromKey("wall:+x")).toEqual({ kind: "wall", face: "+x" });
    expect(surfaceFromKey("timeLimit")).toEqual({ kind: "timeLimit" });
  });

  it("formats numbers per language", () => {
    expect(fmtSig("en", 8.263051984587218, 7)).toBe("8.263052");
    expect(fmtSig("ko", 1234.5678, 6)).toBe("1,234.57");
    expect(fmtSig("en", 1e-10, 3)).toBe("1e-10");
    expect(fmtSig("en", 7.49e-11, 3)).toBe("7.49e-11");
    expect(fmtSig("en", null)).toBe("–");
    expect(fmtCount("ko", 1234567)).toBe("1,234,567");
    expect(fmtMs("en", 0.7639)).toBe("0.76");
    expect(fmtMs("en", 37.959)).toBe("38.0");
    expect(fmtMs("en", 1234.4)).toBe("1,234");
    expect(fmtResolution("en", 123.4)).toBe("123");
    expect(fmtResolution("en", 45.67)).toBe("45.7");
    expect(fmtResolution("en", Infinity)).toBe("∞");
  });

  it("escapes every HTML-significant character", () => {
    expect(esc(`<a href="x">'&'</a>`)).toBe("&lt;a href=&quot;x&quot;&gt;&#39;&amp;&#39;&lt;/a&gt;");
  });
});
