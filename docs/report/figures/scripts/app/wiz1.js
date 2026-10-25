
const { useApp } = await import('/src/state/store.ts');
const project = await import('/src/state/project.ts');
const wiz = await import('/src/scene/importWizard.ts');
const { useWizard } = await import('/src/state/wizard.ts');
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const Q = __args.dir;
useApp.getState().setLang('ko'); useApp.getState().setThemePref('light');
useApp.setState({ dirty: false });
// start from an empty project so only the imported model is in the scene
for (const o of [...useApp.getState().objects]) useApp.getState().removeObject(o.id);
await wiz.openWizard({ path: Q + '\\quadrupole.gem', name: 'quadrupole.gem', size: 587 });
for (let i = 0; i < 400 && useWizard.getState().wizard?.status !== 'ready'; i++) await sleep(50);
const w0 = useWizard.getState().wizard;
const volts = { 1: 2.5, 2: -2.5, 3: 0 };
useWizard.getState().set({ method: 'cutcell', sidecar: true, parts: w0.parts.map((p) => ({ ...p, voltage: volts[p.number] ?? 0 })) });
useApp.setState({ toasts: [] });
useApp.getState().requestView({ type: 'preset', view: 'iso', fit: true });
await sleep(1200);
const w = useWizard.getState().wizard;
return { status: w.status, keys: Object.keys(w), parts: w.parts.map(p => ({ name: p.name, number: p.number, voltage: p.voltage })), method: w.method, sidecar: w.sidecar };
