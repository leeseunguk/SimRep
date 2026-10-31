import { boundsSize, scaleBounds, unionBounds, type Bounds, type MeshData, type Vec3 } from "../scene/mesh";
import { OrbitCamera } from "./camera";
import { srgbToLinear, type Rgb, type ViewportTheme } from "./color";
import type { PaDrawItem, ProbeHit } from "./paLayer";
import { FRAME_FLOATS } from "./shaders";
import type { Highlight, IonHit, TrajectoryItem } from "./trajectoryLayer";

/** Graphics API drawing the 3D view: WebGPU when the browser has an adapter, else WebGL2. */
export type RendererApi = "webgpu" | "webgl2";

/** Status bar / log name of a graphics API. */
export const rendererName = (api: RendererApi) => (api === "webgpu" ? "WebGPU" : "WebGL2");

/** CSS px of the viewport covered by overlays on each side (see Viewport's data-inset elements). */
export interface Insets {
  left: number;
  right: number;
  top: number;
  bottom: number;
}

export interface DrawItem {
  id: string;
  mesh: MeshData;
  /** sRGB base colour. */
  color: Rgb;
  visible: boolean;
  selected: boolean;
  scale: number;
}

/** CPU side of the array / trajectory layers both backends share (fit, picking, probing). */
interface SceneLayers {
  pa: {
    bounds(ids?: readonly string[]): Bounds | null;
    pickables(): { id: string; bounds: Bounds; visible: boolean }[];
    probe(origin: Vec3, dir: Vec3): ProbeHit | null;
  };
  trajectories: {
    bounds(ids?: readonly string[]): Bounds | null;
    pick(viewProj: Float32Array, width: number, height: number, px: number, py: number, tolerance: number): IonHit | null;
  };
}

export const GRID_OPACITY = 0.32;
export const BENCH_OPACITY = 0.85;
export const OBJECT_FLOATS = 24;

/** The 12 edges of a box, as line-list vertex pairs. */
export function boxLines(b: Bounds, out: number[]) {
  const c = (i: number) => [i & 1 ? b.max[0] : b.min[0], i & 2 ? b.max[1] : b.min[1], i & 4 ? b.max[2] : b.min[2]];
  for (const [a, z] of [[0, 1], [2, 3], [4, 5], [6, 7], [0, 2], [1, 3], [4, 6], [5, 7], [0, 4], [1, 5], [2, 6], [3, 7]] as const) out.push(...c(a), ...c(z));
}

function multiply(a: Float32Array, b: Float32Array): Float32Array {
  const out = new Float32Array(16);
  for (let c = 0; c < 4; c++) {
    for (let r = 0; r < 4; r++) {
      let sum = 0;
      for (let k = 0; k < 4; k++) sum += a[k * 4 + r]! * b[c * 4 + k]!;
      out[c * 4 + r] = sum;
    }
  }
  return out;
}

/** Object uniforms (model = uniform scale, linear colour, selected flag); same layout in WGSL and GLSL. */
export function objectUniforms(item: DrawItem, data: Float32Array) {
  data.fill(0);
  data[0] = item.scale;
  data[5] = item.scale;
  data[10] = item.scale;
  data[15] = 1;
  data.set(srgbToLinear(item.color), 16);
  data[19] = 1;
  data[20] = item.selected ? 1 : 0;
}

/**
 * Scene bookkeeping shared by the WebGPU and WebGL2 renderers: camera, frame loop, fit bounds,
 * projection, picking and PNG capture. Subclasses own the GPU resources and draw the frame.
 */
export abstract class RendererBase {
  readonly camera = new OrbitCamera();
  gridVisible = true;
  onDeviceLost: ((reason: string) => void) | null = null;
  /** Fraction of the viewport not covered by overlays (used to frame "fit"). */
  safe = { w: 1, h: 1 };
  abstract readonly api: RendererApi;
  abstract readonly adapterLabel: string;
  abstract readonly maxTextureSize: number;

  protected items: DrawItem[] = [];
  protected theme: ViewportTheme | null = null;
  protected benchBoxes: Bounds[] = [];
  protected readonly frameData = new Float32Array(FRAME_FLOATS);
  protected destroyed = false;
  protected abstract readonly layers: SceneLayers;
  private readonly frameListeners = new Set<() => void>();
  private raf = 0;

  protected constructor(protected readonly canvas: HTMLCanvasElement) {}

  abstract setTheme(theme: ViewportTheme): void;
  abstract setItems(items: DrawItem[]): void;
  /** Workbench boxes (field-free flight volumes), drawn as lines; part of "fit". */
  abstract setBenchBoxes(boxes: Bounds[]): void;
  abstract setPaItems(items: PaDrawItem[]): void;
  abstract setTrajectories(items: TrajectoryItem[]): void;
  abstract setHighlight(h: Highlight | null): void;
  /** Draws one frame into the canvas. */
  protected abstract draw(): void;
  /** Frees the GPU resources (destroy() has already stopped the frame loop). */
  protected abstract release(): void;

  /** Ion end point or drawn path under a CSS-px point of the canvas, within `tolerance` CSS px. */
  pickIon(cssX: number, cssY: number, tolerance: number): IonHit | null {
    const dpr = this.canvas.width / Math.max(1, this.canvas.getBoundingClientRect().width);
    return this.layers.trajectories.pick(this.viewProj(), this.canvas.width, this.canvas.height, cssX * dpr, cssY * dpr, tolerance * dpr);
  }

  /**
   * PNG of the 3D view on the viewport background (the canvas itself is transparent). The frame is
   * drawn and copied in the same task, while the canvas still holds it.
   */
  async capture(background: { center: string; edge: string }): Promise<Blob> {
    this.draw();
    const frame = await createImageBitmap(this.canvas);
    const { width, height } = this.canvas;
    const out = document.createElement("canvas");
    out.width = width;
    out.height = height;
    const ctx = out.getContext("2d")!;
    // Same as the viewport CSS: radial-gradient(ellipse 120% 90% at 50% 40%, center, edge).
    const rx = 1.2 * width;
    const ry = 0.9 * height;
    ctx.save();
    ctx.translate(width / 2, height * 0.4);
    ctx.scale(1, ry / rx);
    const g = ctx.createRadialGradient(0, 0, 0, 0, 0, rx);
    g.addColorStop(0, background.center);
    g.addColorStop(1, background.edge);
    ctx.fillStyle = g;
    ctx.fillRect(-width, -height * (rx / ry), width * 2, height * 2 * (rx / ry));
    ctx.restore();
    ctx.drawImage(frame, 0, 0);
    frame.close();
    return new Promise((resolve, reject) => out.toBlob((b) => (b ? resolve(b) : reject(new Error("PNG encoding failed"))), "image/png"));
  }

  /** Keeps the scene centred in the part of the canvas that overlays leave free. */
  setInsets(insets: Insets, cssWidth: number, cssHeight: number) {
    if (cssWidth <= 0 || cssHeight <= 0) return;
    const w = Math.max(0.2, (cssWidth - insets.left - insets.right) / cssWidth);
    const h = Math.max(0.2, (cssHeight - insets.top - insets.bottom) / cssHeight);
    this.safe = { w, h };
    this.camera.lens = [(insets.left - insets.right) / cssWidth, (insets.bottom - insets.top) / cssHeight];
    this.requestRender();
  }

  /** Bounding boxes of potential arrays (click selection picks them by box). */
  paPickables() {
    return this.layers.pa.pickables();
  }

  /** Section value under the cursor (ndc, y up), or null. */
  probe(ndcX: number, ndcY: number): ProbeHit | null {
    const ray = this.camera.ray(ndcX, ndcY, this.aspect());
    return this.layers.pa.probe(ray.origin, ray.dir);
  }

  /** World-space bounds of visible items, optionally restricted to some ids. */
  bounds(ids?: readonly string[]): Bounds | null {
    let b: Bounds | null = null;
    const pa = this.layers.pa.bounds(ids);
    if (pa) b = unionBounds(b, pa);
    const flights = this.layers.trajectories.bounds(ids);
    if (flights) b = unionBounds(b, flights);
    if (!ids) for (const box of this.benchBoxes) b = unionBounds(b, box);
    for (const item of this.items) {
      if (!item.visible || (ids && !ids.includes(item.id))) continue;
      b = unionBounds(b, scaleBounds(item.mesh.bounds, item.scale));
    }
    return b;
  }

  /** Canvas size in CSS px. */
  cssSize(): { width: number; height: number } {
    const rect = this.canvas.getBoundingClientRect();
    return { width: rect.width, height: rect.height };
  }

  /** CSS px (from the canvas' top-left) of a world point (mm), or null when it is behind the camera. */
  project(p: readonly [number, number, number]): [number, number] | null {
    const m = this.viewProj();
    const x = m[0]! * p[0] + m[4]! * p[1] + m[8]! * p[2] + m[12]!;
    const y = m[1]! * p[0] + m[5]! * p[1] + m[9]! * p[2] + m[13]!;
    const w = m[3]! * p[0] + m[7]! * p[1] + m[11]! * p[2] + m[15]!;
    if (w <= 1e-9) return null;
    const rect = this.canvas.getBoundingClientRect();
    return [(x / w * 0.5 + 0.5) * rect.width, (0.5 - (y / w) * 0.5) * rect.height];
  }

  /** Called after every drawn frame (the axes gizmo follows the camera through this). */
  addFrameListener(listener: () => void): () => void {
    this.frameListeners.add(listener);
    return () => this.frameListeners.delete(listener);
  }

  drawItems(): readonly DrawItem[] {
    return this.items;
  }

  aspect(): number {
    return this.canvas.height > 0 ? this.canvas.width / this.canvas.height : 1;
  }

  requestRender() {
    if (this.destroyed || this.raf) return;
    this.raf = requestAnimationFrame(this.frame);
  }

  destroy() {
    this.destroyed = true;
    cancelAnimationFrame(this.raf);
    this.release();
  }

  private readonly frame = (now: number) => {
    this.raf = 0;
    const animating = this.camera.update(now);
    this.draw();
    for (const listener of this.frameListeners) listener();
    if (animating) this.requestRender();
  };

  protected viewProj(): Float32Array {
    const cam = this.camera;
    const bounds = this.bounds();
    const sceneRadius = bounds ? Math.hypot(...boundsSize(bounds)) / 2 : 100;
    return multiply(cam.projectionMatrix(this.aspect(), sceneRadius), cam.viewMatrix());
  }

  /** Fills `frameData` (the Frame struct of shaders.ts / glShaders.ts). */
  protected writeFrameData() {
    const cam = this.camera;
    const viewProj = this.viewProj();
    const { right, up, back } = cam.basis();
    const eye = cam.eye();
    const t = this.theme;
    const d = this.frameData;
    d.set(viewProj, 0);
    d.set([eye[0], eye[1], eye[2], cam.orthographic ? 1 : 0], 16);
    d.set([...back, 0], 20);
    d.set([...right, 0], 24);
    d.set([...up, 0], 28);
    d.set([...(t?.grid ?? [0.5, 0.5, 0.5]), GRID_OPACITY], 32);
    const lod = Math.log10(Math.max((2 * cam.halfHeight()) / 40, 1e-9));
    d.set([lod, cam.distance * 6, cam.target[0], cam.target[2]], 36);
    d.set([...(t?.axisX ?? [1, 0, 0]), 1], 40);
    d.set([...(t?.axisZ ?? [0, 0, 1]), 1], 44);
    d.set([...(t?.axisY ?? [0, 1, 0]), 1], 48);
    d.set([...srgbToLinear(t?.selection ?? [1, 0.6, 0.2]), 1], 52);
    d.set([...srgbToLinear(t?.sky ?? [1, 1, 1]), 1], 56);
    d.set([...srgbToLinear(t?.ground ?? [0.3, 0.3, 0.3]), 1], 60);
  }
}
