import { expect, test } from "@playwright/test";

test("WebGL2 releases all ten programs and VAOs on repeated teardown and shader failure", async ({ page }) => {
  await page.goto("/?replay=manual");
  const cycles = await page.evaluate(async () => {
    const url = "/src/viewport/glRenderer.ts";
    const { WebGl2Renderer } = (await import(/* @vite-ignore */ url)) as typeof import("../src/viewport/glRenderer");
    const canvas = document.createElement("canvas");
    const gl = canvas.getContext("webgl2")!;
    if (!gl) throw new Error("WebGL2 is required for the lifecycle scenario");
    const created = new Set<WebGLProgram>();
    const deleted = new Set<WebGLProgram>();
    const vaosCreated = new Set<WebGLVertexArrayObject>();
    const vaosDeleted = new Set<WebGLVertexArrayObject>();
    const create = gl.createProgram.bind(gl);
    const remove = gl.deleteProgram.bind(gl);
    const createVao = gl.createVertexArray.bind(gl);
    const removeVao = gl.deleteVertexArray.bind(gl);
    gl.createVertexArray = () => {
      const vao = createVao();
      if (vao) vaosCreated.add(vao);
      return vao;
    };
    gl.deleteVertexArray = (vao) => {
      if (vao) vaosDeleted.add(vao);
      removeVao(vao);
    };
    gl.createProgram = () => {
      const program = create();
      if (program) created.add(program);
      return program;
    };
    gl.deleteProgram = (program) => {
      if (program) deleted.add(program);
      remove(program);
    };
    const record = () => ({ created: created.size, deleted: deleted.size, vaosCreated: vaosCreated.size, vaosDeleted: vaosDeleted.size });
    const records = [];
    for (let i = 0; i < 3; i++) {
      const renderer = WebGl2Renderer.create(canvas);
      renderer.destroy();
      records.push(record());
    }
    const source = gl.shaderSource.bind(gl);
    let shaders = 0;
    gl.shaderSource = (shader, code) => source(shader, ++shaders === 13 ? "invalid shader" : code);
    let error = "";
    try {
      WebGl2Renderer.create(canvas);
    } catch (caught) {
      error = String(caught);
    }
    records.push(record());
    return { records, error };
  });
  expect(cycles.records.slice(0, 3)).toEqual([
    { created: 10, deleted: 10, vaosCreated: 2, vaosDeleted: 2 },
    { created: 20, deleted: 20, vaosCreated: 4, vaosDeleted: 4 },
    { created: 30, deleted: 30, vaosCreated: 6, vaosDeleted: 6 },
  ]);
  expect(cycles.error).toContain("shader");
  const failed = cycles.records[3]!;
  expect(failed.created).toBeGreaterThan(30);
  expect(failed.deleted).toBe(failed.created);
  expect(failed.vaosCreated).toBe(7);
  expect(failed.vaosDeleted).toBe(failed.vaosCreated);
  await test.info().attach("program-lifecycle", { body: JSON.stringify(cycles, null, 2), contentType: "application/json" });
});

test("WebGL2 electrode texel lookup uses WGSL ties-to-even at half texels", async ({ page }) => {
  await page.goto("/?replay=manual");
  const pixels = await page.evaluate(async () => {
    const url = "/src/viewport/glShaders.ts";
    const { glShaders } = (await import(/* @vite-ignore */ url)) as typeof import("../src/viewport/glShaders");
    const expression = glShaders(false).slice.fs.match(/vec2 near = ([^;]+);/)?.[1];
    if (!expression) throw new Error("production electrode lookup expression not found");
    const canvas = document.createElement("canvas");
    canvas.width = 8;
    canvas.height = 1;
    const gl = canvas.getContext("webgl2", { antialias: false })!;
    if (!gl) throw new Error("WebGL2 is required for the half-texel pixel scenario");
    const program = gl.createProgram()!;
    const compile = (type: number, code: string) => {
      const shader = gl.createShader(type)!;
      gl.shaderSource(shader, code);
      gl.compileShader(shader);
      if (!gl.getShaderParameter(shader, gl.COMPILE_STATUS)) throw new Error(gl.getShaderInfoLog(shader) ?? "shader compilation failed");
      gl.attachShader(program, shader);
      gl.deleteShader(shader);
    };
    compile(gl.VERTEX_SHADER, `#version 300 es
      void main() {
        vec2 p = vec2((gl_VertexID << 1) & 2, gl_VertexID & 2);
        gl_Position = vec4(p * 2.0 - 1.0, 0.0, 1.0);
      }`);
    compile(gl.FRAGMENT_SHADER, `#version 300 es
      precision highp float;
      precision highp int;
      out vec4 color;
      vec2 texel(int i, int j) { return vec2(i, j); }
      void main() {
        float values[8] = float[8](0.5, 1.5, 2.5, 3.5, 0.499, 0.501, 2.499, 2.501);
        vec2 g = vec2(values[int(gl_FragCoord.x)], 2.5);
        vec2 near = ${expression};
        color = vec4(near / 255.0, 0.0, 1.0);
      }`);
    gl.linkProgram(program);
    if (!gl.getProgramParameter(program, gl.LINK_STATUS)) throw new Error(gl.getProgramInfoLog(program) ?? "link failed");
    gl.useProgram(program);
    gl.viewport(0, 0, 8, 1);
    gl.drawArrays(gl.TRIANGLES, 0, 3);
    const bytes = new Uint8Array(32);
    gl.readPixels(0, 0, 8, 1, gl.RGBA, gl.UNSIGNED_BYTE, bytes);
    const error = gl.getError();
    gl.deleteProgram(program);
    if (error !== gl.NO_ERROR) throw new Error(`WebGL error ${error}`);
    return Array.from({ length: 8 }, (_, i) => [bytes[i * 4], bytes[i * 4 + 1]]);
  });
  expect(pixels).toEqual([[0, 2], [2, 2], [2, 2], [4, 2], [0, 2], [1, 2], [2, 2], [3, 2]]);
  await test.info().attach("half-texel-pixels", { body: JSON.stringify(pixels), contentType: "application/json" });
});
