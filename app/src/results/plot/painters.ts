import type { Painter, PlotTheme, Raster, TextStyle } from "./chart";

/** Paints on a 2D canvas context already scaled to CSS px. */
export class CanvasPainter implements Painter {
  private readonly bitmaps = new WeakMap<Raster, HTMLCanvasElement>();

  constructor(
    private readonly ctx: CanvasRenderingContext2D,
    private readonly theme: PlotTheme,
  ) {}

  rect(x: number, y: number, w: number, h: number, fill: string) {
    this.ctx.fillStyle = fill;
    this.ctx.fillRect(x, y, w, h);
  }

  path(points: ArrayLike<number>, s: { stroke?: string; width?: number; dash?: number[]; fill?: string }) {
    const c = this.ctx;
    if (points.length < 4) return;
    c.beginPath();
    c.moveTo(points[0]!, points[1]!);
    for (let i = 2; i < points.length; i += 2) c.lineTo(points[i]!, points[i + 1]!);
    if (s.fill) {
      c.closePath();
      c.fillStyle = s.fill;
      c.fill();
      return;
    }
    c.strokeStyle = s.stroke ?? "black";
    c.lineWidth = s.width ?? 1;
    c.lineJoin = "round";
    c.setLineDash(s.dash ?? []);
    c.stroke();
    c.setLineDash([]);
  }

  circle(x: number, y: number, r: number, s: { stroke?: string; width?: number; fill?: string }) {
    const c = this.ctx;
    c.beginPath();
    c.arc(x, y, r, 0, Math.PI * 2);
    if (s.fill) {
      c.fillStyle = s.fill;
      c.fill();
    }
    if (s.stroke) {
      c.strokeStyle = s.stroke;
      c.lineWidth = s.width ?? 1;
      c.stroke();
    }
  }

  private font(mono?: boolean, bold?: boolean) {
    return `${bold ? 600 : 400} ${this.theme.fontSize}px ${mono ? this.theme.mono : this.theme.font}`;
  }

  text(str: string, x: number, y: number, s: TextStyle) {
    const c = this.ctx;
    c.font = this.font(s.mono, s.bold);
    c.fillStyle = s.color;
    c.textAlign = s.align ?? "left";
    c.textBaseline = s.baseline ?? "alphabetic";
    if (s.vertical) {
      c.save();
      c.translate(x, y);
      c.rotate(-Math.PI / 2);
      c.fillText(str, 0, 0);
      c.restore();
    } else {
      c.fillText(str, x, y);
    }
  }

  measure(str: string, mono?: boolean) {
    this.ctx.font = this.font(mono);
    return this.ctx.measureText(str).width;
  }

  image(r: Raster, x: number, y: number, w: number, h: number) {
    let bitmap = this.bitmaps.get(r);
    if (!bitmap) {
      bitmap = rasterCanvas(r);
      this.bitmaps.set(r, bitmap);
    }
    // Density cells stay crisp; point rasters are drawn 1:1 anyway.
    this.ctx.imageSmoothingEnabled = false;
    this.ctx.drawImage(bitmap, x, y, w, h);
  }

  clip(x: number, y: number, w: number, h: number) {
    this.ctx.save();
    this.ctx.beginPath();
    this.ctx.rect(x, y, w, h);
    this.ctx.clip();
  }

  unclip() {
    this.ctx.restore();
  }
}

export function rasterCanvas(r: Raster): HTMLCanvasElement {
  const canvas = document.createElement("canvas");
  canvas.width = r.width;
  canvas.height = r.height;
  canvas.getContext("2d")!.putImageData(new ImageData(r.data, r.width, r.height), 0, 0);
  return canvas;
}

const esc = (s: string) => s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");
const n = (v: number) => (Math.round(v * 100) / 100).toString();
const hex2 = (v: number) => Math.round(Math.min(255, Math.max(0, v))).toString(16).padStart(2, "0");

/**
 * `fill="#rrggbb" fill-opacity="a"` for any computed CSS colour: SVG 1.1 tools (Inkscape,
 * Illustrator) do not read the modern `rgb(r g b / a)` syntax the tokens resolve to.
 */
function paint(attr: "fill" | "stroke", color: string): string {
  const m = color.trim().match(/^rgba?\(\s*([\d.]+)[\s,]+([\d.]+)[\s,]+([\d.]+)(?:\s*[,/]\s*([\d.]+%?))?\s*\)$/i);
  if (!m) return `${attr}="${esc(color)}"`;
  const a = m[4] === undefined ? 1 : m[4].endsWith("%") ? parseFloat(m[4]) / 100 : parseFloat(m[4]);
  const rgb = `#${hex2(Number(m[1]))}${hex2(Number(m[2]))}${hex2(Number(m[3]))}`;
  return a < 1 ? `${attr}="${rgb}" ${attr}-opacity="${n(a)}"` : `${attr}="${rgb}"`;
}

/** Builds an SVG document; rasters are embedded as PNG data URLs. */
export class SvgPainter implements Painter {
  private readonly parts: string[] = [];
  private clips = 0;
  private readonly measureCtx = document.createElement("canvas").getContext("2d")!;

  constructor(private readonly theme: PlotTheme) {}

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

  measure(str: string, mono?: boolean) {
    this.measureCtx.font = `${this.theme.fontSize}px ${mono ? this.theme.mono : this.theme.font}`;
    return this.measureCtx.measureText(str).width;
  }

  image(r: Raster, x: number, y: number, w: number, h: number) {
    const url = rasterCanvas(r).toDataURL("image/png");
    this.parts.push(`<image x="${n(x)}" y="${n(y)}" width="${n(w)}" height="${n(h)}" preserveAspectRatio="none" style="image-rendering:pixelated" href="${url}"/>`);
  }

  clip(x: number, y: number, w: number, h: number) {
    const id = `c${++this.clips}`;
    this.parts.push(`<clipPath id="${id}"><rect x="${n(x)}" y="${n(y)}" width="${n(w)}" height="${n(h)}"/></clipPath><g clip-path="url(#${id})">`);
  }

  unclip() {
    this.parts.push("</g>");
  }

  document(width: number, height: number): string {
    return `<?xml version="1.0" encoding="UTF-8"?>\n<svg xmlns="http://www.w3.org/2000/svg" width="${n(width)}" height="${n(height)}" viewBox="0 0 ${n(width)} ${n(height)}">\n${this.parts.join("\n")}\n</svg>\n`;
  }
}
