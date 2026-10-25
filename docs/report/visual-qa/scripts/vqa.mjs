// Visual QA capture driver for the real SimRep Windows app (WebView2 over CDP).
// Run with Windows node from a /mnt/c cwd, the app started with --remote-debugging-port=9333:
//   node vqa.mjs <outDir> <phase> [surface ids...]
// For every surface in plan.mjs: run its setup in the page, then for light and dark theme and for
// each size (1280x800 CSS at 2x, 1920x1080 CSS at 1x) set the viewport, run its prep, and take a
// screenshot <id>-<w>x<h>-<theme>-<phase>.png. Writes/merges <outDir>/manifest.json.
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { PRELUDE, surfaces } from "./plan.mjs";

const port = process.env.CDP_PORT || "9333";
const [outDir, phase, ...only] = process.argv.slice(2);
// AUDIT_ONLY=1: no screenshots, 1280x800 only, DOM audit per theme (manifest key `audits`).
const AUDIT_ONLY = process.env.AUDIT_ONLY === "1";
const SIZES = [
  { w: 1280, h: 800, dsf: 2 },
  { w: 1920, h: 1080, dsf: 1 },
].slice(0, AUDIT_ONLY ? 1 : 2);
const AUDIT = readFileSync(new URL("./audit.js", import.meta.url), "utf8");
const THEMES = ["light", "dark"];

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
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

async function run(body, args = null) {
  const m = await send("Runtime.evaluate", {
    expression: `(async () => { window.__args = ${JSON.stringify(args)}; ${PRELUDE}\n${body} })()`,
    awaitPromise: true,
    returnByValue: true,
  });
  if (m.result?.exceptionDetails) throw new Error(JSON.stringify(m.result.exceptionDetails).slice(0, 2000));
  return m.result?.result?.value ?? null;
}

function pngSize(buf) {
  if (buf.readUInt32BE(0) !== 0x89504e47) throw new Error("not a PNG");
  return [buf.readUInt32BE(16), buf.readUInt32BE(20)];
}

mkdirSync(outDir, { recursive: true });
const manifestPath = join(outDir, "manifest.json");
const manifest = existsSync(manifestPath) ? JSON.parse(readFileSync(manifestPath, "utf8")) : { phase, captures: [], surfaces: {} };
manifest.audits ??= {};
const todo = surfaces.filter((s) => !only.length || only.includes(s.id));

for (const s of todo) {
  const t0 = Date.now();
  let info = null;
  try {
    await send("Emulation.setDeviceMetricsOverride", { width: SIZES[0].w, height: SIZES[0].h, deviceScaleFactor: SIZES[0].dsf, mobile: false });
    if (s.setup && !s.perShot) info = await run(s.setup, s.args ?? null);
    if (!AUDIT_ONLY) manifest.captures = manifest.captures.filter((c) => c.surface !== s.id);
    for (const theme of THEMES) {
      for (const size of SIZES) {
        await send("Emulation.setDeviceMetricsOverride", { width: size.w, height: size.h, deviceScaleFactor: size.dsf, mobile: false });
        await run(`useApp.getState().setThemePref(${JSON.stringify(theme)});`);
        if (s.perShot) info = await run(s.setup, s.args ?? null);
        await sleep(s.wait ?? 900);
        const prep = s.prep ? await run(s.prep, s.args ?? null) : null;
        if (s.prep) await sleep(s.prepWait ?? 500);
        const file = `${s.id}-${size.w}x${size.h}-${theme}-${phase}.png`;
        if (AUDIT_ONLY) {
          manifest.audits[`${s.id}-${theme}`] = await run(AUDIT);
          console.log("audited", s.id, theme);
          continue;
        }
        let shot;
        if (s.drag && prep?.drag) {
          const [x1, y1, x2, y2] = prep.drag;
          const mouse = (type, x, y, buttons) => send("Input.dispatchMouseEvent", { type, x, y, button: "left", buttons, clickCount: 1 });
          await mouse("mouseMoved", x1, y1, 0);
          await mouse("mousePressed", x1, y1, 1);
          for (let i = 1; i <= 24; i++) {
            await mouse("mouseMoved", x1 + ((x2 - x1) * i) / 24, y1 + ((y2 - y1) * i) / 24, 1);
            await sleep(16);
          }
          await sleep(900);
          shot = await send("Page.captureScreenshot", { format: "png" });
          await mouse("mouseReleased", x2, y2, 0);
        } else {
          shot = await send("Page.captureScreenshot", { format: "png" });
        }
        const buf = Buffer.from(shot.result.data, "base64");
        const [pw, ph] = pngSize(buf);
        if (pw !== size.w * size.dsf || ph !== size.h * size.dsf) throw new Error(`bad size ${pw}x${ph} for ${file}`);
        writeFileSync(join(outDir, file), buf);
        manifest.captures.push({ surface: s.id, theme, css: `${size.w}x${size.h}`, dsf: size.dsf, file, px: `${pw}x${ph}`, bytes: buf.length, prep });
        console.log("saved", file, buf.length);
      }
    }
    manifest.surfaces[s.id] = { title: s.title, info, ms: Date.now() - t0, at: new Date().toISOString() };
  } catch (e) {
    manifest.surfaces[s.id] = { title: s.title, error: String(e).slice(0, 2000), at: new Date().toISOString() };
    console.log("FAILED", s.id, String(e).slice(0, 600));
  }
  writeFileSync(manifestPath, JSON.stringify(manifest, null, 1));
}
await send("Emulation.clearDeviceMetricsOverride");
ws.close();
