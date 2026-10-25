// Reduced-motion check (visual-qa G4/H2) on the running app: emulate prefers-reduced-motion,
// start a CPU refine so the progress spinners exist, and read their Web Animations timing
// twice 300 ms apart. A spinner that keeps advancing passes; also reports the rise-in keyframes.
//   node motion.mjs <out.json>
import { writeFileSync } from "node:fs";
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
  const m = await send("Runtime.evaluate", { expression: `(async () => { ${PRELUDE}\n${body} })()`, awaitPromise: true, returnByValue: true });
  if (m.result?.exceptionDetails) throw new Error(JSON.stringify(m.result.exceptionDetails).slice(0, 1500));
  return m.result.result.value;
};
const out = {};
for (const reduce of [false, true]) {
  await send("Emulation.setEmulatedMedia", { features: [{ name: "prefers-reduced-motion", value: reduce ? "reduce" : "no-preference" }] });
  await run(surfaces.find((s) => s.id === "refine-progress").setup);
  out[reduce ? "reduce" : "normal"] = await run(`
    const sample = () => document.getAnimations().filter((a) => a.playState === 'running').map((a) => ({
      name: a.animationName, el: a.effect?.target?.getAttribute?.('class')?.split(' ')[0] ?? a.effect?.target?.tagName,
      duration: a.effect.getComputedTiming().duration, progress: a.effect.getComputedTiming().progress }));
    const a = sample(); await sleep(300); const b = sample();
    const spinners = a.filter((x) => /spin/i.test(x.name)).map((x, i) => ({ ...x, progressLater: b.find((y) => y.name === x.name && y.el === x.el)?.progress }));
    const rise = [...document.styleSheets].flatMap((s) => { try { return [...s.cssRules]; } catch { return []; } })
      .flatMap((r) => r.cssRules ? [r, ...r.cssRules] : [r]).filter((r) => r.name === 'rise-in' && matchMedia(r.parentRule?.conditionText ? '(' + r.parentRule.conditionText.replace(/^\\(|\\)$/g, '') + ')' : 'all').matches)
      .map((r) => r.cssText.replace(/\\s+/g, ' ').slice(0, 160));
    return { reducedMotionMatches: matchMedia('(prefers-reduced-motion: reduce)').matches, spinners, riseIn: rise.at(-1) };`);
}
await send("Emulation.setEmulatedMedia", { features: [] });
writeFileSync(process.argv[2], JSON.stringify(out, null, 1));
console.log(JSON.stringify(out, null, 1));
ws.close();
