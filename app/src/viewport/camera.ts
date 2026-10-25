import { boundsCenter, boundsSize, type Bounds, type Vec3 } from "../scene/mesh";
import type { ViewPreset } from "../state/store";

export const FOV_Y = (40 * Math.PI) / 180;
const MIN_DISTANCE = 1e-3;
const MAX_DISTANCE = 1e7;
const ORBIT_RADIANS_PER_PIXEL = 0.006;
const HALF_PI = Math.PI / 2;

export interface CameraState {
  target: Vec3;
  distance: number;
  yaw: number;
  pitch: number;
}

export interface CameraBasis {
  right: Vec3;
  up: Vec3;
  /** Unit vector from the target towards the eye. */
  back: Vec3;
}

const add = (a: Vec3, b: Vec3): Vec3 => [a[0] + b[0], a[1] + b[1], a[2] + b[2]];
const sub = (a: Vec3, b: Vec3): Vec3 => [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
const scale = (a: Vec3, s: number): Vec3 => [a[0] * s, a[1] * s, a[2] * s];
const dot = (a: Vec3, b: Vec3) => a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
const cross = (a: Vec3, b: Vec3): Vec3 => [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
const normalize = (a: Vec3): Vec3 => scale(a, 1 / Math.hypot(a[0], a[1], a[2]));
const clamp = (v: number, lo: number, hi: number) => Math.min(hi, Math.max(lo, v));
const easeOutCubic = (t: number) => 1 - (1 - t) ** 3;

/** Y-up world (SIMION: X is the beam / symmetry axis, Y up). Yaw 0 + pitch 0 looks down -Z (front view). */
export const PRESET_ANGLES: Record<ViewPreset, { yaw: number; pitch: number }> = {
  front: { yaw: 0, pitch: 0 },
  back: { yaw: Math.PI, pitch: 0 },
  right: { yaw: HALF_PI, pitch: 0 },
  left: { yaw: -HALF_PI, pitch: 0 },
  top: { yaw: 0, pitch: HALF_PI },
  bottom: { yaw: 0, pitch: -HALF_PI },
  iso: { yaw: Math.PI / 4, pitch: Math.atan(1 / Math.SQRT2) },
};

/**
 * Turntable orbit camera with CAD-style controls. The basis is built from yaw/pitch directly,
 * so straight top/bottom views (pitch = +-90 deg) have a well-defined "up" (no lookAt singularity).
 */
export class OrbitCamera {
  target: Vec3 = [0, 0, 0];
  distance = 100;
  yaw = PRESET_ANGLES.iso.yaw;
  pitch = PRESET_ANGLES.iso.pitch;
  orthographic = false;
  /**
   * Off-centre projection (NDC units): the orbit target is drawn at the centre of the area not
   * covered by viewport overlays (toolbar, legend, hints) instead of the canvas centre.
   */
  lens: [number, number] = [0, 0];
  private animation: { from: CameraState; to: CameraState; start: number; duration: number } | null = null;

  state(): CameraState {
    return { target: [...this.target], distance: this.distance, yaw: this.yaw, pitch: this.pitch };
  }

  basis(): CameraBasis {
    const cp = Math.cos(this.pitch);
    const sp = Math.sin(this.pitch);
    const cy = Math.cos(this.yaw);
    const sy = Math.sin(this.yaw);
    const back: Vec3 = [cp * sy, sp, cp * cy];
    const right: Vec3 = [cy, 0, -sy];
    return { right, up: cross(back, right), back };
  }

  eye(): Vec3 {
    return add(this.target, scale(this.basis().back, this.distance));
  }

  /** Half the visible height at the target plane (also the orthographic half-height). */
  halfHeight(): number {
    return this.distance * Math.tan(FOV_Y / 2);
  }

  /** Column-major view matrix (inverse of the camera frame). */
  viewMatrix(): Float32Array {
    const { right: r, up: u, back: b } = this.basis();
    const e = this.eye();
    // prettier-ignore
    return new Float32Array([
      r[0], u[0], b[0], 0,
      r[1], u[1], b[1], 0,
      r[2], u[2], b[2], 0,
      -dot(r, e), -dot(u, e), -dot(b, e), 1,
    ]);
  }

  /**
   * Reversed-Z projection (depth 1 = near, 0 = far; clear to 0 and test "greater").
   * Perspective uses an infinite far plane, so huge and tiny scenes keep depth precision.
   */
  projectionMatrix(aspect: number, sceneRadius: number): Float32Array {
    const m = this.centeredProjection(aspect, sceneRadius);
    // Shift clip x/y by lens * w (row 0 / row 1 += lens * row 3), column-major.
    for (let c = 0; c < 4; c++) {
      m[c * 4] = m[c * 4]! + this.lens[0] * m[c * 4 + 3]!;
      m[c * 4 + 1] = m[c * 4 + 1]! + this.lens[1] * m[c * 4 + 3]!;
    }
    return m;
  }

  private centeredProjection(aspect: number, sceneRadius: number): Float32Array {
    if (this.orthographic) {
      const h = this.halfHeight();
      const w = h * aspect;
      const extent = this.distance + sceneRadius * 4;
      const near = -extent;
      const far = extent;
      // prettier-ignore
      return new Float32Array([
        1 / w, 0, 0, 0,
        0, 1 / h, 0, 0,
        0, 0, 1 / (far - near), 0,
        0, 0, far / (far - near), 1,
      ]);
    }
    const f = 1 / Math.tan(FOV_Y / 2);
    const near = Math.max(this.distance * 1e-3, 1e-5);
    // prettier-ignore
    return new Float32Array([
      f / aspect, 0, 0, 0,
      0, f, 0, 0,
      0, 0, 0, -1,
      0, 0, near, 0,
    ]);
  }

  orbit(dxPx: number, dyPx: number) {
    this.animation = null;
    this.yaw -= dxPx * ORBIT_RADIANS_PER_PIXEL;
    this.pitch = clamp(this.pitch + dyPx * ORBIT_RADIANS_PER_PIXEL, -HALF_PI, HALF_PI);
  }

  pan(dxPx: number, dyPx: number, viewportHeightPx: number) {
    this.animation = null;
    const { right, up } = this.basis();
    const worldPerPixel = (2 * this.halfHeight()) / viewportHeightPx;
    this.target = add(this.target, add(scale(right, -dxPx * worldPerPixel), scale(up, dyPx * worldPerPixel)));
  }

  /** Zoom keeping the point under the cursor fixed (on the focal plane). ndc in [-1, 1], y up. */
  zoomAt(factor: number, ndcX: number, ndcY: number, aspect: number) {
    this.animation = null;
    const focal = this.focalPoint(ndcX, ndcY, aspect);
    const next = clamp(this.distance * factor, MIN_DISTANCE, MAX_DISTANCE);
    const applied = next / this.distance;
    this.distance = next;
    this.target = add(focal, scale(sub(this.target, focal), applied));
  }

  ray(ndcX: number, ndcY: number, aspect: number): { origin: Vec3; dir: Vec3 } {
    const focal = this.focalPoint(ndcX, ndcY, aspect);
    const { back } = this.basis();
    if (this.orthographic) {
      return { origin: add(focal, scale(back, this.distance * 100 + 1e5)), dir: scale(back, -1) };
    }
    const eye = this.eye();
    return { origin: eye, dir: normalize(sub(focal, eye)) };
  }

  /** `safe`: fraction of the viewport width / height left free by overlays. */
  fitState(bounds: Bounds, aspect: number, safe = { w: 1, h: 1 }): Pick<CameraState, "target" | "distance"> {
    const size = boundsSize(bounds);
    const radius = Math.max(Math.hypot(size[0], size[1], size[2]) / 2, 1e-3);
    const t = Math.tan(FOV_Y / 2);
    const halfFov = Math.min(Math.atan(t * safe.h), Math.atan(t * aspect * safe.w));
    return { target: boundsCenter(bounds), distance: (radius / Math.sin(halfFov)) * 1.08 };
  }

  animateTo(to: Partial<CameraState>, duration = 320) {
    // Reduced motion: no fly-over, the view simply changes.
    if (matchMedia("(prefers-reduced-motion: reduce)").matches) return this.jumpTo(to);
    const from = this.state();
    const target: CameraState = { ...from, ...to };
    // Rotate the short way round.
    let dyaw = (target.yaw - from.yaw) % (Math.PI * 2);
    if (dyaw > Math.PI) dyaw -= Math.PI * 2;
    if (dyaw < -Math.PI) dyaw += Math.PI * 2;
    target.yaw = from.yaw + dyaw;
    this.animation = { from, to: target, start: performance.now(), duration };
  }

  jumpTo(to: Partial<CameraState>) {
    this.animation = null;
    Object.assign(this, to);
  }

  /** Advances an in-flight animation. Returns true while still animating. */
  update(now: number): boolean {
    const a = this.animation;
    if (!a) return false;
    const t = clamp((now - a.start) / a.duration, 0, 1);
    const k = easeOutCubic(t);
    const lerp = (x: number, y: number) => x + (y - x) * k;
    this.yaw = lerp(a.from.yaw, a.to.yaw);
    this.pitch = lerp(a.from.pitch, a.to.pitch);
    // Interpolate distance geometrically so large zoom changes feel even.
    this.distance = a.from.distance * (a.to.distance / a.from.distance) ** k;
    this.target = [lerp(a.from.target[0], a.to.target[0]), lerp(a.from.target[1], a.to.target[1]), lerp(a.from.target[2], a.to.target[2])];
    if (t >= 1) this.animation = null;
    return this.animation !== null;
  }

  private focalPoint(ndcX: number, ndcY: number, aspect: number): Vec3 {
    const { right, up } = this.basis();
    const hh = this.halfHeight();
    const x = ndcX - this.lens[0];
    const y = ndcY - this.lens[1];
    return add(this.target, add(scale(right, x * hh * aspect), scale(up, y * hh)));
  }
}
