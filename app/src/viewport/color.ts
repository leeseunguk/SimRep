export type Rgb = [number, number, number];

/** Parses #rgb / #rrggbb tokens (the --vp-* viewport tokens are hex on purpose). */
export function parseHex(value: string): Rgb {
  const hex = value.trim().replace(/^#/, "");
  const full = hex.length === 3 ? [...hex].map((c) => c + c).join("") : hex.slice(0, 6);
  const n = Number.parseInt(full, 16);
  if (!Number.isFinite(n) || full.length !== 6) throw new Error(`Viewport colour token is not hex: "${value}"`);
  return [((n >> 16) & 255) / 255, ((n >> 8) & 255) / 255, (n & 255) / 255];
}

export function srgbToLinear(c: Rgb): Rgb {
  return c.map((v) => (v <= 0.04045 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4)) as Rgb;
}

export function mixRgb(a: Rgb, b: Rgb, t: number): Rgb {
  return [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t];
}

export interface ViewportTheme {
  grid: Rgb;
  axisX: Rgb;
  axisY: Rgb;
  axisZ: Rgb;
  electrodeNeutral: Rgb;
  electrodePositive: Rgb;
  electrodeNegative: Rgb;
  meshImported: Rgb;
  selection: Rgb;
  sky: Rgb;
  ground: Rgb;
  /** Potential colour map, low to high (5 evenly spaced stops). */
  field: [Rgb, Rgb, Rgb, Rgb, Rgb];
  /** Equipotential lines over light / dark parts of the colour map. */
  contourDark: Rgb;
  contourLight: Rgb;
  /** Ion trajectories and their end points (splats); electrode hits stand out. */
  trajectory: Rgb;
  splat: Rgb;
  splatHit: Rgb;
  /** Selected ion (results panel) and its line width / end ring radius, CSS px. */
  highlight: Rgb;
  highlightWidth: number;
  highlightRing: number;
  /** Workbench box lines. */
  bench: Rgb;
  /** Import-wizard part colours, by (electrode number - 1) mod 6. */
  parts: Rgb[];
}

export const PART_COLORS = 6;

export function readViewportTheme(el: Element = document.documentElement): ViewportTheme {
  const css = getComputedStyle(el);
  const token = (name: string) => parseHex(css.getPropertyValue(name));
  return {
    grid: token("--vp-grid"),
    axisX: token("--vp-axis-x"),
    axisY: token("--vp-axis-y"),
    axisZ: token("--vp-axis-z"),
    electrodeNeutral: token("--vp-electrode-neutral"),
    electrodePositive: token("--vp-electrode-positive"),
    electrodeNegative: token("--vp-electrode-negative"),
    meshImported: token("--vp-mesh-imported"),
    selection: token("--vp-selection"),
    sky: token("--vp-sky"),
    ground: token("--vp-ground"),
    field: [token("--vp-field-0"), token("--vp-field-1"), token("--vp-field-2"), token("--vp-field-3"), token("--vp-field-4")],
    contourDark: token("--vp-contour-dark"),
    contourLight: token("--vp-contour-light"),
    trajectory: token("--vp-trajectory"),
    splat: token("--vp-splat"),
    splatHit: token("--vp-splat-hit"),
    highlight: token("--vp-highlight"),
    highlightWidth: parseFloat(css.getPropertyValue("--vp-highlight-w")),
    highlightRing: parseFloat(css.getPropertyValue("--vp-highlight-ring")),
    bench: token("--vp-bench"),
    parts: Array.from({ length: PART_COLORS }, (_, i) => token(`--vp-part-${i + 1}`)),
  };
}
