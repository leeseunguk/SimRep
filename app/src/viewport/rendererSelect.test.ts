import { describe, expect, it } from "vitest";
import {
  RendererCancelledError,
  RendererUnavailableError,
  selectRenderer,
  WebGl2UnavailableError,
  WebGpuUnavailableError,
} from "./rendererSelect";

const calls: string[] = [];
const ok = (name: string) => () => {
  calls.push(name);
  return Promise.resolve(name);
};
const fail = (name: string, err: Error) => () => {
  calls.push(name);
  return Promise.reject(err);
};

describe("selectRenderer", () => {
  it("prefers WebGPU and never touches WebGL2 when it works", async () => {
    calls.length = 0;
    await expect(selectRenderer({ webgpu: ok("webgpu"), webgl2: ok("webgl2") })).resolves.toEqual({ renderer: "webgpu", webgpuReason: null });
    expect(calls).toEqual(["webgpu"]);
  });

  it("falls back to WebGL2 when WebGPU is unavailable, keeping the reason", async () => {
    const r = await selectRenderer({ webgpu: fail("webgpu", new WebGpuUnavailableError("no adapter")), webgl2: ok("webgl2") });
    expect(r).toEqual({ renderer: "webgl2", webgpuReason: "no adapter" });
  });

  it("reports both reasons when neither API can draw", async () => {
    const r = selectRenderer({
      webgpu: fail("webgpu", new WebGpuUnavailableError("no navigator.gpu.")),
      webgl2: () => {
        throw new WebGl2UnavailableError("no webgl2.");
      },
    });
    await expect(r).rejects.toBeInstanceOf(RendererUnavailableError);
    await expect(r).rejects.toThrow("no navigator.gpu. no webgl2.");
  });

  it("does not fall back on WebGPU pipeline errors or cancellation (the canvas already has a WebGPU context)", async () => {
    calls.length = 0;
    await expect(selectRenderer({ webgpu: fail("webgpu", new Error("WebGPU pipeline setup failed: x")), webgl2: ok("webgl2") })).rejects.toThrow(
      "pipeline setup failed",
    );
    await expect(selectRenderer({ webgpu: fail("webgpu", new RendererCancelledError()), webgl2: ok("webgl2") })).rejects.toBeInstanceOf(
      RendererCancelledError,
    );
    expect(calls).toEqual(["webgpu", "webgpu"]);
  });

  it("passes WebGL2 shader errors through instead of showing the fallback card", async () => {
    const r = selectRenderer({
      webgpu: fail("webgpu", new WebGpuUnavailableError("no adapter")),
      webgl2: () => {
        throw new Error("WebGL2 mesh vertex shader: bad");
      },
    });
    await expect(r).rejects.not.toBeInstanceOf(RendererUnavailableError);
  });
});
