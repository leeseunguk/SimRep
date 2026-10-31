import { unionBounds, type Bounds, type Vec3 } from "../scene/mesh";
import {
  contourStep,
  displayBounds,
  displayMirrors,
  extractSlice,
  sampleSlice,
  slicePlane,
  type PaGrid,
  type PaInfo,
  type SliceImage,
  type SlicePlane,
  type SliceState,
} from "../scene/pa";
import { QUAD_FLOATS, type PaSurface } from "../scene/paSurface";
import { boundsToWorld, dirToLocal, dirToWorld, IDENTITY, modelMatrix, toLocal, toWorld, type Placement } from "../scene/placement";
import { srgbToLinear, type ViewportTheme } from "./color";
import { HIDDEN_LABELS, PA_HIDDEN_OFFSET, PA_MODEL_OFFSET, PA_UNIFORM_FLOATS, paSurfaceShader, SLICE_UNIFORM_FLOATS, sliceShader } from "./shaders";

export interface PaDrawItem {
  id: string;
  info: PaInfo;
  grid: PaGrid;
  surface: PaSurface;
  visible: boolean;
  selected: boolean;
  /** Electrode label highlighted on the surface (null = none). */
  selectedElectrode: number | null;
  hiddenElectrodes: readonly number[];
  /** Electrode label per grid point (null until computed). */
  labels: Uint16Array | null;
  showMirrors: boolean;
  slice: SliceState;
  /** Where the array sits on the workbench (null = at the origin). */
  placement: Placement | null;
}

export interface ProbeHit {
  id: string;
  /** World (= array) position on the section plane, mm. */
  point: Vec3;
  value: number;
  electrode: boolean;
  /** Ray parameter, to prefer the nearest section. */
  t: number;
}

interface PaGpu {
  surface: PaSurface;
  quadBuffer: GPUBuffer | null;
  /** One uniform buffer + bind group per mirror combination (up to 8). */
  variants: { buffer: GPUBuffer; bindGroup: GPUBindGroup }[];
  sliceUniform: GPUBuffer;
  sliceTexture: GPUTexture | null;
  sliceBindGroup: GPUBindGroup | null;
  sliceKey: string;
  grid: PaGrid | null;
  labels: Uint16Array | null;
  plane: SlicePlane | null;
  image: SliceImage | null;
}

const MAX_VARIANTS = 8;

/** CPU state of one array both backends keep: the shown section plane and its image. */
export interface PaSliceState {
  plane: SlicePlane | null;
  image: SliceImage | null;
}

export function paBounds(items: readonly PaDrawItem[], ids?: readonly string[]): Bounds | null {
  let b: Bounds | null = null;
  for (const item of items) {
    if (!item.visible || (ids && !ids.includes(item.id))) continue;
    b = unionBounds(b, boundsToWorld(item.placement, displayBounds(item.info, item.showMirrors)));
  }
  return b;
}

export function paPickables(items: readonly PaDrawItem[]): { id: string; bounds: Bounds; visible: boolean }[] {
  return items.map((i) => ({ id: i.id, bounds: boundsToWorld(i.placement, displayBounds(i.info, i.showMirrors)), visible: i.visible }));
}

/** Section value under a ray (nearest visible section), for the hover readout. */
export function paProbe(
  items: readonly PaDrawItem[],
  state: (id: string) => PaSliceState | undefined,
  worldOrigin: Vec3,
  worldDir: Vec3,
): ProbeHit | null {
  let best: ProbeHit | null = null;
  for (const item of items) {
    const g = state(item.id);
    if (!item.visible || !item.slice.visible || !g?.plane || !g.image) continue;
    // Same ray parameter in array coordinates (the placement is affine).
    const place = item.placement ?? IDENTITY;
    const origin = toLocal(place, worldOrigin);
    const dir = dirToLocal(place, worldDir);
    const p = g.plane;
    const d = dir[p.axis];
    if (Math.abs(d) < 1e-12) continue;
    const t = (p.position - origin[p.axis]) / d;
    if (t < 0 || (best && t >= best.t)) continue;
    const point: Vec3 = [origin[0] + dir[0] * t, origin[1] + dir[1] * t, origin[2] + dir[2] * t];
    point[p.axis] = p.position;
    const u = point[p.u];
    const v = point[p.v];
    if (u < p.uRange[0] || u > p.uRange[1] || v < p.vRange[0] || v > p.vRange[1]) continue;
    const s = sampleSlice(g.image, p, u, v);
    best = { id: item.id, point: toWorld(place, point), value: s.value, electrode: s.electrode, t };
  }
  return best;
}

/** The slice image to upload when the shown grid plane changed (null = unchanged). */
export function paSliceUpdate(
  g: PaSliceState & { sliceKey: string; grid: PaGrid | null; labels: Uint16Array | null },
  item: PaDrawItem,
  maxTextureSize: number,
): SliceImage | null {
  const plane = slicePlane(item.info, item.slice, item.showMirrors);
  g.plane = plane;
  const marks = item.labels ? { labels: item.labels, selected: item.selectedElectrode, hidden: item.hiddenElectrodes } : null;
  const key = `${plane.axis}:${plane.index}:${plane.normalSign}:${marks ? `${marks.selected}:${marks.hidden.join(",")}` : ""}`;
  if (key === g.sliceKey && g.grid === item.grid && g.labels === item.labels) return null;
  const image = extractSlice(item.info, item.grid, plane, maxTextureSize, marks);
  g.image = image;
  g.grid = item.grid;
  g.labels = item.labels;
  g.sliceKey = key;
  return image;
}

/**
 * Uniforms of the electrode surface: one block per mirror combination (up to 8), in the Pa struct
 * layout of the WGSL and GLSL shaders. `emit` is called once per combination, data filled.
 */
export function paSurfaceUniforms(
  item: PaDrawItem,
  plane: SlicePlane | null,
  eye: Vec3,
  theme: ViewportTheme,
  data: Float32Array<ArrayBuffer>,
  emit: (data: Float32Array<ArrayBuffer>, variant: number) => void,
) {
  const lin = (c: [number, number, number]) => srgbToLinear(c);
  const { info } = item;
  const fullScale = paFullScale(info);
  const place = item.placement ?? IDENTITY;
  const localEye = toLocal(place, eye);
  const cut = item.slice.visible && item.slice.cut && plane !== null;
  const side = plane && localEye[plane.axis] < plane.position ? -1 : 1;
  const model = modelMatrix(place);
  const m = displayMirrors(info, item.showMirrors);
  const signs = (flag: boolean) => (flag ? [1, -1] : [1]);
  const hiddenBits = new Uint32Array(data.buffer, data.byteOffset + PA_HIDDEN_OFFSET * 4, HIDDEN_LABELS / 32);
  hiddenBits.fill(0);
  for (const label of item.hiddenElectrodes) if (label < HIDDEN_LABELS) hiddenBits[label >> 5] = hiddenBits[label >> 5]! | (1 << (label & 31));
  let v = 0;
  for (const sx of signs(m.x))
    for (const sy of signs(m.y))
      for (const sz of signs(m.z)) {
        const anti = (sx < 0 && info.antiMirror.x ? -1 : 1) * (sy < 0 && info.antiMirror.y ? -1 : 1) * (sz < 0 && info.antiMirror.z ? -1 : 1);
        data.set([sx, sy, sz, anti], 0);
        data.set([...lin(theme.electrodeNeutral), 1], 4);
        data.set([...lin(theme.electrodePositive), 1], 8);
        data.set([...lin(theme.electrodeNegative), 1], 12);
        data.set([fullScale, item.selected ? 1 : 0, cut ? 1 : 0, side], 16);
        data.set([plane?.axis ?? 0, plane?.position ?? 0, 0, 0], 20);
        data.set([item.selectedElectrode ?? -1, 0, 0, 0], 24);
        data.set(model, PA_MODEL_OFFSET);
        emit(data, v++);
      }
}

/** Uniforms of the potential section (Slice struct of the WGSL and GLSL shaders). */
export function paSliceUniforms(item: PaDrawItem, plane: SlicePlane, image: SliceImage, theme: ViewportTheme, d: Float32Array) {
  const lin = (c: [number, number, number]) => srgbToLinear(c);
  const { info } = item;
  const place = item.placement ?? IDENTITY;
  const origin: Vec3 = [0, 0, 0];
  origin[plane.axis] = plane.position;
  origin[plane.u] = plane.uRange[0];
  origin[plane.v] = plane.vRange[0];
  const uSpan: Vec3 = [0, 0, 0];
  uSpan[plane.u] = plane.uRange[1] - plane.uRange[0];
  const vSpan: Vec3 = [0, 0, 0];
  vSpan[plane.v] = plane.vRange[1] - plane.vRange[0];
  const step = item.slice.contours ? contourStep(info.potentialMin, info.potentialMax, item.slice.density) : 0;
  d.set([...toWorld(place, origin), 1], 0);
  d.set([...dirToWorld(place, uSpan), plane.uRange[0]], 4);
  d.set([...dirToWorld(place, vSpan), plane.vRange[0]], 8);
  d.set([image.du, image.dv, image.width, image.height], 12);
  d.set([plane.mirrorU ? 1 : 0, plane.mirrorV ? 1 : 0, plane.antiU ? 1 : 0, plane.antiV ? 1 : 0], 16);
  d.set([info.potentialMin, info.potentialMax, step, paFullScale(info)], 20);
  d.set([item.selected ? 1 : 0, place.scale, 0, 0], 24);
  d.set([...lin(theme.electrodeNeutral), 1], 28);
  d.set([...lin(theme.electrodePositive), 1], 32);
  d.set([...lin(theme.electrodeNegative), 1], 36);
  d.set([...lin(theme.contourDark), 1], 40);
  d.set([...lin(theme.contourLight), 1], 44);
  theme.field.forEach((c, i) => d.set([...lin(c), 1], 48 + i * 4));
}

function paFullScale(info: PaInfo): number {
  return Math.max(Math.abs(info.electrodeMin ?? 0), Math.abs(info.electrodeMax ?? 0), 1e-9);
}

/** Electrode surfaces and potential sections of SIMION arrays (owned by Renderer). */
export class PaLayer {
  private readonly surfacePipeline: GPURenderPipeline;
  private readonly slicePipeline: GPURenderPipeline;
  private readonly surfaceLayout: GPUBindGroupLayout;
  private readonly sliceLayout: GPUBindGroupLayout;
  private readonly gpu = new Map<string, PaGpu>();
  private items: PaDrawItem[] = [];
  private theme: ViewportTheme | null = null;
  private readonly surfaceData = new Float32Array(PA_UNIFORM_FLOATS);
  private readonly sliceData = new Float32Array(SLICE_UNIFORM_FLOATS);

  constructor(
    private readonly device: GPUDevice,
    format: GPUTextureFormat,
    frameLayout: GPUBindGroupLayout,
    depthFormat: GPUTextureFormat,
    sampleCount: number,
  ) {
    this.surfaceLayout = device.createBindGroupLayout({
      entries: [{ binding: 0, visibility: GPUShaderStage.VERTEX | GPUShaderStage.FRAGMENT, buffer: { type: "uniform" } }],
    });
    this.sliceLayout = device.createBindGroupLayout({
      entries: [
        { binding: 0, visibility: GPUShaderStage.VERTEX | GPUShaderStage.FRAGMENT, buffer: { type: "uniform" } },
        { binding: 1, visibility: GPUShaderStage.FRAGMENT, texture: { sampleType: "unfilterable-float" } },
      ],
    });
    const multisample = { count: sampleCount };

    const surfaceModule = device.createShaderModule({ label: "pa-surface", code: paSurfaceShader });
    const vec3 = (location: number): GPUVertexAttribute => ({ shaderLocation: location, offset: location * 12, format: "float32x3" });
    this.surfacePipeline = device.createRenderPipeline({
      label: "pa-surface",
      layout: device.createPipelineLayout({ bindGroupLayouts: [frameLayout, this.surfaceLayout] }),
      vertex: {
        module: surfaceModule,
        entryPoint: "vs",
        buffers: [
          {
            arrayStride: QUAD_FLOATS * 4,
            stepMode: "instance",
            attributes: [
              vec3(0),
              vec3(1),
              vec3(2),
              vec3(3),
              { shaderLocation: 4, offset: 48, format: "float32" },
              { shaderLocation: 5, offset: 52, format: "float32" },
            ],
          },
        ],
      },
      fragment: { module: surfaceModule, entryPoint: "fs", targets: [{ format }] },
      primitive: { topology: "triangle-list", cullMode: "none" },
      depthStencil: { format: depthFormat, depthWriteEnabled: true, depthCompare: "greater" },
      multisample,
    });

    const sliceModule = device.createShaderModule({ label: "pa-slice", code: sliceShader });
    this.slicePipeline = device.createRenderPipeline({
      label: "pa-slice",
      layout: device.createPipelineLayout({ bindGroupLayouts: [frameLayout, this.sliceLayout] }),
      vertex: { module: sliceModule, entryPoint: "vs" },
      fragment: { module: sliceModule, entryPoint: "fs", targets: [{ format }] },
      primitive: { topology: "triangle-list", cullMode: "none" },
      // Reversed Z: a positive bias pulls the section in front of coplanar electrode faces.
      depthStencil: { format: depthFormat, depthWriteEnabled: true, depthCompare: "greater", depthBias: 4, depthBiasSlopeScale: 1 },
      multisample,
    });
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
        const variants = Array.from({ length: MAX_VARIANTS }, () => {
          const buffer = this.device.createBuffer({ size: PA_UNIFORM_FLOATS * 4, usage: GPUBufferUsage.UNIFORM | GPUBufferUsage.COPY_DST });
          const bindGroup = this.device.createBindGroup({ layout: this.surfaceLayout, entries: [{ binding: 0, resource: { buffer } }] });
          return { buffer, bindGroup };
        });
        const sliceUniform = this.device.createBuffer({
          size: SLICE_UNIFORM_FLOATS * 4,
          usage: GPUBufferUsage.UNIFORM | GPUBufferUsage.COPY_DST,
        });
        g = { surface: item.surface, quadBuffer: null, variants, sliceUniform, sliceTexture: null, sliceBindGroup: null, sliceKey: "", grid: null, labels: null, plane: null, image: null };
        this.uploadSurface(g, item.surface);
        this.gpu.set(item.id, g);
      } else if (g.surface !== item.surface) {
        g.quadBuffer?.destroy();
        this.uploadSurface(g, item.surface);
      }
      this.updateSlice(g, item);
    }
    this.items = items;
  }

  bounds(ids?: readonly string[]): Bounds | null {
    return paBounds(this.items, ids);
  }

  pickables(): { id: string; bounds: Bounds; visible: boolean }[] {
    return paPickables(this.items);
  }

  /** Section value under a ray (nearest visible section), for the hover readout. */
  probe(worldOrigin: Vec3, worldDir: Vec3): ProbeHit | null {
    return paProbe(this.items, (id) => this.gpu.get(id), worldOrigin, worldDir);
  }

  draw(pass: GPURenderPassEncoder, eye: Vec3) {
    const theme = this.theme;
    if (!theme) return;
    for (const item of this.items) {
      const g = this.gpu.get(item.id);
      if (!item.visible || !g) continue;
      const plane = g.plane;
      if (g.quadBuffer && item.surface.count > 0) {
        pass.setPipeline(this.surfacePipeline);
        pass.setVertexBuffer(0, g.quadBuffer);
        paSurfaceUniforms(item, plane, eye, theme, this.surfaceData, (d, v) => {
          const variant = g.variants[v]!;
          this.device.queue.writeBuffer(variant.buffer, 0, d);
          pass.setBindGroup(1, variant.bindGroup);
          pass.draw(6, item.surface.count);
        });
      }
      if (item.slice.visible && plane && g.image && g.sliceBindGroup) {
        paSliceUniforms(item, plane, g.image, theme, this.sliceData);
        this.device.queue.writeBuffer(g.sliceUniform, 0, this.sliceData);
        pass.setPipeline(this.slicePipeline);
        pass.setBindGroup(1, g.sliceBindGroup);
        pass.draw(6);
      }
    }
  }

  destroy() {
    for (const g of this.gpu.values()) this.release(g);
    this.gpu.clear();
  }

  private uploadSurface(g: PaGpu, surface: PaSurface) {
    g.surface = surface;
    g.quadBuffer = null;
    if (surface.count === 0) return;
    g.quadBuffer = this.device.createBuffer({
      label: "pa-quads",
      size: surface.quads.byteLength,
      usage: GPUBufferUsage.VERTEX | GPUBufferUsage.COPY_DST,
    });
    this.device.queue.writeBuffer(g.quadBuffer, 0, surface.quads);
  }

  /** Re-extracts the slice image only when the shown grid plane changes. */
  private updateSlice(g: PaGpu, item: PaDrawItem) {
    const image = paSliceUpdate(g, item, this.device.limits.maxTextureDimension2D);
    if (!image) return;
    if (!g.sliceTexture || g.sliceTexture.width !== image.width || g.sliceTexture.height !== image.height) {
      g.sliceTexture?.destroy();
      g.sliceTexture = this.device.createTexture({
        size: [image.width, image.height],
        format: "rg32float",
        usage: GPUTextureUsage.TEXTURE_BINDING | GPUTextureUsage.COPY_DST,
      });
      g.sliceBindGroup = this.device.createBindGroup({
        layout: this.sliceLayout,
        entries: [
          { binding: 0, resource: { buffer: g.sliceUniform } },
          { binding: 1, resource: g.sliceTexture.createView() },
        ],
      });
    }
    this.device.queue.writeTexture({ texture: g.sliceTexture }, image.data, { bytesPerRow: image.width * 8, rowsPerImage: image.height }, [
      image.width,
      image.height,
    ]);
  }

  private release(g: PaGpu) {
    g.quadBuffer?.destroy();
    for (const v of g.variants) v.buffer.destroy();
    g.sliceUniform.destroy();
    g.sliceTexture?.destroy();
  }
}
