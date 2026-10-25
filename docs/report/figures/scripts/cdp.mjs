// CDP driver for the report captures of the real SimRep Windows app (WebView2).
// Run with Windows node from a /mnt/c cwd (the script is copied to C: by capture.py):
//   node cdp.mjs eval <file.js> [json-args]   run file body as an async function, args in window.__args
//   node cdp.mjs shot <out.png> [x,y,w,h]     screenshot (CSS px clip optional)
//   node cdp.mjs evalshot <file.js> <json-args> <out.png> [x,y,w,h]  run a script, then screenshot
//   node cdp.mjs dragshot <out.png> x1,y1,x2,y2,steps,holdMs [x,y,w,h]   screenshot during a real mouse drag
// Viewport is emulated at METRICS (default 1440x900) with deviceScaleFactor DSF (default 2),
// so a capture is 2x CSS pixels and stays sharp when cropped to one panel.
import { readFileSync, writeFileSync } from "node:fs";

const port = process.env.CDP_PORT || "9333";
const [cmd, a1, a2] = process.argv.slice(2);
const list = await (await fetch(`http://127.0.0.1:${port}/json/list`)).json();
const page = list.find((p) => p.type === "page" && p.url.includes("1420")) ?? list.find((p) => p.type === "page");
if (!page) { console.error("no page", JSON.stringify(list)); process.exit(2); }
const ws = new WebSocket(page.webSocketDebuggerUrl);
let id = 0;
const pending = new Map();
ws.onmessage = (e) => { const m = JSON.parse(e.data); if (m.id && pending.has(m.id)) { pending.get(m.id)(m); pending.delete(m.id); } };
await new Promise((r, j) => { ws.onopen = r; ws.onerror = j; });
const send = (method, params = {}) => new Promise((r) => { const i = ++id; pending.set(i, r); ws.send(JSON.stringify({ id: i, method, params })); });
const [mw, mh] = (process.env.METRICS || "1440x900").split("x").map(Number);
const dsf = Number(process.env.DSF || "2");
await send("Emulation.setDeviceMetricsOverride", { width: mw, height: mh, deviceScaleFactor: dsf, mobile: false });
await new Promise((r) => setTimeout(r, 400));

if (cmd === "eval") {
  const body = readFileSync(a1, "utf8");
  const args = a2 ?? "null";
  const m = await send("Runtime.evaluate", {
    expression: `(async () => { window.__args = ${args}; ${body} })()`,
    awaitPromise: true,
    returnByValue: true,
  });
  if (m.result?.exceptionDetails) { console.log("EXCEPTION", JSON.stringify(m.result.exceptionDetails).slice(0, 3000)); process.exitCode = 1; }
  else console.log(JSON.stringify(m.result?.result?.value ?? null));
} else if (cmd === "evalshot") {
  // Run a script and screenshot in the same connection (keeps transient scroll/hover state).
  const [, , , , out, clip] = process.argv.slice(1);
  const body = readFileSync(a1, "utf8");
  const m = await send("Runtime.evaluate", { expression: `(async () => { window.__args = ${a2 ?? "null"}; ${body} })()`, awaitPromise: true, returnByValue: true });
  console.log(JSON.stringify(m.result?.exceptionDetails ?? m.result?.result?.value ?? null).slice(0, 3000));
  await new Promise((r) => setTimeout(r, 300));
  const params = { format: "png" };
  if (clip) { const [x, y, width, height] = clip.split(",").map(Number); params.clip = { x, y, width, height, scale: 1 }; }
  const s = await send("Page.captureScreenshot", params);
  writeFileSync(out, Buffer.from(s.result.data, "base64"));
  console.log("saved", out);
} else if (cmd === "dragshot") {
  // Real pointer drag: press at (x1,y1), move in steps to (x2,y2), screenshot while still pressed, release.
  const [x1, y1, x2, y2, steps, hold] = a2.split(",").map(Number);
  const clip = process.argv[5];
  const mouse = (type, x, y, buttons) => send("Input.dispatchMouseEvent", { type, x, y, button: "left", buttons, clickCount: 1 });
  await mouse("mouseMoved", x1, y1, 0);
  await mouse("mousePressed", x1, y1, 1);
  for (let i = 1; i <= steps; i++) {
    await mouse("mouseMoved", x1 + ((x2 - x1) * i) / steps, y1 + ((y2 - y1) * i) / steps, 1);
    await new Promise((r) => setTimeout(r, 16));
  }
  await new Promise((r) => setTimeout(r, hold || 800));
  const params = { format: "png" };
  if (clip) { const [x, y, width, height] = clip.split(",").map(Number); params.clip = { x, y, width, height, scale: 1 }; }
  const m = await send("Page.captureScreenshot", params);
  writeFileSync(a1, Buffer.from(m.result.data, "base64"));
  await mouse("mouseReleased", x2, y2, 0);
  console.log("saved", a1);
} else if (cmd === "shot") {
  const params = { format: "png", captureBeyondViewport: false };
  if (a2) {
    const [x, y, width, height] = a2.split(",").map(Number);
    params.clip = { x, y, width, height, scale: 1 };
  }
  const m = await send("Page.captureScreenshot", params);
  writeFileSync(a1, Buffer.from(m.result.data, "base64"));
  console.log("saved", a1);
}
ws.close();
