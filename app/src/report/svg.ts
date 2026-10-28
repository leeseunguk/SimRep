import { drawChart, type ChartSpec, type Painter, type PlotTheme, type Raster, type TextStyle } from "../results/plot/chart";

/**
 * Report figures: the results panel's chart model (results/plot/chart.ts) painted to a standalone
 * SVG document without the DOM, so figures are drawn from the data (never screenshots), the same
 * in the app and in unit tests. Rasters (detector maps) become merged runs of <rect>s instead of an
 * embedded PNG; text widths are estimated (no canvas to measure with).
 */

/** Light, print-friendly plot colours (independent of the app theme). */
export const PRINT_THEME: PlotTheme = {
  bg: "#ffffff",
  grid: "#e7e7e7",
  axis: "#8a8a8a",
  text: "#5a5a5a",
  textStrong: "#222222",
  primary: "#c86f00",
  primaryFill: "rgba(245, 154, 20, 0.45)",
  band: "rgba(245, 154, 20, 0.14)",
  select: "#17191d",
  series: ["#1d68c4", "#12845a", "#d1176f", "#7a4fd6", "#9a6a00"],
  xyz: ["#e0404c", "#17a06c", "#2f7de1"],
  heat: ["#440154", "#3b528b", "#21918c", "#5ec962", "#fde725"],
  font: "'IBM Plex Sans', 'IBM Plex Sans KR', 'Malgun Gothic', 'Apple SD Gothic Neo', 'Noto Sans KR', Arial, sans-serif",
  mono: "'IBM Plex Mono', Consolas, 'Courier New', monospace",
  fontSize: 12,
  tickGap: 14,
  lineW: 1.5,
  selectW: 2,
  markerR: 5,
  point: 2,
};

const esc = (s: string) => s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");
const n = (v: number) => (Math.round(v * 100) / 100).toString();
const hex2 = (v: number) => Math.round(Math.min(255, Math.max(0, v))).toString(16).padStart(2, "0");

/** `fill="#rrggbb" fill-opacity="a"` (SVG 1.1 readers do not take every CSS colour syntax). */
function paint(attr: "fill" | "stroke", color: string): string {
  const m = color.trim().match(/^rgba?\(\s*([\d.]+)[\s,]+([\d.]+)[\s,]+([\d.]+)(?:\s*[,/]\s*([\d.]+%?))?\s*\)$/i);
  if (!m) return `${attr}="${esc(color)}"`;
  const a = m[4] === undefined ? 1 : m[4].endsWith("%") ? parseFloat(m[4]) / 100 : parseFloat(m[4]);
  const rgb = `#${hex2(Number(m[1]))}${hex2(Number(m[2]))}${hex2(Number(m[3]))}`;
  return a < 1 ? `${attr}="${rgb}" ${attr}-opacity="${n(a)}"` : `${attr}="${rgb}"`;
}

export class ReportSvgPainter implements Painter {
  private readonly parts: string[] = [];
  private clips = 0;

  constructor(private readonly theme: PlotTheme = PRINT_THEME) {}

  rect(x: number, y: number, w: number, h: number, fill: string) {
    this.parts.push(`<rect x="${n(x)}" y="${n(y)}" width="${n(w)}" height="${n(h)}" ${paint("fill", fill)}/>`);
  }

  path(points: ArrayLike<number>, s: { stroke?: string; width?: number; dash?: number[]; fill?: string }) {
    if (points.length < 4) return;
    let d = `M${n(points[0]!)} ${n(points[1]!)}`;
    for (let i = 2; i < points.length; i += 2) d += `L${n(points[i]!)} ${n(points[i + 1]!)}`;
    if (s.fill) this.parts.push(`<path d="${d}Z" ${paint("fill", s.fill)}/>`);
    else
      this.parts.push(
        `<path d="${d}" fill="none" ${paint("stroke", s.stroke ?? "black")} stroke-width="${n(s.width ?? 1)}" stroke-linejoin="round"${s.dash ? ` stroke-dasharray="${s.dash.join(" ")}"` : ""}/>`,
      );
  }

  circle(x: number, y: number, r: number, s: { stroke?: string; width?: number; fill?: string }) {
    this.parts.push(
      `<circle cx="${n(x)}" cy="${n(y)}" r="${n(r)}" ${s.fill ? paint("fill", s.fill) : 'fill="none"'}${s.stroke ? ` ${paint("stroke", s.stroke)} stroke-width="${n(s.width ?? 1)}"` : ""}/>`,
    );
  }

  text(str: string, x: number, y: number, s: TextStyle) {
    const anchor = s.align === "center" ? "middle" : s.align === "right" ? "end" : "start";
    const baseline = s.baseline === "top" ? "hanging" : s.baseline === "middle" ? "central" : s.baseline === "bottom" ? "text-after-edge" : "alphabetic";
    const family = esc(s.mono ? this.theme.mono : this.theme.font);
    const transform = s.vertical ? ` transform="rotate(-90 ${n(x)} ${n(y)})"` : "";
    this.parts.push(
      `<text x="${n(x)}" y="${n(y)}" ${paint("fill", s.color)} font-family="${family}" font-size="${this.theme.fontSize}" font-weight="${s.bold ? 600 : 400}" text-anchor="${anchor}" dominant-baseline="${baseline}"${transform}>${esc(str)}</text>`,
    );
  }

  /** Estimated advance width: wide (CJK) characters count as one em, others as ~0.56 em. */
  measure(str: string, mono?: boolean) {
    let em = 0;
    for (const ch of str) em += ch.codePointAt(0)! > 0x2e80 ? 1 : mono ? 0.6 : 0.56;
    return em * this.theme.fontSize;
  }

  /** Each row's runs of equal, non-transparent pixels as one rect. */
  image(r: Raster, x: number, y: number, w: number, h: number) {
    const sx = w / r.width;
    const sy = h / r.height;
    const out: string[] = [];
    for (let row = 0; row < r.height; row++) {
      let col = 0;
      while (col < r.width) {
        const o = (row * r.width + col) * 4;
        const a = r.data[o + 3]!;
        if (a === 0) {
          col++;
          continue;
        }
        const color = `#${hex2(r.data[o]!)}${hex2(r.data[o + 1]!)}${hex2(r.data[o + 2]!)}`;
        let end = col + 1;
        while (end < r.width) {
          const p = (row * r.width + end) * 4;
          if (r.data[p + 3] !== a || r.data[p] !== r.data[o] || r.data[p + 1] !== r.data[o + 1] || r.data[p + 2] !== r.data[o + 2]) break;
          end++;
        }
        // A hair of overlap hides anti-aliasing seams between cells.
        out.push(
          `<rect x="${n(x + col * sx)}" y="${n(y + row * sy)}" width="${n((end - col) * sx + 0.3)}" height="${n(sy + 0.3)}" fill="${color}"${a < 255 ? ` fill-opacity="${n(a / 255)}"` : ""}/>`,
        );
        col = end;
      }
    }
    this.parts.push(`<g shape-rendering="crispEdges">${out.join("")}</g>`);
  }

  clip(x: number, y: number, w: number, h: number) {
    const id = `c${++this.clips}`;
    this.parts.push(`<clipPath id="${id}"><rect x="${n(x)}" y="${n(y)}" width="${n(w)}" height="${n(h)}"/></clipPath><g clip-path="url(#${id})">`);
  }

  unclip() {
    this.parts.push("</g>");
  }

  document(width: number, height: number): string {
    return `<svg xmlns="http://www.w3.org/2000/svg" width="${n(width)}" height="${n(height)}" viewBox="0 0 ${n(width)} ${n(height)}">${this.parts.join("")}</svg>`;
  }
}

/** One chart as an SVG document. */
export function chartSvg(spec: ChartSpec, width: number, height: number, theme: PlotTheme = PRINT_THEME): string {
  const p = new ReportSvgPainter(theme);
  drawChart(p, spec, width, height, theme);
  return p.document(width, height);
}

/** Standard base64 of bytes (browser and Node both have btoa). */
export function base64(bytes: Uint8Array): string {
  let s = "";
  const chunk = 0x8000;
  for (let i = 0; i < bytes.length; i += chunk) s += String.fromCharCode(...bytes.subarray(i, i + chunk));
  return btoa(s);
}

export const svgDataUri = (svg: string) => `data:image/svg+xml;base64,${base64(new TextEncoder().encode(svg))}`;
export const pngDataUri = (png: Uint8Array) => `data:image/png;base64,${base64(png)}`;
