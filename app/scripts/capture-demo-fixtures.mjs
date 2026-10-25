// Records the demo-mode fixtures (src/demo/fixtures/einzel.json) from the real desktop app: the
// einzel template's .pa# packet, every refine channel message, the refined field, a fast adjust and
// live / precise flights, all as the native core sends them, so demo mode runs the real decoders.
// Run with Windows node from a /mnt/c cwd while the debug app runs with CDP on port 9333
// (docs/dev/frontend.md, "Demo mode"):
//   node capture-demo-fixtures.mjs <output .json path>
import { writeFileSync } from "node:fs";

const port = process.env.CDP_PORT || "9333";
const [out] = process.argv.slice(2);
if (!out) throw new Error("usage: node capture-demo-fixtures.mjs <output .json>");

const list = await (await fetch(`http://127.0.0.1:${port}/json/list`)).json();
const page = list.find((p) => p.type === "page" && p.url.includes("1420")) ?? list.find((p) => p.type === "page");
if (!page) throw new Error("no page " + JSON.stringify(list));
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
await new Promise((r, j) => {
  ws.onopen = r;
  ws.onerror = j;
});
const send = (method, params = {}) =>
  new Promise((r) => {
    const i = ++id;
    pending.set(i, r);
    ws.send(JSON.stringify({ id: i, method, params }));
  });

const PAGE = String.raw`(async () => {
  const b = await import("/src/scene/backend.ts");
  const sim = await import("/src/scene/simulation.ts");
  const project = await import("/src/state/project.ts");
  const { useApp } = await import("/src/state/store.ts");
  const b64 = (buf) => { const u = new Uint8Array(buf); let s = ""; for (let i = 0; i < u.length; i += 0x8000) s += String.fromCharCode(...u.subarray(i, i + 0x8000)); return btoa(s); };

  // The app's own template run gives the exact fly request the UI sends for this template.
  useApp.setState({ dirty: false });
  await project.newFromTemplate("einzel");
  const app = useApp.getState().objects.find((o) => o.kind === "pa");
  const request = { ...sim.flyRequest(app), id: "demo" };

  const created = await b.createTemplate("einzel");
  const DEMO_DIR = "C:\\Users\\demo\\Documents\\SimRep\\einzel";
  const clean = (s) => s.split(created.dir).join(DEMO_DIR);
  const cleanJson = (v) =>
    typeof v === "string" ? clean(v) : Array.isArray(v) ? v.map(cleanJson) : v && typeof v === "object" ? Object.fromEntries(Object.entries(v).map(([k, x]) => [k, cleanJson(x)])) : v;
  // Rewrites a packet's JSON header without the local folder (pad: the packet's header alignment).
  const rehead = (buf, pad) => {
    const bytes = new Uint8Array(buf);
    const len = new DataView(buf).getUint32(4, true);
    const head = JSON.parse(new TextDecoder().decode(bytes.subarray(8, 8 + len)));
    const text = JSON.stringify(cleanJson(head));
    if (text === JSON.stringify(head)) return buf;
    const next = new TextEncoder().encode(text);
    const padded = Math.ceil(next.length / pad) * pad;
    const outBuf = new ArrayBuffer(8 + padded + buf.byteLength - 8 - len);
    const o = new Uint8Array(outBuf);
    o.set(bytes.subarray(0, 4));
    new DataView(outBuf).setUint32(4, padded, true);
    o.fill(32, 8, 8 + padded);
    o.set(next, 8);
    o.set(bytes.subarray(8 + len), 8 + padded);
    return outBuf;
  };
  const packet = (buf) => {
    const magic = String.fromCharCode(...new Uint8Array(buf, 0, 4));
    const pad = magic === "STR4" ? 8 : 4;
    return b64(magic === "SPV1" ? buf : rehead(buf, pad));
  };

  const info = cleanJson(await b.pathInfo([created.path]));
  const opened = packet(await b.openPa("demo", created.path));
  const targets = cleanJson(await b.refineTargets("demo", null));
  const messages = [];
  const t0 = performance.now();
  await new Promise((resolve) => {
    void b.refineStart("demo", null, "auto", (m) => {
      const at = Math.round(performance.now() - t0);
      if (m instanceof ArrayBuffer) messages.push({ at, packet: packet(m) });
      else {
        messages.push({ at, json: cleanJson(m) });
        if (m.type === "done" || m.type === "error" || m.type === "cancelled") resolve();
      }
    });
  });
  const voltages = [[1, 0], [2, 80], [3, 0]];
  const labels = b64(await b.electrodeLabels("demo"));
  const fast = packet(await b.fastAdjust("demo", voltages));
  const flights = {};
  for (const precision of ["live", "precise"]) {
    let buf = await b.fly({ ...request, voltages, precision });
    // The first live flight may run on the CPU while the field goes up to the GPU: fly again.
    for (let i = 0; i < 5 && precision === "live" && JSON.parse(new TextDecoder().decode(new Uint8Array(buf, 8, new DataView(buf).getUint32(4, true)))).fallback; i++) {
      await new Promise((r) => setTimeout(r, 400));
      buf = await b.fly({ ...request, voltages, precision });
    }
    const head = JSON.parse(new TextDecoder().decode(new Uint8Array(buf, 8, new DataView(buf).getUint32(4, true))));
    const velocities = await b.flightVelocities("demo", head.token);
    flights[precision] = { packet: packet(buf), velocities: b64(velocities.buffer) };
  }
  const trace = b64(await b.ionTrace({ ...request, voltages }, 0));
  b.closeSession("demo");
  const computeDevice = await b.computeDevice();
  return { created: { ...cleanJson(created), path: DEMO_DIR + "\\einzel.pa#" }, info, opened, targets, messages, labels, fast, flights, trace, computeDevice, request: { ...request, voltages } };
})()`;

const m = await send("Runtime.evaluate", { expression: PAGE, awaitPromise: true, returnByValue: true });
if (m.result?.exceptionDetails) throw new Error(JSON.stringify(m.result.exceptionDetails).slice(0, 3000));
const value = m.result.result.value;
// Keep each solution's last field preview (the one before its stageDone); the rest only add size.
const isPreview = (x) => x?.packet && Buffer.from(x.packet.slice(0, 8), "base64").subarray(0, 4).toString() === "SPV1";
value.messages = value.messages.filter((x, i, all) => !isPreview(x) || all[i + 1]?.json?.type === "stageDone");
writeFileSync(out, JSON.stringify(value) + "\n");
const progress = value.messages.filter((x) => x.json?.type === "progress").length;
console.log(`wrote ${out}: ${value.messages.length} refine messages (${progress} progress), flights ${Object.keys(value.flights).join(", ")}`);
ws.close();
