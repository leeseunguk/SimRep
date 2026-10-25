
const { useApp } = await import('/src/state/store.ts');
const { useWizard } = await import('/src/state/wizard.ts');
const sim = await import('/src/scene/simulation.ts');
const { useResults } = await import('/src/results/store.ts');
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const a = __args; const Q = a.dir;
const o0 = useApp.getState().objects.find((o) => o.kind === 'pa' && o.visible);
const id = o0.id;
const pa = () => useApp.getState().objects.find((o) => o.id === id);
if (a.closeWizard) { useWizard.getState().close?.(); useWizard.setState?.({ wizard: null }); }
useApp.getState().updatePa(id, (o) => ({ source: { ...o.source, mode: 'file', file: { path: Q + '\\particles.ion', name: 'particles.ion' }, bench: { ...o.source.bench, enabled: true, min: [0, -18, -18], max: [60, 18, 18] }, drawCount: 1000, precision: a.precision, exactSurfaces: a.exact } }));
await sleep(300);
for (let i = 0; i < 600 && pa().flight.status === 'running'; i++) await sleep(50);
await sim.flyNow(id);
for (let i = 0; i < 1200 && pa().flight.status === 'running'; i++) await sleep(50);
useApp.getState().select(id);
if (a.tab) { useResults.getState().setTab(a.tab); useResults.getState().setHeight(a.height ?? 300); } else useResults.getState().setOpen(false);
if (a.view) useApp.getState().requestView({ type: 'preset', view: a.view, fit: true });
useApp.setState({ toasts: [] });
await sleep(1500);
const h = pa().flight.result.header;
return { ions: h.ions, drawn: h.drawn, device: h.device?.name, precision: h.precision, fallback: h.fallback, surface: h.surface, cuts: h.cuts, tof: h.tof, surfaces: h.surfaces.map(s => [s.surface, s.count, s.tofMean]), flyMs: h.flyMs };
