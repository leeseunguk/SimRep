
const { useApp } = await import('/src/state/store.ts');
const wiz = await import('/src/scene/importWizard.ts');
const { useWizard } = await import('/src/state/wizard.ts');
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
useWizard.getState().set({ step: 'refine' });
await sleep(300);
const t0 = performance.now();
await wiz.runWizard();
const ms = performance.now() - t0;
const w = useWizard.getState().wizard;
const id = w.objectId;
const pa = () => useApp.getState().objects.find((o) => o.id === id);
for (let i = 0; i < 200 && pa()?.formatId !== 'pa0'; i++) await sleep(50);
useApp.setState({ toasts: [] });
await sleep(800);
const o = pa();
return { ms, status: w.status, step: w.step, build: w.build, pa: o && { path: o.path, format: o.formatId, cuts: o.info.cuts, refine: o.refine && { status: o.refine.status, device: o.refine.device, cycles: o.refine.cycles, residual: o.refine.residual, stages: o.refine.stages, elapsedMs: o.refine.elapsedMs } } };
