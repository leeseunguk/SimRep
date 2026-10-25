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

export function registerCapture(fn: () => Promise<Blob>): () => void {
  capture = fn;
  return () => {
    if (capture === fn) capture = null;
  };
}

export function captureViewport(): Promise<Blob> | null {
  return capture ? capture() : null;
}
