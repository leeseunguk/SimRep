import { BYTES_PER_VERTEX, type Bounds, type MeshData } from "../scene/mesh";
import type { ViewportTheme } from "./color";
import { WebGl2Renderer } from "./glRenderer";
import { PaLayer, type PaDrawItem } from "./paLayer";
import { BENCH_OPACITY, boxLines, objectUniforms, OBJECT_FLOATS, RendererBase, type DrawItem } from "./rendererBase";
import { RendererCancelledError, selectRenderer, WebGpuUnavailableError } from "./rendererSelect";
import { axisShader, BENCH_UNIFORM_FLOATS, benchShader, FRAME_FLOATS, gridShader, meshShader } from "./shaders";
import { TrajectoryLayer, type Highlight, type TrajectoryItem } from "./trajectoryLayer";

export type { DrawItem, Insets, RendererApi } from "./rendererBase";
export { RendererCancelledError, RendererUnavailableError, WebGpuUnavailableError } from "./rendererSelect";
/** The 3D view's renderer, whichever graphics API draws it. */
export type Renderer = RendererBase;

/**
 * WebGPU when the browser or WebView has an adapter, else the WebGL2 port of the same passes.
 * Throws RendererUnavailableError (the fallback card) when neither can draw.
 */
export function createRenderer(canvas: HTMLCanvasElement, isCancelled: () => boolean): Promise<{ renderer: Renderer; webgpuReason: string | null }> {
  return selectRenderer<Renderer>({
    webgpu: () => WebGpuRenderer.create(canvas, isCancelled),
    webgl2: () => WebGl2Renderer.create(canvas),
  });
}

interface GpuObject {
  mesh: MeshData;
  vertexBuffer: GPUBuffer;
  uniformBuffer: GPUBuffer;
  bindGroup: GPUBindGroup;
}

const SAMPLE_COUNT = 4;
const DEPTH_FORMAT: GPUTextureFormat = "depth32float";

export class WebGpuRenderer extends RendererBase {
  readonly api = "webgpu" as const;
  private readonly objects = new Map<string, GpuObject>();
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
  private readonly pa: PaLayer;
  private readonly trajectories: TrajectoryLayer;
  protected readonly layers: { pa: PaLayer; trajectories: TrajectoryLayer };
  private msaaTexture: GPUTexture | null = null;
  private depthTexture: GPUTexture | null = null;

  static async create(canvas: HTMLCanvasElement, isCancelled: () => boolean): Promise<WebGpuRenderer> {
    if (!navigator.gpu) {
      throw new WebGpuUnavailableError("navigator.gpu is undefined: this browser or WebView does not expose WebGPU.");
    }
    const adapter = await navigator.gpu.requestAdapter({ powerPreference: "high-performance" });
    if (!adapter) {
      throw new WebGpuUnavailableError("navigator.gpu.requestAdapter() returned null: no compatible GPU adapter.");
    }
    // Until a WebGPU context is taken the canvas can still fall back to WebGL2.
    const device = await adapter.requestDevice().catch((err: unknown) => {
      throw new WebGpuUnavailableError(`adapter.requestDevice() failed: ${err instanceof Error ? err.message : String(err)}`);
    });
    if (isCancelled()) {
      device.destroy();
      throw new RendererCancelledError();
    }
    const context = canvas.getContext("webgpu");
    if (!context) {
      device.destroy();
      throw new WebGpuUnavailableError("canvas.getContext('webgpu') returned null.");
    }
    const format = navigator.gpu.getPreferredCanvasFormat();
    context.configure({ device, format, alphaMode: "premultiplied" });

    const info = adapter.info;
    const label = info.description || [info.vendor, info.architecture].filter(Boolean).join(" ") || "GPU";

    device.pushErrorScope("validation");
    const renderer = new WebGpuRenderer(device, context, format, canvas, label);
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
    canvas: HTMLCanvasElement,
    readonly adapterLabel: string,
  ) {
    super(canvas);
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
    this.layers = { pa: this.pa, trajectories: this.trajectories };

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

  get maxTextureSize(): number {
    return this.device.limits.maxTextureDimension2D;
  }

  protected release() {
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

  private writeObjectUniforms() {
    const data = new Float32Array(OBJECT_FLOATS);
    for (const item of this.items) {
      const obj = this.objects.get(item.id);
      if (!obj) continue;
      objectUniforms(item, data);
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

  private writeFrameUniforms() {
    this.writeFrameData();
    this.device.queue.writeBuffer(this.frameBuffer, 0, this.frameData);
  }

  protected draw() {
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
