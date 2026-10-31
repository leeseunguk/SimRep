// Which graphics API draws the 3D view. Kept free of GPU code so the policy is unit-testable.

/** WebGPU is missing (no navigator.gpu, no adapter, no device or context): try WebGL2 next. */
export class WebGpuUnavailableError extends Error {}

/** No WebGL2 context on this canvas. */
export class WebGl2UnavailableError extends Error {}

/** Neither WebGPU nor WebGL2 can draw: the UI shows the fallback card. */
export class RendererUnavailableError extends Error {}

/** The owner unmounted while the device was being requested. */
export class RendererCancelledError extends Error {}

export interface RendererFactories<R> {
  webgpu(): Promise<R>;
  webgl2(): R | Promise<R>;
}

/**
 * Prefers WebGPU; falls back to WebGL2 only when WebGPU is unavailable (a WebGPU pipeline error
 * or a cancellation is passed through: the canvas already belongs to WebGPU then).
 * Returns the renderer and, for the fallback, why WebGPU was not used.
 */
export async function selectRenderer<R>(f: RendererFactories<R>): Promise<{ renderer: R; webgpuReason: string | null }> {
  let webgpuReason: string;
  try {
    return { renderer: await f.webgpu(), webgpuReason: null };
  } catch (err) {
    if (!(err instanceof WebGpuUnavailableError)) throw err;
    webgpuReason = err.message;
  }
  try {
    return { renderer: await f.webgl2(), webgpuReason };
  } catch (err) {
    if (!(err instanceof WebGl2UnavailableError)) throw err;
    throw new RendererUnavailableError(`${webgpuReason} ${err.message}`);
  }
}
