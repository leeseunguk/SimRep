import { BYTES_PER_VERTEX, type Bounds, type MeshData, type Vec3 } from "../scene/mesh";
import type { PaGrid, SliceImage, SlicePlane } from "../scene/pa";
import { QUAD_FLOATS, type PaSurface } from "../scene/paSurface";
import type { Flight } from "../scene/flight";
import type { ViewportTheme } from "./color";
import { glShaders, type GlProgramSource } from "./glShaders";
import { paBounds, paPickables, paProbe, paSliceUniforms, paSliceUpdate, paSurfaceUniforms, type PaDrawItem, type PaSliceState, type ProbeHit } from "./paLayer";
import { BENCH_OPACITY, boxLines, objectUniforms, OBJECT_FLOATS, RendererBase, type DrawItem } from "./rendererBase";
import { WebGl2UnavailableError } from "./rendererSelect";
import { BENCH_UNIFORM_FLOATS, PA_UNIFORM_FLOATS, SLICE_UNIFORM_FLOATS, TRAJECTORY_UNIFORM_FLOATS } from "./shaders";
import {
  trajectoryBounds,
  trajectoryGeometry,
  trajectoryPick,
  trajectoryUniforms,
  type Highlight,
  type IonHit,
  type TrajectoryItem,
} from "./trajectoryLayer";

// WebGL2 port of the WebGPU renderer (renderer.ts): the same passes in the same order, the same
// uniform layouts (std140 = WGSL uniform layout), 4x MSAA with a float depth buffer and
// reversed Z, resolved into the canvas by a blit. Used when the browser has no WebGPU adapter.

const SAMPLE_COUNT = 4;
const FRAME_BINDING = 0;
const BLOCK_BINDING = 1;

interface ClipControl {
  clipControlEXT(origin: number, depth: number): void;
  readonly LOWER_LEFT_EXT: number;
  readonly ZERO_TO_ONE_EXT: number;
}

type Depth = { func: number; write: boolean };

class GlProgram {
  readonly program: WebGLProgram;
  constructor(gl: WebGL2RenderingContext, label: string, src: GlProgramSource, block: string | null) {
    const compile = (type: number, code: string) => {
      const s = gl.createShader(type)!;
      gl.shaderSource(s, code);
      gl.compileShader(s);
      if (!gl.getShaderParameter(s, gl.COMPILE_STATUS) && !gl.isContextLost())
        throw new Error(`WebGL2 ${label} ${type === gl.VERTEX_SHADER ? "vertex" : "fragment"} shader: ${gl.getShaderInfoLog(s)}`);
      return s;
    };
    const p = gl.createProgram()!;
    const vs = compile(gl.VERTEX_SHADER, src.vs);
    const fs = compile(gl.FRAGMENT_SHADER, src.fs);
    gl.attachShader(p, vs);
    gl.attachShader(p, fs);
    gl.linkProgram(p);
    gl.deleteShader(vs);
    gl.deleteShader(fs);
    if (!gl.getProgramParameter(p, gl.LINK_STATUS) && !gl.isContextLost()) throw new Error(`WebGL2 ${label} program: ${gl.getProgramInfoLog(p)}`);
    const frame = gl.getUniformBlockIndex(p, "Frame");
    if (frame !== gl.INVALID_INDEX) gl.uniformBlockBinding(p, frame, FRAME_BINDING);
    if (block) {
      const i = gl.getUniformBlockIndex(p, block);
      if (i !== gl.INVALID_INDEX) gl.uniformBlockBinding(p, i, BLOCK_BINDING);
    }
    this.program = p;
  }
}

/** Shared GL state helpers of the layers. */
class GlContext {
  readonly blockUbo: Map<number, WebGLBuffer> = new Map();
  constructor(readonly gl: WebGL2RenderingContext) {}

  /** Uploads one block's uniforms (scratch buffer per size) and binds it to BLOCK_BINDING. */
  block(data: Float32Array) {
    const gl = this.gl;
    let ubo = this.blockUbo.get(data.length);
    if (!ubo) {
      ubo = gl.createBuffer()!;
      gl.bindBuffer(gl.UNIFORM_BUFFER, ubo);
      gl.bufferData(gl.UNIFORM_BUFFER, data.byteLength, gl.DYNAMIC_DRAW);
      this.blockUbo.set(data.length, ubo);
    }
    gl.bindBuffer(gl.UNIFORM_BUFFER, ubo);
    gl.bufferSubData(gl.UNIFORM_BUFFER, 0, data);
    gl.bindBufferBase(gl.UNIFORM_BUFFER, BLOCK_BINDING, ubo);
  }

  use(p: GlProgram, depth: Depth, blend: boolean, offset = false) {
    const gl = this.gl;
    gl.useProgram(p.program);
    gl.depthFunc(depth.func);
    gl.depthMask(depth.write);
    if (blend) gl.enable(gl.BLEND);
    else gl.disable(gl.BLEND);
    if (offset) gl.enable(gl.POLYGON_OFFSET_FILL);
    else gl.disable(gl.POLYGON_OFFSET_FILL);
  }

  buffer(target: number, data: ArrayBufferView<ArrayBuffer>): WebGLBuffer {
    const gl = this.gl;
    const b = gl.createBuffer()!;
    gl.bindBuffer(target, b);
    gl.bufferData(target, data, gl.STATIC_DRAW);
    return b;
  }

  destroy() {
    for (const b of this.blockUbo.values()) this.gl.deleteBuffer(b);
    this.blockUbo.clear();
  }
}

interface GlPa extends PaSliceState {
  surface: PaSurface;
  quadBuffer: WebGLBuffer | null;
  vao: WebGLVertexArrayObject | null;
  sliceTexture: WebGLTexture | null;
  sliceKey: string;
  grid: PaGrid | null;
  labels: Uint16Array | null;
}

/** Electrode surfaces and potential sections (GL counterpart of PaLayer). */
class GlPaLayer {
  private readonly gpu = new Map<string, GlPa>();
  private items: PaDrawItem[] = [];
  private theme: ViewportTheme | null = null;
  private readonly surfaceData = new Float32Array(PA_UNIFORM_FLOATS);
  private readonly sliceData = new Float32Array(SLICE_UNIFORM_FLOATS);
  private readonly emptyVao: WebGLVertexArrayObject;

  constructor(
    private readonly c: GlContext,
    private readonly surfaceProgram: GlProgram,
    private readonly sliceProgram: GlProgram,
    private readonly maxTextureSize: number,
  ) {
    this.emptyVao = c.gl.createVertexArray()!;
  }

  setTheme(theme: ViewportTheme) {
    this.theme = theme;
  }

  setItems(items: PaDrawItem[]) {
    const keep = new Set(items.map((i) => i.id));
    for (const [id, g] of this.gpu) {
      if (!keep.has(id)) {
        this.release(g);
        this.gpu.delete(id);
      }
    }
    for (const item of items) {
      let g = this.gpu.get(item.id);
      if (!g) {
        g = { surface: item.surface, quadBuffer: null, vao: null, sliceTexture: null, sliceKey: "", grid: null, labels: null, plane: null, image: null };
        this.uploadSurface(g, item.surface);
        this.gpu.set(item.id, g);
      } else if (g.surface !== item.surface) {
        this.releaseSurface(g);
        this.uploadSurface(g, item.surface);
      }
      const image = paSliceUpdate(g, item, this.maxTextureSize);
      if (image) this.uploadSlice(g, image);
    }
    this.items = items;
  }

  bounds(ids?: readonly string[]): Bounds | null {
    return paBounds(this.items, ids);
  }

  pickables() {
    return paPickables(this.items);
  }

  probe(origin: Vec3, dir: Vec3): ProbeHit | null {
    return paProbe(this.items, (id) => this.gpu.get(id), origin, dir);
  }

  draw(eye: Vec3) {
    const theme = this.theme;
    if (!theme) return;
    const gl = this.c.gl;
    for (const item of this.items) {
      const g = this.gpu.get(item.id);
      if (!item.visible || !g) continue;
      const plane: SlicePlane | null = g.plane;
      if (g.vao && item.surface.count > 0) {
        this.c.use(this.surfaceProgram, { func: gl.GREATER, write: true }, false);
        gl.bindVertexArray(g.vao);
        paSurfaceUniforms(item, plane, eye, theme, this.surfaceData, (d) => {
          this.c.block(d);
          gl.drawArraysInstanced(gl.TRIANGLES, 0, 6, item.surface.count);
        });
      }
      if (item.slice.visible && plane && g.image && g.sliceTexture) {
        paSliceUniforms(item, plane, g.image, theme, this.sliceData);
        // Reversed Z: a positive offset pulls the section in front of coplanar electrode faces.
        this.c.use(this.sliceProgram, { func: gl.GREATER, write: true }, false, true);
        this.c.block(this.sliceData);
        gl.activeTexture(gl.TEXTURE0);
        gl.bindTexture(gl.TEXTURE_2D, g.sliceTexture);
        gl.bindVertexArray(this.emptyVao);
        gl.drawArrays(gl.TRIANGLES, 0, 6);
      }
    }
    gl.disable(gl.POLYGON_OFFSET_FILL);
  }

  destroy() {
    for (const g of this.gpu.values()) this.release(g);
    this.gpu.clear();
    this.c.gl.deleteVertexArray(this.emptyVao);
  }

  private uploadSurface(g: GlPa, surface: PaSurface) {
    g.surface = surface;
    if (surface.count === 0) return;
    const gl = this.c.gl;
    g.vao = gl.createVertexArray()!;
    gl.bindVertexArray(g.vao);
    g.quadBuffer = this.c.buffer(gl.ARRAY_BUFFER, surface.quads);
    const stride = QUAD_FLOATS * 4;
    for (let loc = 0; loc < 4; loc++) {
      gl.enableVertexAttribArray(loc);
      gl.vertexAttribPointer(loc, 3, gl.FLOAT, false, stride, loc * 12);
      gl.vertexAttribDivisor(loc, 1);
    }
    for (const [loc, offset] of [[4, 48], [5, 52]] as const) {
      gl.enableVertexAttribArray(loc);
      gl.vertexAttribPointer(loc, 1, gl.FLOAT, false, stride, offset);
      gl.vertexAttribDivisor(loc, 1);
    }
    gl.bindVertexArray(null);
  }

  private uploadSlice(g: GlPa, image: SliceImage) {
    const gl = this.c.gl;
    if (!g.sliceTexture) {
      g.sliceTexture = gl.createTexture()!;
      gl.bindTexture(gl.TEXTURE_2D, g.sliceTexture);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.NEAREST);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.NEAREST);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
    }
    gl.bindTexture(gl.TEXTURE_2D, g.sliceTexture);
    gl.pixelStorei(gl.UNPACK_ALIGNMENT, 4);
    gl.texImage2D(gl.TEXTURE_2D, 0, gl.RG32F, image.width, image.height, 0, gl.RG, gl.FLOAT, image.data);
  }

  private releaseSurface(g: GlPa) {
    const gl = this.c.gl;
    if (g.quadBuffer) gl.deleteBuffer(g.quadBuffer);
    if (g.vao) gl.deleteVertexArray(g.vao);
    g.quadBuffer = null;
    g.vao = null;
  }

  private release(g: GlPa) {
    this.releaseSurface(g);
    if (g.sliceTexture) this.c.gl.deleteTexture(g.sliceTexture);
  }
}

interface GlTrajectory {
  flight: Flight;
  buffers: WebGLBuffer[];
  lineVao: WebGLVertexArrayObject | null;
  indexCount: number;
  splatVao: WebGLVertexArrayObject | null;
  splatCount: number;
  bounds: Bounds | null;
}

/** Ion trajectories, splats and the selected-ion highlight (GL counterpart of TrajectoryLayer). */
class GlTrajectoryLayer {
  private readonly gpu = new Map<string, GlTrajectory>();
  private items: TrajectoryItem[] = [];
  private theme: ViewportTheme | null = null;
  private readonly data = new Float32Array(TRAJECTORY_UNIFORM_FLOATS);
  private highlight: { buffers: WebGLBuffer[]; thickVao: WebGLVertexArrayObject | null; segments: number; ringVao: WebGLVertexArrayObject } | null = null;

  constructor(
    private readonly c: GlContext,
    private readonly p: { line: GlProgram; splat: GlProgram; thick: GlProgram; ring: GlProgram },
  ) {}

  setTheme(theme: ViewportTheme) {
    this.theme = theme;
  }

  setItems(items: TrajectoryItem[]) {
    const keep = new Set(items.map((i) => i.id));
    for (const [id, g] of this.gpu) {
      if (!keep.has(id)) {
        this.release(g);
        this.gpu.delete(id);
      }
    }
    for (const item of items) {
      const g = this.gpu.get(item.id);
      if (g?.flight === item.flight) continue;
      if (g) this.release(g);
      this.gpu.set(item.id, this.upload(item.flight));
    }
    this.items = items;
  }

  setHighlight(h: Highlight | null) {
    const gl = this.c.gl;
    if (this.highlight) {
      for (const b of this.highlight.buffers) gl.deleteBuffer(b);
      if (this.highlight.thickVao) gl.deleteVertexArray(this.highlight.thickVao);
      gl.deleteVertexArray(this.highlight.ringVao);
      this.highlight = null;
    }
    if (!h) return;
    const segments = h.points ? Math.max(0, h.points.length / 3 - 1) : 0;
    const buffers: WebGLBuffer[] = [];
    let thickVao: WebGLVertexArrayObject | null = null;
    if (segments > 0) {
      thickVao = gl.createVertexArray()!;
      gl.bindVertexArray(thickVao);
      buffers.push(this.c.buffer(gl.ARRAY_BUFFER, h.points!));
      // Segment i = points i and i + 1: the same buffer read twice, the second 12 bytes in.
      for (const [loc, offset] of [[0, 0], [1, 12]] as const) {
        gl.enableVertexAttribArray(loc);
        gl.vertexAttribPointer(loc, 3, gl.FLOAT, false, 12, offset);
        gl.vertexAttribDivisor(loc, 1);
      }
    }
    const ringVao = gl.createVertexArray()!;
    gl.bindVertexArray(ringVao);
    buffers.push(this.c.buffer(gl.ARRAY_BUFFER, new Float32Array([...h.end, 0])));
    gl.enableVertexAttribArray(0);
    gl.vertexAttribPointer(0, 4, gl.FLOAT, false, 16, 0);
    gl.vertexAttribDivisor(0, 1);
    gl.bindVertexArray(null);
    this.highlight = { buffers, thickVao, segments, ringVao };
  }

  pick(viewProj: Float32Array, width: number, height: number, px: number, py: number, tolerance: number): IonHit | null {
    return trajectoryPick(this.items, viewProj, width, height, px, py, tolerance);
  }

  bounds(ids?: readonly string[]): Bounds | null {
    return trajectoryBounds(this.items, (id) => this.gpu.get(id)?.bounds ?? null, ids);
  }

  draw(width: number, height: number) {
    const theme = this.theme;
    if (!theme || !this.items.some((i) => i.visible)) return;
    const gl = this.c.gl;
    trajectoryUniforms(this.items, theme, this.highlight !== null, width, height, this.data);
    // Depth-tested against electrodes and the section, but not written: lines don't hide each other.
    const tested: Depth = { func: gl.GEQUAL, write: false };
    this.c.use(this.p.line, tested, true);
    this.c.block(this.data);
    for (const item of this.items) {
      const g = this.gpu.get(item.id);
      if (!item.visible || !g?.lineVao) continue;
      gl.bindVertexArray(g.lineVao);
      // WebGL2 always restarts strips at index 0xffffffff.
      gl.drawElements(gl.LINE_STRIP, g.indexCount, gl.UNSIGNED_INT, 0);
    }
    this.c.use(this.p.splat, tested, true);
    for (const item of this.items) {
      const g = this.gpu.get(item.id);
      if (!item.visible || !g?.splatVao) continue;
      gl.bindVertexArray(g.splatVao);
      gl.drawArraysInstanced(gl.TRIANGLES, 0, 6, g.splatCount);
    }
    const h = this.highlight;
    if (h) {
      // The highlight is drawn over electrodes and other paths: it must stay findable.
      const onTop: Depth = { func: gl.ALWAYS, write: false };
      if (h.thickVao && h.segments > 0) {
        this.c.use(this.p.thick, onTop, true);
        gl.bindVertexArray(h.thickVao);
        gl.drawArraysInstanced(gl.TRIANGLES, 0, 6, h.segments);
      }
      this.c.use(this.p.ring, onTop, true);
      gl.bindVertexArray(h.ringVao);
      gl.drawArraysInstanced(gl.TRIANGLES, 0, 6, 1);
    }
  }

  destroy() {
    this.setHighlight(null);
    for (const g of this.gpu.values()) this.release(g);
    this.gpu.clear();
  }

  private upload(flight: Flight): GlTrajectory {
    const gl = this.c.gl;
    const geo = trajectoryGeometry(flight);
    const buffers: WebGLBuffer[] = [];
    let lineVao: WebGLVertexArrayObject | null = null;
    if (flight.header.points > 0 && geo.indexCount > 0) {
      lineVao = gl.createVertexArray()!;
      gl.bindVertexArray(lineVao);
      buffers.push(this.c.buffer(gl.ARRAY_BUFFER, flight.points));
      gl.enableVertexAttribArray(0);
      gl.vertexAttribPointer(0, 3, gl.FLOAT, false, 12, 0);
      buffers.push(this.c.buffer(gl.ELEMENT_ARRAY_BUFFER, geo.indices));
    }
    let splatVao: WebGLVertexArrayObject | null = null;
    if (geo.splatCount > 0) {
      splatVao = gl.createVertexArray()!;
      gl.bindVertexArray(splatVao);
      buffers.push(this.c.buffer(gl.ARRAY_BUFFER, geo.splats));
      gl.enableVertexAttribArray(0);
      gl.vertexAttribPointer(0, 4, gl.FLOAT, false, 16, 0);
      gl.vertexAttribDivisor(0, 1);
    }
    gl.bindVertexArray(null);
    return { flight, buffers, lineVao, indexCount: geo.indexCount, splatVao, splatCount: geo.splatCount, bounds: geo.bounds };
  }

  private release(g: GlTrajectory) {
    const gl = this.c.gl;
    for (const b of g.buffers) gl.deleteBuffer(b);
    if (g.lineVao) gl.deleteVertexArray(g.lineVao);
    if (g.splatVao) gl.deleteVertexArray(g.splatVao);
  }
}

interface GlObject {
  mesh: MeshData;
  buffer: WebGLBuffer;
  vao: WebGLVertexArrayObject;
}

export class WebGl2Renderer extends RendererBase {
  readonly api = "webgl2" as const;
  readonly maxTextureSize: number;
  protected readonly layers: { pa: GlPaLayer; trajectories: GlTrajectoryLayer };
  private readonly c: GlContext;
  private readonly programs: Record<"mesh" | "grid" | "axis" | "bench", GlProgram>;
  private readonly objects = new Map<string, GlObject>();
  private readonly objectData = new Float32Array(OBJECT_FLOATS);
  private readonly benchData = new Float32Array(BENCH_UNIFORM_FLOATS);
  private readonly frameUbo: WebGLBuffer;
  private readonly emptyVao: WebGLVertexArrayObject;
  private bench: { buffer: WebGLBuffer; vao: WebGLVertexArrayObject; vertices: number } | null = null;
  private targets: { fbo: WebGLFramebuffer; color: WebGLRenderbuffer; depth: WebGLRenderbuffer; width: number; height: number } | null = null;
  private readonly samples: number;
  private readonly onLost = (e: Event) => {
    e.preventDefault();
    if (!this.destroyed) this.onDeviceLost?.("WebGL2 context lost");
  };

  /** Throws WebGl2UnavailableError when the canvas has no WebGL2 context. */
  static create(canvas: HTMLCanvasElement): WebGl2Renderer {
    const gl = canvas.getContext("webgl2", { alpha: true, premultipliedAlpha: true, antialias: false, depth: false, stencil: false });
    if (!gl) throw new WebGl2UnavailableError("canvas.getContext('webgl2') returned null.");
    const debug = gl.getExtension("WEBGL_debug_renderer_info");
    const label = String((debug && gl.getParameter(debug.UNMASKED_RENDERER_WEBGL)) || gl.getParameter(gl.RENDERER) || "WebGL2");
    return new WebGl2Renderer(gl, canvas, label);
  }

  private constructor(
    private readonly gl: WebGL2RenderingContext,
    canvas: HTMLCanvasElement,
    readonly adapterLabel: string,
  ) {
    super(canvas);
    canvas.addEventListener("webglcontextlost", this.onLost);
    this.maxTextureSize = Math.min(gl.getParameter(gl.MAX_TEXTURE_SIZE) as number, gl.getParameter(gl.MAX_RENDERBUFFER_SIZE) as number);
    this.samples = Math.min(SAMPLE_COUNT, gl.getParameter(gl.MAX_SAMPLES) as number);
    // Zero-to-one clip depth like WebGPU when available; the shaders remap z otherwise.
    const clip = gl.getExtension("EXT_clip_control") as ClipControl | null;
    if (clip) clip.clipControlEXT(clip.LOWER_LEFT_EXT, clip.ZERO_TO_ONE_EXT);
    const src = glShaders(clip !== null);

    this.c = new GlContext(gl);
    this.programs = {
      mesh: new GlProgram(gl, "mesh", src.mesh, "Object"),
      grid: new GlProgram(gl, "grid", src.grid, null),
      axis: new GlProgram(gl, "axis", src.axis, null),
      bench: new GlProgram(gl, "bench", src.bench, "Bench"),
    };
    const slice = new GlProgram(gl, "pa-slice", src.slice, "Slice");
    gl.useProgram(slice.program);
    gl.uniform1i(gl.getUniformLocation(slice.program, "data"), 0);
    this.layers = {
      pa: new GlPaLayer(this.c, new GlProgram(gl, "pa-surface", src.paSurface, "Pa"), slice, this.maxTextureSize),
      trajectories: new GlTrajectoryLayer(this.c, {
        line: new GlProgram(gl, "trajectory-lines", src.line, "Traj"),
        splat: new GlProgram(gl, "trajectory-splats", src.splat, "Traj"),
        thick: new GlProgram(gl, "trajectory-highlight", src.thick, "Traj"),
        ring: new GlProgram(gl, "trajectory-highlight-end", src.ring, "Traj"),
      }),
    };

    this.frameUbo = gl.createBuffer()!;
    gl.bindBuffer(gl.UNIFORM_BUFFER, this.frameUbo);
    gl.bufferData(gl.UNIFORM_BUFFER, this.frameData.byteLength, gl.DYNAMIC_DRAW);
    this.emptyVao = gl.createVertexArray()!;

    gl.disable(gl.CULL_FACE);
    gl.enable(gl.DEPTH_TEST);
    gl.blendFunc(gl.ONE, gl.ONE_MINUS_SRC_ALPHA);
    gl.polygonOffset(1, 4);
  }

  setTheme(theme: ViewportTheme) {
    this.theme = theme;
    this.benchData.set([...theme.bench, BENCH_OPACITY]);
    this.layers.pa.setTheme(theme);
    this.layers.trajectories.setTheme(theme);
    this.requestRender();
  }

  setItems(items: DrawItem[]) {
    const gl = this.gl;
    const keep = new Set(items.map((i) => i.id));
    for (const [id, obj] of this.objects) {
      if (!keep.has(id)) {
        gl.deleteBuffer(obj.buffer);
        gl.deleteVertexArray(obj.vao);
        this.objects.delete(id);
      }
    }
    for (const item of items) {
      const existing = this.objects.get(item.id);
      if (existing && existing.mesh === item.mesh) continue;
      if (existing) {
        gl.deleteBuffer(existing.buffer);
        gl.deleteVertexArray(existing.vao);
      }
      const vao = gl.createVertexArray()!;
      gl.bindVertexArray(vao);
      const buffer = this.c.buffer(gl.ARRAY_BUFFER, item.mesh.vertices);
      gl.enableVertexAttribArray(0);
      gl.vertexAttribPointer(0, 3, gl.FLOAT, false, BYTES_PER_VERTEX, 0);
      gl.enableVertexAttribArray(1);
      gl.vertexAttribPointer(1, 3, gl.FLOAT, false, BYTES_PER_VERTEX, 12);
      gl.bindVertexArray(null);
      this.objects.set(item.id, { mesh: item.mesh, buffer, vao });
    }
    this.items = items;
    this.requestRender();
  }

  setBenchBoxes(boxes: Bounds[]) {
    const gl = this.gl;
    this.benchBoxes = boxes;
    const lines: number[] = [];
    for (const b of boxes) boxLines(b, lines);
    if (this.bench) {
      gl.deleteBuffer(this.bench.buffer);
      gl.deleteVertexArray(this.bench.vao);
      this.bench = null;
    }
    if (lines.length) {
      const vao = gl.createVertexArray()!;
      gl.bindVertexArray(vao);
      const buffer = this.c.buffer(gl.ARRAY_BUFFER, new Float32Array(lines));
      gl.enableVertexAttribArray(0);
      gl.vertexAttribPointer(0, 3, gl.FLOAT, false, 12, 0);
      gl.bindVertexArray(null);
      this.bench = { buffer, vao, vertices: lines.length / 3 };
    }
    this.requestRender();
  }

  setPaItems(items: PaDrawItem[]) {
    this.layers.pa.setItems(items);
    this.requestRender();
  }

  setTrajectories(items: TrajectoryItem[]) {
    this.layers.trajectories.setItems(items);
    this.requestRender();
  }

  setHighlight(h: Highlight | null) {
    this.layers.trajectories.setHighlight(h);
    this.requestRender();
  }

  protected release() {
    const gl = this.gl;
    this.canvas.removeEventListener("webglcontextlost", this.onLost);
    for (const obj of this.objects.values()) {
      gl.deleteBuffer(obj.buffer);
      gl.deleteVertexArray(obj.vao);
    }
    this.objects.clear();
    this.layers.pa.destroy();
    this.layers.trajectories.destroy();
    if (this.bench) {
      gl.deleteBuffer(this.bench.buffer);
      gl.deleteVertexArray(this.bench.vao);
    }
    this.releaseTargets();
    gl.deleteBuffer(this.frameUbo);
    gl.deleteVertexArray(this.emptyVao);
    this.c.destroy();
    for (const p of Object.values(this.programs)) gl.deleteProgram(p.program);
    // The context stays alive: a remount (React StrictMode) takes the same canvas context again.
  }

  private releaseTargets() {
    const t = this.targets;
    if (!t) return;
    this.gl.deleteFramebuffer(t.fbo);
    this.gl.deleteRenderbuffer(t.color);
    this.gl.deleteRenderbuffer(t.depth);
    this.targets = null;
  }

  /** 4x MSAA colour + float depth, like the WebGPU targets. */
  private ensureTargets(width: number, height: number) {
    if (this.targets && this.targets.width === width && this.targets.height === height) return this.targets;
    this.releaseTargets();
    const gl = this.gl;
    const color = gl.createRenderbuffer()!;
    gl.bindRenderbuffer(gl.RENDERBUFFER, color);
    gl.renderbufferStorageMultisample(gl.RENDERBUFFER, this.samples, gl.RGBA8, width, height);
    const depth = gl.createRenderbuffer()!;
    gl.bindRenderbuffer(gl.RENDERBUFFER, depth);
    gl.renderbufferStorageMultisample(gl.RENDERBUFFER, this.samples, gl.DEPTH_COMPONENT32F, width, height);
    const fbo = gl.createFramebuffer()!;
    gl.bindFramebuffer(gl.FRAMEBUFFER, fbo);
    gl.framebufferRenderbuffer(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.RENDERBUFFER, color);
    gl.framebufferRenderbuffer(gl.FRAMEBUFFER, gl.DEPTH_ATTACHMENT, gl.RENDERBUFFER, depth);
    this.targets = { fbo, color, depth, width, height };
    return this.targets;
  }

  protected draw() {
    const gl = this.gl;
    if (this.destroyed || gl.isContextLost()) return;
    const { width, height } = this.canvas;
    if (width === 0 || height === 0) return;
    const targets = this.ensureTargets(width, height);
    this.writeFrameData();
    gl.bindBuffer(gl.UNIFORM_BUFFER, this.frameUbo);
    gl.bufferSubData(gl.UNIFORM_BUFFER, 0, this.frameData);
    gl.bindBufferBase(gl.UNIFORM_BUFFER, FRAME_BINDING, this.frameUbo);

    gl.bindFramebuffer(gl.FRAMEBUFFER, targets.fbo);
    gl.viewport(0, 0, width, height);
    gl.depthMask(true);
    gl.clearColor(0, 0, 0, 0);
    gl.clearDepth(0);
    gl.clear(gl.COLOR_BUFFER_BIT | gl.DEPTH_BUFFER_BIT);

    this.c.use(this.programs.mesh, { func: gl.GREATER, write: true }, false);
    for (const item of this.items) {
      const obj = this.objects.get(item.id);
      if (!item.visible || !obj || item.mesh.triangleCount === 0) continue;
      objectUniforms(item, this.objectData);
      this.c.block(this.objectData);
      gl.bindVertexArray(obj.vao);
      gl.drawArrays(gl.TRIANGLES, 0, item.mesh.triangleCount * 3);
    }
    this.layers.pa.draw(this.camera.eye());
    if (this.bench) {
      this.c.use(this.programs.bench, { func: gl.GEQUAL, write: false }, true);
      this.c.block(this.benchData);
      gl.bindVertexArray(this.bench.vao);
      gl.drawArrays(gl.LINES, 0, this.bench.vertices);
    }
    this.layers.trajectories.draw(width, height);

    if (this.gridVisible) {
      gl.bindVertexArray(this.emptyVao);
      this.c.use(this.programs.grid, { func: gl.GREATER, write: false }, true);
      gl.drawArrays(gl.TRIANGLES, 0, 6);
      this.c.use(this.programs.axis, { func: gl.GREATER, write: false }, true);
      gl.drawArrays(gl.LINES, 0, 6);
    }
    gl.bindVertexArray(null);

    // Resolve the multisampled frame into the canvas.
    gl.bindFramebuffer(gl.READ_FRAMEBUFFER, targets.fbo);
    gl.bindFramebuffer(gl.DRAW_FRAMEBUFFER, null);
    gl.blitFramebuffer(0, 0, width, height, 0, 0, width, height, gl.COLOR_BUFFER_BIT, gl.NEAREST);
    gl.bindFramebuffer(gl.FRAMEBUFFER, null);
  }
}
