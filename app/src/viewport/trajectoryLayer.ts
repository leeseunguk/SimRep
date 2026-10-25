import type { Flight } from "../scene/flight";
import { unionBounds, type Bounds } from "../scene/mesh";
import type { ViewportTheme } from "./color";
import { TRAJECTORY_UNIFORM_FLOATS, trajectoryShader } from "./shaders";

export interface TrajectoryItem {
  id: string;
  flight: Flight;
  visible: boolean;
}

interface TrajectoryGpu {
  flight: Flight;
  points: GPUBuffer | null;
  indices: GPUBuffer | null;
  indexCount: number;
  splats: GPUBuffer | null;
  splatCount: number;
  bounds: Bounds | null;
}

/**
 * Line opacity falls with the ion count (alpha = LINE_DENSITY / sqrt(ions), clamped), so a few
 * ions read as solid lines and thousands blend into a density map where focal points stand out.
 */
const LINE_DENSITY = 2.5;
const LINE_ALPHA_MIN = 0.06;
const LINE_ALPHA_MAX = 0.8;
const SPLAT_ALPHA = 0.95;
const SPLAT_RADIUS_CSS_PX = 3;
const RIM_DARKNESS = 0.4;
const RESTART = 0xffffffff;
/** Other trajectories fade to this share of their alpha while an ion is highlighted. */
const DIM_WITH_HIGHLIGHT = 0.45;

/** The selected ion: its path (x, y, z per point) and end point, in workbench mm. */
export interface Highlight {
  points: Float32Array<ArrayBuffer> | null;
  end: [number, number, number];
}

export interface IonHit {
  id: string;
  index: number;
}

/** Ion trajectories (one line strip per ion, primitive restart between ions) and splat markers. */
export class TrajectoryLayer {
  private readonly linePipeline: GPURenderPipeline;
  private readonly splatPipeline: GPURenderPipeline;
  private readonly thickPipeline: GPURenderPipeline;
  private readonly ringPipeline: GPURenderPipeline;
  private highlight: { points: GPUBuffer | null; segments: number; end: GPUBuffer } | null = null;
  private readonly uniform: GPUBuffer;
  private readonly bindGroup: GPUBindGroup;
  private readonly data = new Float32Array(TRAJECTORY_UNIFORM_FLOATS);
  private readonly gpu = new Map<string, TrajectoryGpu>();
  private items: TrajectoryItem[] = [];
  private theme: ViewportTheme | null = null;

  constructor(
    private readonly device: GPUDevice,
    format: GPUTextureFormat,
    frameLayout: GPUBindGroupLayout,
    depthFormat: GPUTextureFormat,
    sampleCount: number,
  ) {
    const layout = device.createBindGroupLayout({
      entries: [{ binding: 0, visibility: GPUShaderStage.VERTEX | GPUShaderStage.FRAGMENT, buffer: { type: "uniform" } }],
    });
    this.uniform = device.createBuffer({ size: TRAJECTORY_UNIFORM_FLOATS * 4, usage: GPUBufferUsage.UNIFORM | GPUBufferUsage.COPY_DST });
    this.bindGroup = device.createBindGroup({ layout, entries: [{ binding: 0, resource: { buffer: this.uniform } }] });
    const module = device.createShaderModule({ label: "trajectories", code: trajectoryShader });
    const pipelineLayout = device.createPipelineLayout({ bindGroupLayouts: [frameLayout, layout] });
    const blend: GPUBlendState = {
      color: { srcFactor: "one", dstFactor: "one-minus-src-alpha" },
      alpha: { srcFactor: "one", dstFactor: "one-minus-src-alpha" },
    };
    // Depth-tested against electrodes and the section, but not written: lines don't hide each other.
    const depthStencil: GPUDepthStencilState = { format: depthFormat, depthWriteEnabled: false, depthCompare: "greater-equal" };
    this.linePipeline = device.createRenderPipeline({
      label: "trajectory-lines",
      layout: pipelineLayout,
      vertex: { module, entryPoint: "vsLine", buffers: [{ arrayStride: 12, attributes: [{ shaderLocation: 0, offset: 0, format: "float32x3" }] }] },
      fragment: { module, entryPoint: "fsLine", targets: [{ format, blend }] },
      primitive: { topology: "line-strip", stripIndexFormat: "uint32" },
      depthStencil,
      multisample: { count: sampleCount },
    });
    this.splatPipeline = device.createRenderPipeline({
      label: "trajectory-splats",
      layout: pipelineLayout,
      vertex: {
        module,
        entryPoint: "vsSplat",
        buffers: [{ arrayStride: 16, stepMode: "instance", attributes: [{ shaderLocation: 0, offset: 0, format: "float32x4" }] }],
      },
      fragment: { module, entryPoint: "fsSplat", targets: [{ format, blend }] },
      primitive: { topology: "triangle-list" },
      depthStencil,
      multisample: { count: sampleCount },
    });
    // The highlight is drawn over electrodes and other paths: it must stay findable.
    const onTop: GPUDepthStencilState = { format: depthFormat, depthWriteEnabled: false, depthCompare: "always" };
    this.thickPipeline = device.createRenderPipeline({
      label: "trajectory-highlight",
      layout: pipelineLayout,
      vertex: {
        module,
        entryPoint: "vsThick",
        // Segment i = points i and i + 1: the same buffer bound twice, the second 12 bytes in.
        buffers: [
          { arrayStride: 12, stepMode: "instance", attributes: [{ shaderLocation: 0, offset: 0, format: "float32x3" }] },
          { arrayStride: 12, stepMode: "instance", attributes: [{ shaderLocation: 1, offset: 0, format: "float32x3" }] },
        ],
      },
      fragment: { module, entryPoint: "fsThick", targets: [{ format, blend }] },
      primitive: { topology: "triangle-list" },
      depthStencil: onTop,
      multisample: { count: sampleCount },
    });
    this.ringPipeline = device.createRenderPipeline({
      label: "trajectory-highlight-end",
      layout: pipelineLayout,
      vertex: {
        module,
        entryPoint: "vsRing",
        buffers: [{ arrayStride: 16, stepMode: "instance", attributes: [{ shaderLocation: 0, offset: 0, format: "float32x4" }] }],
      },
      fragment: { module, entryPoint: "fsRing", targets: [{ format, blend }] },
      primitive: { topology: "triangle-list" },
      depthStencil: onTop,
      multisample: { count: sampleCount },
    });
  }

  /** Highlights one ion (null clears). Its path may come from the flight or from a re-traced ion. */
  setHighlight(h: Highlight | null) {
    if (this.highlight) {
      this.highlight.points?.destroy();
      this.highlight.end.destroy();
      this.highlight = null;
    }
    if (!h) return;
    const segments = h.points ? Math.max(0, h.points.length / 3 - 1) : 0;
    this.highlight = {
      points: segments > 0 ? this.buffer(h.points!, GPUBufferUsage.VERTEX, "trajectory-highlight") : null,
      segments,
      end: this.buffer(new Float32Array([...h.end, 0]), GPUBufferUsage.VERTEX, "trajectory-highlight-end"),
    };
  }

  /**
   * Ion under a screen point (device px): the nearest end point of any ion, or segment of a drawn
   * trajectory, within `tolerance` px. `viewProj` is the column-major view-projection.
   */
  pick(viewProj: Float32Array, width: number, height: number, px: number, py: number, tolerance: number): IonHit | null {
    const m = viewProj;
    let best: IonHit | null = null;
    let bestD = tolerance * tolerance;
    const project = (x: number, y: number, z: number, out: Float64Array, o: number) => {
      const w = m[3]! * x + m[7]! * y + m[11]! * z + m[15]!;
      if (w <= 1e-9) return false;
      out[o] = ((m[0]! * x + m[4]! * y + m[8]! * z + m[12]!) / w * 0.5 + 0.5) * width;
      out[o + 1] = (0.5 - (m[1]! * x + m[5]! * y + m[9]! * z + m[13]!) / w * 0.5) * height;
      return true;
    };
    const a = new Float64Array(2);
    const b = new Float64Array(2);
    for (const item of this.items) {
      if (!item.visible) continue;
      const { ends, offsets, points, header } = item.flight;
      for (let i = 0; i < header.ions; i++) {
        if (!project(ends[i * 4]!, ends[i * 4 + 1]!, ends[i * 4 + 2]!, a, 0)) continue;
        // End points win ties against the paths running through them.
        const d = (a[0]! - px) ** 2 + (a[1]! - py) ** 2 - 1;
        if (d < bestD) {
          bestD = d;
          best = { id: item.id, index: i };
        }
      }
      for (let i = 0; i < header.ions; i++) {
        const s = offsets[i]!;
        const e = offsets[i + 1]!;
        if (e - s < 2) continue;
        let ok = project(points[s * 3]!, points[s * 3 + 1]!, points[s * 3 + 2]!, a, 0);
        for (let k = s + 1; k < e; k++) {
          const okB = project(points[k * 3]!, points[k * 3 + 1]!, points[k * 3 + 2]!, b, 0);
          if (ok && okB) {
            const d = segmentDistance2(px, py, a[0]!, a[1]!, b[0]!, b[1]!);
            if (d < bestD) {
              bestD = d;
              best = { id: item.id, index: i };
            }
          }
          a[0] = b[0]!;
          a[1] = b[1]!;
          ok = okB;
        }
      }
    }
    return best;
  }

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

  /** Box around the visible trajectories (included in "fit all"). */
  bounds(ids?: readonly string[]): Bounds | null {
    let b: Bounds | null = null;
    for (const item of this.items) {
      const g = this.gpu.get(item.id);
      if (item.visible && g?.bounds && (!ids || ids.includes(item.id))) b = unionBounds(b, g.bounds);
    }
    return b;
  }

  draw(pass: GPURenderPassEncoder, width: number, height: number) {
    const theme = this.theme;
    if (!theme || !this.items.some((i) => i.visible)) return;
    // Only drawn ions have lines (the rest are end points), so they set the line density.
    const ions = this.items.reduce((n, i) => n + (i.visible ? i.flight.header.drawn : 0), 0);
    const lineAlpha =
      Math.min(LINE_ALPHA_MAX, Math.max(LINE_ALPHA_MIN, LINE_DENSITY / Math.sqrt(Math.max(ions, 1)))) * (this.highlight ? DIM_WITH_HIGHLIGHT : 1);
    const d = this.data;
    d.set([...theme.trajectory, lineAlpha], 0);
    d.set([...theme.splatHit, SPLAT_ALPHA], 4);
    d.set([...theme.splat, SPLAT_ALPHA], 8);
    d.set([width, height, SPLAT_RADIUS_CSS_PX * devicePixelRatio, RIM_DARKNESS], 12);
    d.set([...theme.highlight, 1], 16);
    d.set([theme.highlightWidth * devicePixelRatio, theme.highlightRing * devicePixelRatio, 0, 0], 20);
    this.device.queue.writeBuffer(this.uniform, 0, d);
    pass.setBindGroup(1, this.bindGroup);
    for (const item of this.items) {
      const g = this.gpu.get(item.id);
      if (!item.visible || !g?.points || !g.indices) continue;
      pass.setPipeline(this.linePipeline);
      pass.setVertexBuffer(0, g.points);
      pass.setIndexBuffer(g.indices, "uint32");
      pass.drawIndexed(g.indexCount);
    }
    for (const item of this.items) {
      const g = this.gpu.get(item.id);
      if (!item.visible || !g?.splats) continue;
      pass.setPipeline(this.splatPipeline);
      pass.setVertexBuffer(0, g.splats);
      pass.draw(6, g.splatCount);
    }
    const h = this.highlight;
    if (h) {
      if (h.points && h.segments > 0) {
        pass.setPipeline(this.thickPipeline);
        pass.setVertexBuffer(0, h.points);
        pass.setVertexBuffer(1, h.points, 12);
        pass.draw(6, h.segments);
      }
      pass.setPipeline(this.ringPipeline);
      pass.setVertexBuffer(0, h.end);
      pass.draw(6, 1);
    }
  }

  destroy() {
    this.setHighlight(null);
    for (const g of this.gpu.values()) this.release(g);
    this.gpu.clear();
    this.uniform.destroy();
  }

  private buffer(data: ArrayBufferView<ArrayBuffer>, usage: number, label: string): GPUBuffer {
    const b = this.device.createBuffer({ label, size: Math.max(4, Math.ceil(data.byteLength / 4) * 4), usage: usage | GPUBufferUsage.COPY_DST });
    this.device.queue.writeBuffer(b, 0, data);
    return b;
  }

  private upload(flight: Flight): TrajectoryGpu {
    const { offsets, points, ends, surface, header } = flight;
    const ions = header.ions;
    // Strip indices with a restart between ions (single-point trajectories draw nothing).
    const indices = new Uint32Array(header.points + ions);
    let k = 0;
    for (let i = 0; i < ions; i++) {
      const a = offsets[i]!;
      const b = offsets[i + 1]!;
      if (b - a < 2) continue;
      for (let p = a; p < b; p++) indices[k++] = p;
      indices[k++] = RESTART;
    }
    let bounds: Bounds | null = null;
    if (points.length >= 3) {
      const min: [number, number, number] = [Infinity, Infinity, Infinity];
      const max: [number, number, number] = [-Infinity, -Infinity, -Infinity];
      for (let i = 0; i < points.length; i += 3)
        for (let a = 0; a < 3; a++) {
          const v = points[i + a]!;
          if (v < min[a]!) min[a] = v;
          if (v > max[a]!) max[a] = v;
        }
      bounds = { min, max };
    }
    const splats = new Float32Array(ions * 4);
    for (let i = 0; i < ions; i++) {
      splats.set(ends.subarray(i * 4, i * 4 + 3), i * 4);
      splats[i * 4 + 3] = header.surfaces[surface[i]!]?.surface.kind === "electrode" ? 1 : 0;
    }
    return {
      flight,
      points: header.points > 0 ? this.buffer(points, GPUBufferUsage.VERTEX, "trajectory-points") : null,
      indices: k > 0 ? this.buffer(indices.subarray(0, k), GPUBufferUsage.INDEX, "trajectory-indices") : null,
      indexCount: k,
      splats: ions > 0 ? this.buffer(splats, GPUBufferUsage.VERTEX, "trajectory-splats") : null,
      splatCount: ions,
      bounds,
    };
  }

  private release(g: TrajectoryGpu) {
    g.points?.destroy();
    g.indices?.destroy();
    g.splats?.destroy();
  }
}

function segmentDistance2(px: number, py: number, ax: number, ay: number, bx: number, by: number): number {
  const dx = bx - ax;
  const dy = by - ay;
  const len = dx * dx + dy * dy;
  const t = len > 0 ? Math.max(0, Math.min(1, ((px - ax) * dx + (py - ay) * dy) / len)) : 0;
  return (ax + t * dx - px) ** 2 + (ay + t * dy - py) ** 2;
}
