// Runs funccheck.js in the app at 1280x800 with prefers-reduced-motion emulated, after the plan's einzel setup.
//   node funccheck.mjs <out.json>
import { readFileSync, writeFileSync } from "node:fs";
import { PRELUDE, surfaces } from "./plan.mjs";

const list = await (await fetch(`http://127.0.0.1:${process.env.CDP_PORT || "9333"}/json/list`)).json();
const page = list.find((p) => p.type === "page" && p.url.includes("1420")) ?? list.find((p) => p.type === "page");
const ws = new WebSocket(page.webSocketDebuggerUrl);
let id = 0;
const pending = new Map();
ws.onmessage = (e) => {
  const m = JSON.parse(e.data);
  if (m.id && pending.has(m.id)) {
    pending.get(m.id)(m);
    pending.delete(m.id);
  }
};
await new Promise((r) => (ws.onopen = r));
const send = (method, params = {}) => new Promise((r) => { const i = ++id; pending.set(i, r); ws.send(JSON.stringify({ id: i, method, params })); });
const run = async (body) => {
  const m = await send("Runtime.evaluate", { expression: `(async () => { ${body} })()`, awaitPromise: true, returnByValue: true });
  if (m.result?.exceptionDetails) throw new Error(JSON.stringify(m.result.exceptionDetails).slice(0, 1500));
  return m.result.result.value;
};
await send("Emulation.setDeviceMetricsOverride", { width: 1280, height: 800, deviceScaleFactor: 1, mobile: false });
await run(`${PRELUDE}\n${surfaces.find((s) => s.id === "template-einzel").setup}`);
await send("Emulation.setEmulatedMedia", { features: [{ name: "prefers-reduced-motion", value: "reduce" }] });
const out = await run(readFileSync(new URL("./funccheck.js", import.meta.url), "utf8"));
await send("Emulation.setEmulatedMedia", { features: [] });
await send("Emulation.clearDeviceMetricsOverride");
writeFileSync(process.argv[2], JSON.stringify(out, null, 1));
console.log(JSON.stringify(out, null, 1));
ws.close();
