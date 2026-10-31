import type { CameraState } from "./camera";

/** Read access to the live camera (saved with the project) while the viewport is mounted. */
let current: (() => CameraState) | null = null;

export function registerCamera(get: () => CameraState): () => void {
  current = get;
  return () => {
    if (current === get) current = null;
  };
}

export function cameraState(): CameraState | null {
  return current ? current() : null;
}

/** PNG of the 3D view (results export) while the viewport is mounted. */
let capture: (() => Promise<Blob>) | null = null;
let settle: (() => Promise<void>) | null = null;

export function registerCapture(fn: () => Promise<Blob>, layout: () => Promise<void>): () => void {
  capture = fn;
  settle = layout;
  return () => {
    if (capture === fn) {
      capture = null;
      settle = null;
    }
  };
}

/** Test/automation hook for a frame after the current results layout has committed. */
export function settleViewportLayout(): Promise<void> {
  if (!settle) return Promise.reject(new Error("the 3D view is not mounted"));
  return settle();
}

export function captureViewport(): Promise<Blob> | null {
  return capture ? capture() : null;
}
