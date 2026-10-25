import { parseStl, StlError } from "./stl";

export type StlWorkerRequest = { id: number; buffer: ArrayBuffer };
export type StlWorkerResponse =
  | {
      id: number;
      ok: true;
      vertices: Float32Array<ArrayBuffer>;
      triangleCount: number;
      bounds: { min: [number, number, number]; max: [number, number, number] };
      format: "binary" | "ascii";
      ms: number;
    }
  | { id: number; ok: false; code: string; params: Record<string, number>; message: string };

// The app compiles against the DOM lib; describe just the worker-scope surface used here.
interface WorkerScope {
  onmessage: ((event: MessageEvent<StlWorkerRequest>) => void) | null;
  postMessage(message: StlWorkerResponse, transfer?: Transferable[]): void;
}
const ctx = self as unknown as WorkerScope;

ctx.onmessage = (event: MessageEvent<StlWorkerRequest>) => {
  const { id, buffer } = event.data;
  const start = performance.now();
  try {
    const r = parseStl(buffer);
    const response: StlWorkerResponse = {
      id,
      ok: true,
      vertices: r.vertices,
      triangleCount: r.triangleCount,
      bounds: r.bounds,
      format: r.format,
      ms: Math.round(performance.now() - start),
    };
    ctx.postMessage(response, [r.vertices.buffer]);
  } catch (err) {
    const response: StlWorkerResponse =
      err instanceof StlError
        ? { id, ok: false, code: err.code, params: err.params, message: err.message }
        : { id, ok: false, code: "unexpected", params: {}, message: String(err) };
    ctx.postMessage(response);
  }
};
