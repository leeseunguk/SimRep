
const { useApp } = await import('/src/state/store.ts');
const project = await import('/src/state/project.ts');
const { useResults } = await import('/src/results/store.ts');
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const s = useApp.getState();
s.setLang('ko'); s.setThemePref('light');
useApp.setState({ dirty: false });
localStorage.setItem('simrep.tour', JSON.stringify({ seen: true }));
await project.newFromTemplate('einzel');
const pa = () => useApp.getState().objects.find((x) => x.kind === 'pa' && x.visible);
for (let i = 0; i < 600 && !pa()?.flight?.result; i++) await sleep(50);
const a = __args || {};
if (a.count) {
  useApp.getState().updatePa(pa().id, (p) => ({ source: { ...p.source, count: a.count, drawCount: a.draw ?? p.source.drawCount, precision: a.precision ?? p.source.precision } }));
  await sleep(400);
  const sim = await import('/src/scene/simulation.ts');
  for (let k = 0; k < 20; k++) { await sim.flyNow(pa().id); const h = pa().flight.result.header; if (!h.fallback && h.ions === a.count) break; await sleep(1000); }
}
useApp.getState().select(pa().id);
if (a.tab) { useResults.getState().setTab(a.tab); if (a.height) useResults.getState().setHeight(a.height); } else useResults.getState().setOpen(false);
useApp.setState({ toasts: [] });
useApp.getState().requestView({ type: 'fit' });
await sleep(1500);
const f = pa().flight.result; const h = f.header;
return { name: pa().name, path: pa().path, info: { nx: pa().info.nx, ny: pa().info.ny, nz: pa().info.nz }, voltages: pa().voltages, source: { count: pa().source.count, mode: pa().source.mode, precision: pa().source.precision, drawCount: pa().source.drawCount }, header: { ions: h.ions, drawn: h.drawn, device: h.device?.name, precision: h.precision, fallback: h.fallback, tof: h.tof, surfaces: h.surfaces, flyMs: h.flyMs } };
