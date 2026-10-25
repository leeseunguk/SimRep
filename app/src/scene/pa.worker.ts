import { PaReadError, type PaErrorCode, type PaInfo } from "./pa";
import { decodePa } from "./paDecode";
import { buildSurface } from "./paSurface";

export type PaWorkerRequest =
  | { id: number; type: "decode"; buffer: ArrayBuffer }
  | {
      id: number;
      type: "surface";
      info: PaInfo;
      potentials: Float32Array<ArrayBuffer>;
      electrodes: Uint8Array<ArrayBuffer>;
      labels: Uint16Array<ArrayBuffer> | null;
      maxQuads: number;
    };

export type PaWorkerResponse =
  | { id: number; ok: true; type: "decode"; packet: ArrayBuffer }
  | { id: number; ok: true; type: "surface"; quads: Float32Array<ArrayBuffer>; count: number; step: number; ms: number }
  | { id: number; ok: false; code: PaErrorCode; detail: string };

interface WorkerScope {
  onmessage: ((event: MessageEvent<PaWorkerRequest>) => void) | null;
  postMessage(message: PaWorkerResponse, transfer?: Transferable[]): void;
}
const ctx = self as unknown as WorkerScope;

ctx.onmessage = (event: MessageEvent<PaWorkerRequest>) => {
  const req = event.data;
  try {
    if (req.type === "decode") {
      const packet = decodePa(req.buffer);
      ctx.postMessage({ id: req.id, ok: true, type: "decode", packet }, [packet]);
    } else {
      const start = performance.now();
      const s = buildSurface(req.info, { potentials: req.potentials, electrodes: req.electrodes }, req.labels, req.maxQuads);
      const ms = Math.round(performance.now() - start);
      ctx.postMessage({ id: req.id, ok: true, type: "surface", quads: s.quads, count: s.count, step: s.step, ms }, [s.quads.buffer]);
    }
  } catch (err) {
    const response: PaWorkerResponse =
      err instanceof PaReadError
        ? { id: req.id, ok: false, code: err.code, detail: err.detail }
        : { id: req.id, ok: false, code: "unexpected", detail: err instanceof Error ? err.message : String(err) };
    ctx.postMessage(response);
  }
};
