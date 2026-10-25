import { BYTES_PER_VERTEX, boundsSize, scaleBounds, unionBounds, type Bounds, type MeshData } from "../scene/mesh";
import { OrbitCamera } from "./camera";
import { srgbToLinear, type Rgb, type ViewportTheme } from "./color";
import { PaLayer, type PaDrawItem, type ProbeHit } from "./paLayer";
import { axisShader, BENCH_UNIFORM_FLOATS, benchShader, FRAME_FLOATS, gridShader, meshShader } from "./shaders";
import { TrajectoryLayer, type Highlight, type IonHit, type TrajectoryItem } from "./trajectoryLayer";

/** CSS px of the viewport covered by overlays on each side (see Viewport's data-inset elements). */
export interface Insets {
  left: number;
  right: number;
  top: number;
  bottom: number;
}

/** WebGPU is missing entirely (no navigator.gpu / no adapter). The UI shows the fallback card. */
export class WebGpuUnavailableError extends Error {}

/** The owner unmounted while the device was being requested. */
export class RendererCancelledError extends Error {}

export interface DrawItem {
  id: string;
  mesh: MeshData;
  /** sRGB base colour. */
  color: Rgb;
  visible: boolean;
  selected: boolean;
  scale: number;
}

interface GpuObject {
  mesh: MeshData;
  vertexBuffer: GPUBuffer;
  uniformBuffer: GPUBuffer;
  bindGroup: GPUBindGroup;
}

const SAMPLE_COUNT = 4;
const DEPTH_FORMAT: GPUTextureFormat = "depth32float";
const OBJECT_FLOATS = 24;
const GRID_OPACITY = 0.32;
const BENCH_OPACITY = 0.85;

/** The 12 edges of a box, as line-list vertex pairs. */
function boxLines(b: Bounds, out: number[]) {
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

export class Renderer {
  readonly camera = new OrbitCamera();
  gridVisible = true;
  onDeviceLost: ((reason: string) => void) | null = null;
  private readonly frameListeners = new Set<() => void>();

  private readonly objects = new Map<string, GpuObject>();
  private items: DrawItem[] = [];
  private theme: ViewportTheme | null = null;
  private readonly frameData = new Float32Array(FRAME_FLOATS);
  private readonly frameBuffer: GPUBuffer;
  private readonly frameBindGroup: GPUBindGroup;
  private readonly objectLayout: GPUBindGroupLayout;
  private readonly meshPipeline: GPURenderPipeline;
  private readonly gridPipeline: GPURenderPipeline;
  private readonly axisPipeline: GPURenderPipeline;
  private readonly benchPipeline: GPURenderPipeline;
  private readonly benchUniform: GPUBuffer;
  private readonly benchBindGroup: GPUBindGroup;
  private benchBuffer: GPUBuffer | null = null;
  private benchVertices = 0;
  private benchBoxes: Bounds[] = [];
  private readonly pa: PaLayer;
  private readonly trajectories: TrajectoryLayer;
  /** Fraction of the viewport not covered by overlays (used to frame "fit"). */
  safe = { w: 1, h: 1 };
  private msaaTexture: GPUTexture | null = null;
  private depthTexture: GPUTexture | null = null;
  private raf = 0;
  private destroyed = false;

  static async create(canvas: HTMLCanvasElement, isCancelled: () => boolean): Promise<Renderer> {
    if (!navigator.gpu) {
      throw new WebGpuUnavailableError("navigator.gpu is undefined: this browser or WebView does not expose WebGPU.");
    }
    const adapter = await navigator.gpu.requestAdapter({ powerPreference: "high-performance" });
    if (!adapter) {
      throw new WebGpuUnavailableError("navigator.gpu.requestAdapter() returned null: no compatible GPU adapter.");
    }
    const device = await adapter.requestDevice();
    if (isCancelled()) {
      device.destroy();
      throw new RendererCancelledError();
    }
    const context = canvas.getContext("webgpu");
    if (!context) throw new WebGpuUnavailableError("canvas.getContext('webgpu') returned null.");
    const format = navigator.gpu.getPreferredCanvasFormat();
    context.configure({ device, format, alphaMode: "premultiplied" });

    const info = adapter.info;
    const label = info.description || [info.vendor, info.architecture].filter(Boolean).join(" ") || "GPU";

    device.pushErrorScope("validation");
    const renderer = new Renderer(device, context, format, canvas, label);
    const error = await device.popErrorScope();
    if (error) {
      renderer.destroy();
      throw new Error(`WebGPU pipeline setup failed: ${error.message}`);
    }
    return renderer;
  }

  private constructor(
    private readonly device: GPUDevice,
    private readonly context: GPUCanvasContext,
    private readonly format: GPUTextureFormat,
    private readonly canvas: HTMLCanvasElement,
    readonly adapterLabel: string,
  ) {
    device.lost.then((info) => {
      if (!this.destroyed) this.onDeviceLost?.(info.message || info.reason || "unknown");
    });

    const uniformEntry: GPUBindGroupLayoutEntry = {
      binding: 0,
      visibility: GPUShaderStage.VERTEX | GPUShaderStage.FRAGMENT,
      buffer: { type: "uniform" },
    };
    const frameLayout = device.createBindGroupLayout({ entries: [uniformEntry] });
    this.objectLayout = device.createBindGroupLayout({ entries: [uniformEntry] });

    this.frameBuffer = device.createBuffer({
      size: FRAME_FLOATS * 4,
      usage: GPUBufferUsage.UNIFORM | GPUBufferUsage.COPY_DST,
    });
    this.frameBindGroup = device.createBindGroup({
      layout: frameLayout,
      entries: [{ binding: 0, resource: { buffer: this.frameBuffer } }],
    });

    const blend: GPUBlendState = {
      color: { srcFactor: "one", dstFactor: "one-minus-src-alpha" },
      alpha: { srcFactor: "one", dstFactor: "one-minus-src-alpha" },
    };
    const multisample = { count: SAMPLE_COUNT };

    const meshModule = device.createShaderModule({ label: "mesh", code: meshShader });
    this.meshPipeline = device.createRenderPipeline({
      label: "mesh",
      layout: device.createPipelineLayout({ bindGroupLayouts: [frameLayout, this.objectLayout] }),
      vertex: {
        module: meshModule,
        entryPoint: "vs",
        buffers: [
          {
            arrayStride: BYTES_PER_VERTEX,
            attributes: [
              { shaderLocation: 0, offset: 0, format: "float32x3" },
              { shaderLocation: 1, offset: 12, format: "float32x3" },
            ],
          },
        ],
      },
      fragment: { module: meshModule, entryPoint: "fs", targets: [{ format }] },
      primitive: { topology: "triangle-list", cullMode: "none" },
      depthStencil: { format: DEPTH_FORMAT, depthWriteEnabled: true, depthCompare: "greater" },
      multisample,
    });

    this.pa = new PaLayer(device, format, frameLayout, DEPTH_FORMAT, SAMPLE_COUNT);
    this.trajectories = new TrajectoryLayer(device, format, frameLayout, DEPTH_FORMAT, SAMPLE_COUNT);

    const overlayLayout = device.createPipelineLayout({ bindGroupLayouts: [frameLayout] });
    const gridModule = device.createShaderModule({ label: "grid", code: gridShader });
    this.gridPipeline = device.createRenderPipeline({
      label: "grid",
      layout: overlayLayout,
      vertex: { module: gridModule, entryPoint: "vs" },
      fragment: { module: gridModule, entryPoint: "fs", targets: [{ format, blend }] },
      primitive: { topology: "triangle-list" },
      depthStencil: { format: DEPTH_FORMAT, depthWriteEnabled: false, depthCompare: "greater" },
      multisample,
    });

    const benchLayout = device.createBindGroupLayout({ entries: [uniformEntry] });
    this.benchUniform = device.createBuffer({ size: BENCH_UNIFORM_FLOATS * 4, usage: GPUBufferUsage.UNIFORM | GPUBufferUsage.COPY_DST });
    this.benchBindGroup = device.createBindGroup({ layout: benchLayout, entries: [{ binding: 0, resource: { buffer: this.benchUniform } }] });
    const benchModule = device.createShaderModule({ label: "bench", code: benchShader });
    this.benchPipeline = device.createRenderPipeline({
      label: "bench",
      layout: device.createPipelineLayout({ bindGroupLayouts: [frameLayout, benchLayout] }),
      vertex: { module: benchModule, entryPoint: "vs", buffers: [{ arrayStride: 12, attributes: [{ shaderLocation: 0, offset: 0, format: "float32x3" }] }] },
      fragment: { module: benchModule, entryPoint: "fs", targets: [{ format, blend }] },
      primitive: { topology: "line-list" },
      depthStencil: { format: DEPTH_FORMAT, depthWriteEnabled: false, depthCompare: "greater-equal" },
      multisample,
    });

    const axisModule = device.createShaderModule({ label: "axis", code: axisShader });
    this.axisPipeline = device.createRenderPipeline({
      label: "axis",
      layout: overlayLayout,
      vertex: { module: axisModule, entryPoint: "vs" },
      fragment: { module: axisModule, entryPoint: "fs", targets: [{ format, blend }] },
      primitive: { topology: "line-list" },
      depthStencil: { format: DEPTH_FORMAT, depthWriteEnabled: false, depthCompare: "greater" },
      multisample,
    });
  }

  setTheme(theme: ViewportTheme) {
    this.theme = theme;
    this.device.queue.writeBuffer(this.benchUniform, 0, new Float32Array([...theme.bench, BENCH_OPACITY]));
    this.pa.setTheme(theme);
    this.trajectories.setTheme(theme);
    this.writeObjectUniforms();
    this.requestRender();
  }

  setItems(items: DrawItem[]) {
    const keep = new Set(items.map((i) => i.id));
    for (const [id, obj] of this.objects) {
      if (!keep.has(id)) {
        obj.vertexBuffer.destroy();
        obj.uniformBuffer.destroy();
        this.objects.delete(id);
      }
    }
    for (const item of items) {
      const existing = this.objects.get(item.id);
      if (existing && existing.mesh === item.mesh) continue;
      existing?.vertexBuffer.destroy();
      const vertexBuffer = this.device.createBuffer({
        label: item.id,
        size: Math.max(item.mesh.vertices.byteLength, 4),
        usage: GPUBufferUsage.VERTEX | GPUBufferUsage.COPY_DST,
      });
      this.device.queue.writeBuffer(vertexBuffer, 0, item.mesh.vertices);
      const uniformBuffer =
        existing?.uniformBuffer ??
        this.device.createBuffer({ size: OBJECT_FLOATS * 4, usage: GPUBufferUsage.UNIFORM | GPUBufferUsage.COPY_DST });
      const bindGroup =
        existing?.bindGroup ??
        this.device.createBindGroup({
          layout: this.objectLayout,
          entries: [{ binding: 0, resource: { buffer: uniformBuffer } }],
        });
      this.objects.set(item.id, { mesh: item.mesh, vertexBuffer, uniformBuffer, bindGroup });
    }
    this.items = items;
    this.writeObjectUniforms();
    this.requestRender();
  }

  /** Workbench boxes (field-free flight volumes), drawn as lines; part of "fit". */
  setBenchBoxes(boxes: Bounds[]) {
    this.benchBoxes = boxes;
    const lines: number[] = [];
    for (const b of boxes) boxLines(b, lines);
    this.benchBuffer?.destroy();
    this.benchBuffer = null;
    this.benchVertices = lines.length / 3;
    if (lines.length) {
      this.benchBuffer = this.device.createBuffer({ label: "bench", size: lines.length * 4, usage: GPUBufferUsage.VERTEX | GPUBufferUsage.COPY_DST });
      this.device.queue.writeBuffer(this.benchBuffer, 0, new Float32Array(lines));
    }
    this.requestRender();
  }

  setPaItems(items: PaDrawItem[]) {
    this.pa.setItems(items);
    this.requestRender();
  }

  setTrajectories(items: TrajectoryItem[]) {
    this.trajectories.setItems(items);
    this.requestRender();
  }

  setHighlight(h: Highlight | null) {
    this.trajectories.setHighlight(h);
    this.requestRender();
  }

  /** Ion end point or drawn path under a CSS-px point of the canvas, within `tolerance` CSS px. */
  pickIon(cssX: number, cssY: number, tolerance: number): IonHit | null {
    const dpr = this.canvas.width / Math.max(1, this.canvas.getBoundingClientRect().width);
    return this.trajectories.pick(this.viewProj(), this.canvas.width, this.canvas.height, cssX * dpr, cssY * dpr, tolerance * dpr);
  }

  /**
   * PNG of the 3D view on the viewport background (the canvas itself is transparent). The frame is
   * drawn and copied in the same task, while the WebGPU canvas still holds it.
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
    return this.pa.pickables();
  }

  /** Section value under the cursor (ndc, y up), or null. */
  probe(ndcX: number, ndcY: number): ProbeHit | null {
    const ray = this.camera.ray(ndcX, ndcY, this.aspect());
    return this.pa.probe(ray.origin, ray.dir);
  }

  /** World-space bounds of visible items, optionally restricted to some ids. */
  bounds(ids?: readonly string[]): Bounds | null {
    let b: Bounds | null = null;
    const pa = this.pa.bounds(ids);
    if (pa) b = unionBounds(b, pa);
    const flights = this.trajectories.bounds(ids);
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

  get maxTextureSize(): number {
    return this.device.limits.maxTextureDimension2D;
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
    for (const obj of this.objects.values()) {
      obj.vertexBuffer.destroy();
      obj.uniformBuffer.destroy();
    }
    this.objects.clear();
    this.pa.destroy();
    this.trajectories.destroy();
    this.benchBuffer?.destroy();
    this.benchUniform.destroy();
    this.msaaTexture?.destroy();
    this.depthTexture?.destroy();
    this.frameBuffer.destroy();
    this.device.destroy();
  }

  private readonly frame = (now: number) => {
    this.raf = 0;
    const animating = this.camera.update(now);
    this.draw();
    for (const listener of this.frameListeners) listener();
    if (animating) this.requestRender();
  };

  private writeObjectUniforms() {
    const data = new Float32Array(OBJECT_FLOATS);
    for (const item of this.items) {
      const obj = this.objects.get(item.id);
      if (!obj) continue;
      data.fill(0);
      data[0] = item.scale;
      data[5] = item.scale;
      data[10] = item.scale;
      data[15] = 1;
      data.set(srgbToLinear(item.color), 16);
      data[19] = 1;
      data[20] = item.selected ? 1 : 0;
      this.device.queue.writeBuffer(obj.uniformBuffer, 0, data);
    }
  }

  private ensureTargets(width: number, height: number) {
    if (this.msaaTexture && this.msaaTexture.width === width && this.msaaTexture.height === height) return;
    this.msaaTexture?.destroy();
    this.depthTexture?.destroy();
    this.msaaTexture = this.device.createTexture({
      size: [width, height],
      sampleCount: SAMPLE_COUNT,
      format: this.format,
      usage: GPUTextureUsage.RENDER_ATTACHMENT,
    });
    this.depthTexture = this.device.createTexture({
      size: [width, height],
      sampleCount: SAMPLE_COUNT,
      format: DEPTH_FORMAT,
      usage: GPUTextureUsage.RENDER_ATTACHMENT,
    });
  }

  private viewProj(): Float32Array {
    const cam = this.camera;
    const bounds = this.bounds();
    const sceneRadius = bounds ? Math.hypot(...boundsSize(bounds)) / 2 : 100;
    return multiply(cam.projectionMatrix(this.aspect(), sceneRadius), cam.viewMatrix());
  }

  private writeFrameUniforms() {
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
    this.device.queue.writeBuffer(this.frameBuffer, 0, d);
  }

  private draw() {
    if (this.destroyed) return;
    const { width, height } = this.canvas;
    if (width === 0 || height === 0) return;
    this.ensureTargets(width, height);
    this.writeFrameUniforms();

    const encoder = this.device.createCommandEncoder();
    const pass = encoder.beginRenderPass({
      colorAttachments: [
        {
          view: this.msaaTexture!.createView(),
          resolveTarget: this.context.getCurrentTexture().createView(),
          clearValue: { r: 0, g: 0, b: 0, a: 0 },
          loadOp: "clear",
          storeOp: "discard",
        },
      ],
      depthStencilAttachment: {
        view: this.depthTexture!.createView(),
        depthClearValue: 0,
        depthLoadOp: "clear",
        depthStoreOp: "discard",
      },
    });
    pass.setBindGroup(0, this.frameBindGroup);

    pass.setPipeline(this.meshPipeline);
    for (const item of this.items) {
      const obj = this.objects.get(item.id);
      if (!item.visible || !obj || item.mesh.triangleCount === 0) continue;
      pass.setBindGroup(1, obj.bindGroup);
      pass.setVertexBuffer(0, obj.vertexBuffer);
      pass.draw(item.mesh.triangleCount * 3);
    }
    this.pa.draw(pass, this.camera.eye());
    if (this.benchBuffer && this.benchVertices) {
      pass.setPipeline(this.benchPipeline);
      pass.setBindGroup(1, this.benchBindGroup);
      pass.setVertexBuffer(0, this.benchBuffer);
      pass.draw(this.benchVertices);
    }
    this.trajectories.draw(pass, width, height);

    if (this.gridVisible) {
      pass.setPipeline(this.gridPipeline);
      pass.draw(6);
      pass.setPipeline(this.axisPipeline);
      pass.draw(6);
    }
    pass.end();
    this.device.queue.submit([encoder.finish()]);
  }
}
