// Surfaces captured by vqa.mjs. Each setup/prep is the body of an async function run in the app
// page after PRELUDE. State carries over between surfaces in this order.
export const PRELUDE = String.raw`
const { useApp } = await import('/src/state/store.ts');
const project = await import('/src/state/project.ts');
const { useResults } = await import('/src/results/store.ts');
const { useWizard } = await import('/src/state/wizard.ts');
const { useTour } = await import('/src/state/tour.ts');
const { useExport } = await import('/src/state/exportDialog.ts');
const sim = await import('/src/scene/simulation.ts');
const wiz = await import('/src/scene/importWizard.ts');
const files = await import('/src/scene/importFiles.ts');
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const W = globalThis.SIMREP_VISUAL_QA_DIR ?? 'C:\\simrep-gui-test\\visual-qa';
const own = (e) => [...e.childNodes].filter((n) => n.nodeType === 3).map((n) => n.textContent).join('').trim();
const byText = (text, sel = '*') => [...document.querySelectorAll(sel)].find((e) => own(e) === text) ?? [...document.querySelectorAll(sel)].find((e) => own(e).startsWith(text));
const scroller = (el) => { let p = el?.parentElement; while (p && !(p.scrollHeight > p.clientHeight && /auto|scroll/.test(getComputedStyle(p).overflowY))) p = p.parentElement; return p; };
const scrollToEl = (el, pad = 8) => { const p = scroller(el); if (p) p.scrollTop += el.getBoundingClientRect().top - p.getBoundingClientRect().top - pad; return !!p; };
const pa = () => useApp.getState().objects.filter((x) => x.kind === 'pa' && x.visible).at(-1);
const until = async (f, n = 600, ms = 50) => { for (let i = 0; i < n; i++) { if (f()) return true; await sleep(ms); } return false; };
const reset = async () => {
  useWizard.setState({ wizard: null });
  useTour.setState({ mode: 'off' });
  localStorage.setItem('simrep.tour', 'seen');
  useExport.getState().close();
  useApp.getState().setLang('ko');
  useApp.setState({ dirty: false, toasts: [], helpOpen: false, logOpen: false, busy: null });
};
const clearScene = () => { for (const o of [...useApp.getState().objects]) useApp.getState().removeObject(o.id); };
const summary = () => { const o = pa(); const h = o?.flight?.result?.header; return o && { name: o.fileName, path: o.path, template: o.template, dims: [o.info.nx, o.info.ny, o.info.nz], voltages: o.voltages, ions: h?.ions, drawn: h?.drawn, device: h?.device?.name, precision: h?.precision, tof: h?.tof, surfaces: h?.surfaces?.map((s) => [s.surface, s.count]) }; };
const fresh = async (tpl) => {
  await reset();
  await project.newFromTemplate(tpl);
  await until(() => pa()?.flight?.result);
  useApp.getState().select(pa().id);
  useResults.getState().selectIon(null);
  useResults.getState().setHeight(null);
  useResults.getState().setTab('hist');
  await sleep(300);
  useApp.setState({ toasts: [] });
  useApp.getState().requestView({ type: 'fit' });
  await sleep(900);
  return summary();
};
const ensure = async (tpl) => {
  const o = pa();
  if (!o || o.template !== tpl || useApp.getState().home) return fresh(tpl);
  await reset();
  useApp.getState().select(o.id);
  return summary();
};
const restoreToasts = () => { if (window.__toasts) useApp.setState({ toasts: window.__toasts }); };
// Same focus voltage on every build (the before/after pairs must show the same field and flight).
const setV = async (v) => { const o = pa(); if (o.voltages[2] === v) return; const f0 = o.flight.result; sim.setElectrodeVoltage(o.id, 2, v); await until(() => pa().flight.result !== f0 && pa().flight.status !== 'running'); await sleep(400); };
`;

const tpl = (id, title) => ({ id: `template-${id}`, title, setup: `return await fresh('${id}');` });

export const surfaces = [
  {
    id: "start",
    title: "Start screen: templates and open file (no recent projects)",
    setup: `await reset(); clearScene(); useApp.setState({ recent: [], recovery: null }); useApp.getState().setHome(true); await sleep(600); return { home: true };`,
  },
  {
    id: "start-recent",
    title: "Start screen with recent projects and a recovery offer",
    setup: `await reset(); const now = Date.now();
      useApp.setState({ recent: [
        { path: W + '\\\\projects\\\\einzel-focus.simrep', name: 'einzel-focus', savedAt: now - 3600e3 },
        { path: W + '\\\\projects\\\\quad-scan.simrep', name: 'quad-scan', savedAt: now - 2 * 86400e3 },
        { path: W + '\\\\projects\\\\tof-mirror-v2.simrep', name: 'tof-mirror-v2', savedAt: now - 9 * 86400e3 } ],
        recovery: { meta: { name: 'quad-scan', nameKey: null, path: W + '\\\\projects\\\\quad-scan.simrep', dir: null }, savedAt: now - 600e3 } });
      useApp.getState().setHome(true); await sleep(600); return { recent: 3, recovery: true };`,
  },
  {
    id: "start-tour-offer",
    title: "First-launch tour offer card on the start screen",
    setup: `await reset(); useApp.setState({ recent: [], recovery: null }); useApp.getState().setHome(true); useTour.setState({ mode: 'offer' }); await sleep(600); return { tour: 'offer' };`,
  },
  {
    id: "busy",
    title: "Busy overlay while a template is created",
    setup: `await reset(); useApp.getState().setHome(true); useApp.getState().setBusy({ key: 'busy.template', params: { name: '아인젤 렌즈' } }); await sleep(600); return { busy: true };`,
  },
  {
    id: "empty-workspace",
    title: "Empty workspace (no objects, start screen closed)",
    setup: `await reset(); clearScene(); useApp.getState().setHome(false); await sleep(600); return { objects: 0 };`,
  },
  tpl("einzel", "Einzel lens template after refine + fly, results (TOF histogram)"),
  tpl("quadrupole", "Quadrupole template after refine + fly, results (TOF histogram)"),
  tpl("reflectron", "Reflectron TOF template after refine + fly, results (TOF histogram)"),
  tpl("trap", "Ion trap template after refine + fly, results (TOF histogram)"),
  {
    id: "props-ions",
    title: "Einzel: properties panel scrolled to the ion source section",
    setup: `return await ensure('einzel');`,
    prep: `const h = [...document.querySelectorAll('aside h2, aside h3, aside button')].find((e) => e.textContent.trim().startsWith('이온')); return { found: !!h && scrollToEl(h) };`,
  },
  {
    id: "props-flight-results",
    title: "Einzel: properties panel scrolled to the flight results section (TOF stats, hits)",
    setup: `return await ensure('einzel');`,
    prep: `const h = byText('비행 결과', 'aside *'); return { found: !!h && scrollToEl(h) };`,
  },
  {
    id: "props-end",
    title: "Einzel: properties panel scrolled to the end (flight results section)",
    setup: `return await ensure('einzel');`,
    prep: `const a = [...document.querySelectorAll('aside')].at(-1); const p = [a, ...a.querySelectorAll('*')].find((e) => e.scrollHeight > e.clientHeight && /auto|scroll/.test(getComputedStyle(e).overflowY)); if (p) p.scrollTop = p.scrollHeight; return { scrolled: !!p };`,
  },
  {
    id: "electrode-selected",
    title: "Einzel: electrode #2 selected in the scene tree (electrode properties)",
    setup: `await ensure('einzel'); useApp.getState().select(pa().id, 2); await sleep(500); return { electrode: 2 };`,
  },
  {
    id: "live-slider",
    title: "Einzel: dragging the #2 voltage slider (live re-fly during the drag)",
    setup: `await ensure('einzel'); useApp.getState().select(pa().id); return summary();`,
    drag: true,
    prep: `sim.setElectrodeVoltage(pa().id, 2, 80); await sleep(700);
      const box = document.querySelector('[data-tour="voltage-2"]'); scrollToEl(box, 40); await sleep(200);
      const r = box.querySelector('input[type=range]'); const b = r.getBoundingClientRect();
      const f = (Number(r.value) - Number(r.min)) / (Number(r.max) - Number(r.min));
      const x = b.left + 8 + f * (b.width - 16); const y = b.top + b.height / 2;
      const to = b.left + 8 + ((40 - Number(r.min)) / (Number(r.max) - Number(r.min))) * (b.width - 16);
      return { drag: [x, y, to, y], value: r.value, min: r.min, max: r.max, target: 40 };`,
  },
  {
    id: "results-map",
    title: "Results: detector map tab",
    setup: `await ensure('einzel'); await setV(40); useResults.getState().setTab('map'); await sleep(600); return summary();`,
  },
  {
    id: "results-ion",
    title: "Results: ion tab with ion #5 selected (time series)",
    setup: `await ensure('einzel'); await setV(40); useResults.getState().selectIon({ objectId: pa().id, index: 5 }); useResults.getState().setTab('ion'); await until(() => useResults.getState().trace, 200); await sleep(600); return { trace: !!useResults.getState().trace };`,
  },
  {
    id: "results-table",
    title: "Results: end-position table",
    setup: `await ensure('einzel'); await setV(40); useResults.getState().selectIon(null); useResults.getState().setTab('table'); await sleep(600); return summary();`,
  },
  {
    id: "results-compare",
    title: "Results: compare tab with one kept run and a changed voltage",
    setup: `await ensure('einzel'); await setV(40); useResults.getState().setTab('compare'); await sleep(500);
      if (!useResults.getState().snapshots.length) { byText('이 결과 보관', 'button')?.click(); await sleep(300);
        const f0 = pa().flight.result; sim.setElectrodeVoltage(pa().id, 2, 60); await until(() => pa().flight.result !== f0 && pa().flight.status !== 'running'); }
      await sleep(800); return { snapshots: useResults.getState().snapshots.length, voltages: pa().voltages };`,
  },
  {
    id: "results-export-menu",
    title: "Results: export menu open",
    setup: `await ensure('einzel'); useResults.getState().setTab('hist'); await sleep(400); return summary();`,
    prep: `const w = document.querySelector('[data-tour="results-export"]'); if (!w.querySelector('[role=menu]')) w.querySelector('button').click(); await sleep(300); return { open: !!w.querySelector('[role=menu]') };`,
  },
  {
    id: "help",
    title: "Help overlay (shortcuts, start tour)",
    setup: `await ensure('einzel'); useApp.getState().setHelpOpen(true); await sleep(500); return { help: true };`,
  },
  {
    id: "log-drawer",
    title: "Log drawer open",
    setup: `await ensure('einzel'); useApp.getState().setLogOpen(true); await sleep(500); return { logs: useApp.getState().logs.length };`,
  },
  {
    id: "export-dialog",
    title: "Export dialog (geometry tab)",
    setup: `await ensure('einzel'); useExport.getState().open(pa().id, 'geometry'); await sleep(600); return { export: true };`,
  },
  {
    id: "toasts",
    title: "Toasts after creating a template",
    setup: `await reset(); await project.newFromTemplate('einzel'); await until(() => pa()?.flight?.result); await sleep(500); window.__toasts = useApp.getState().toasts; useApp.getState().select(pa().id); return { toasts: window.__toasts.map((t) => t.title) };`,
    prep: `restoreToasts(); await sleep(200); return { n: useApp.getState().toasts.length };`,
  },
  {
    id: "error-unsupported-file",
    title: "Error: opening an unsupported file (toast)",
    setup: `await reset(); await files.importPaths([W + '\\\\bad\\\\notes.txt']); await sleep(600); window.__toasts = useApp.getState().toasts; return { toasts: window.__toasts.map((t) => [t.tone, t.title, t.body]) };`,
    prep: `restoreToasts(); await sleep(200); return { n: useApp.getState().toasts.length };`,
  },
  {
    id: "refine-progress",
    title: "Refine in progress (properties refine section + viewport chip)",
    perShot: true,
    wait: 0,
    setup: `await until(() => !useApp.getState().objects.some((o) => o.kind === 'pa' && (o.refine?.status === 'running' || o.refine?.status === 'starting')), 1200);
      await reset(); await project.newFromFiles([W + '\\\\refine\\\\quadrupole.pa#']); await until(() => pa()?.formatId === 'pa#' && pa().grid);
      const id = pa().id; useApp.getState().select(id); useResults.getState().setOpen(false); await sleep(400);
      void sim.startRefine(id, 'cpu');
      await until(() => { const r = useApp.getState().objects.find((o) => o.id === id)?.refine; return r && r.status === 'running' && r.history.length >= 4; }, 1000, 10);
      const sec = byText('Refine', 'aside h2, aside h3, aside button') ?? document.querySelector('[data-refine]'); const r = useApp.getState().objects.find((o) => o.id === id).refine;
      return { status: r.status, stage: r.stage, cycle: r.cycle, device: r.device?.name };`,
  },
  {
    id: "exact-on",
    title: "Exact surface card ON (quadrupole cut-cell .pa0 with .cuts.json) + results badge",
    setup: `await until(() => !useApp.getState().objects.some((o) => o.kind === 'pa' && (o.refine?.status === 'running' || o.refine?.status === 'starting')), 1200);
      await reset(); await project.newFromFiles([W + '\\\\quad\\\\quadrupole.pa0']); await until(() => pa()?.formatId === 'pa0' && pa().grid);
      const id = pa().id;
      useApp.getState().updatePa(id, (o) => ({ source: { ...o.source, mode: 'file', file: { path: W + '\\\\quad\\\\particles.ion', name: 'particles.ion' }, bench: { ...o.source.bench, enabled: true, min: [0, -18, -18], max: [60, 18, 18] }, drawCount: 300, precision: 'live', exactSurfaces: true } }));
      await sleep(300); await until(() => pa().flight.status !== 'running'); await sim.flyNow(id); await until(() => pa().flight.status !== 'running', 1200);
      useApp.getState().select(id); useResults.getState().setTab('hist'); useApp.setState({ toasts: [] }); useApp.getState().requestView({ type: 'preset', view: 'iso', fit: true }); await sleep(900); return summary();`,
    prep: `const el = byText('정확한 표면', 'aside *'); return { found: !!el && scrollToEl(el, 60) };`,
  },
  {
    id: "exact-off",
    title: "Exact surface card OFF (grid-point collision, GPU live)",
    setup: `await reset(); const id = pa().id; useApp.getState().updatePa(id, (o) => ({ source: { ...o.source, exactSurfaces: false } })); await sleep(300); await until(() => pa().flight.status !== 'running'); await sim.flyNow(id); await until(() => pa().flight.status !== 'running', 1200); useApp.setState({ toasts: [] }); await sleep(600); return summary();`,
    prep: `const el = byText('정확한 표면', 'aside *'); return { found: !!el && scrollToEl(el, 60) };`,
  },
  {
    id: "wizard-gem-parts",
    title: "Import wizard (GEM): step 1 parts",
    setup: `await reset(); clearScene(); useApp.getState().setHome(false); useResults.getState().setOpen(false);
      await wiz.openWizard({ path: W + '\\\\wiz\\\\quadrupole.gem', name: 'quadrupole.gem', size: 587 }); await until(() => useWizard.getState().wizard?.status === 'ready');
      const w0 = useWizard.getState().wizard; const v = { 1: 2.5, 2: -2.5, 3: 0 }; useWizard.getState().set({ parts: w0.parts.map((p) => ({ ...p, voltage: v[p.number] ?? 0 })) });
      useApp.getState().requestView({ type: 'preset', view: 'iso', fit: true }); await sleep(900); const w = useWizard.getState().wizard; return { status: w.status, parts: w.parts.length, method: w.method };`,
  },
  {
    id: "wizard-gem-grid",
    title: "Import wizard (GEM): step 2 grid",
    setup: `useWizard.getState().set({ step: 'grid' }); await sleep(600); const w = useWizard.getState().wizard; return { method: w.method, spacing: w.spacing };`,
  },
  {
    id: "wizard-gem-refine",
    title: "Import wizard (GEM): step 3 output + refine (before running)",
    setup: `useWizard.getState().set({ step: 'refine' }); await sleep(600); return { step: 'refine' };`,
  },
  {
    id: "wizard-gem-running",
    title: "Import wizard (GEM): build + refine in progress",
    perShot: true,
    wait: 0,
    setup: `await until(() => !['building', 'refining'].includes(useWizard.getState().wizard?.status));
      await reset(); clearScene(); useApp.getState().setHome(false); useResults.getState().setOpen(false);
      await wiz.openWizard({ path: W + '\\\\wiz\\\\quadrupole.gem', name: 'quadrupole.gem', size: 587 }); await until(() => useWizard.getState().wizard?.status === 'ready');
      const w0 = useWizard.getState().wizard; const v = { 1: 2.5, 2: -2.5, 3: 0 };
      useWizard.getState().set({ method: 'cutcell', sidecar: true, device: 'cpu', step: 'refine', parts: w0.parts.map((p) => ({ ...p, voltage: v[p.number] ?? 0 })) }); await sleep(300);
      window.__run = wiz.runWizard();
      await until(() => { const w = useWizard.getState().wizard; const o = w?.objectId && useApp.getState().objects.find((x) => x.id === w.objectId); return w?.status === 'refining' && o?.refine?.history?.length >= 3; }, 1500, 10);
      return { status: useWizard.getState().wizard.status };`,
  },
  {
    id: "wizard-gem-done",
    title: "Import wizard (GEM): done, ready to fly",
    setup: `await window.__run; await until(() => useWizard.getState().wizard?.status === 'done'); useApp.setState({ toasts: [] }); await sleep(800); const w = useWizard.getState().wizard; return { status: w.status, build: w.build && Object.keys(w.build) };`,
  },
  {
    id: "wizard-step-parts",
    title: "Import wizard (STEP pair.step): step 1 parts (CAD B-rep check)",
    setup: `await until(() => !['building', 'refining'].includes(useWizard.getState().wizard?.status));
      await reset(); clearScene(); useApp.getState().setHome(false); useResults.getState().setOpen(false);
      await wiz.openWizard({ path: W + '\\\\step\\\\pair.step', name: 'pair.step', size: 4124 }); await until(() => useWizard.getState().wizard?.status === 'ready');
      const w0 = useWizard.getState().wizard; useWizard.getState().set({ parts: w0.parts.map((p, i) => ({ ...p, voltage: i === 0 ? 100 : 0 })) });
      useApp.getState().requestView({ type: 'preset', view: 'iso', fit: true }); useApp.setState({ toasts: [] }); await sleep(900); const w = useWizard.getState().wizard; return { status: w.status, error: w.error, parts: w.parts.map((p) => p.name) };`,
  },
  {
    id: "wizard-step-grid",
    title: "Import wizard (STEP): step 2 grid (cut-cell, CAD surface)",
    setup: `useWizard.getState().set({ step: 'grid' }); await sleep(600); const w = useWizard.getState().wizard; return { method: w.method, cadSurface: w.cadSurface };`,
  },
  {
    id: "error-import-gem",
    title: "Error: import wizard on a broken GEM file",
    setup: `await reset(); clearScene(); useApp.getState().setHome(false);
      await wiz.openWizard({ path: W + '\\\\bad\\\\broken.gem', name: 'broken.gem', size: 60 }); await until(() => ['error', 'ready'].includes(useWizard.getState().wizard?.status));
      await sleep(600); window.__toasts = useApp.getState().toasts; const w = useWizard.getState().wizard; return { status: w.status, error: w.error };`,
    prep: `restoreToasts(); await sleep(200); return null;`,
  },
  {
    id: "error-import-step",
    title: "Error: import wizard on a broken STEP file",
    setup: `await reset(); clearScene(); useApp.getState().setHome(false);
      await wiz.openWizard({ path: W + '\\\\bad\\\\broken.step', name: 'broken.step', size: 50 }); await until(() => ['error', 'ready'].includes(useWizard.getState().wizard?.status));
      await sleep(600); window.__toasts = useApp.getState().toasts; const w = useWizard.getState().wizard; return { status: w.status, error: w.error };`,
    prep: `restoreToasts(); await sleep(200); return null;`,
  },
  {
    id: "tour-1",
    title: "Tour step 1/5: pick the einzel template (start screen)",
    setup: `await reset(); clearScene(); useApp.setState({ recent: [], recovery: null }); useTour.getState().start(); await sleep(900); return { step: useTour.getState().step };`,
  },
  {
    id: "tour-2",
    title: "Tour step 2/5: change the focus voltage",
    setup: `useApp.setState({ dirty: false }); await project.newFromTemplate('einzel'); await until(() => pa()?.flight?.result); await until(() => useTour.getState().step === 1, 200); useResults.getState().setOpen(false); useApp.setState({ toasts: [] }); await sleep(1200); return { step: useTour.getState().step };`,
  },
  {
    id: "tour-3",
    title: "Tour step 3/5: fly",
    setup: `useTour.getState().go(2); useApp.setState({ toasts: [] }); await sleep(900); return { step: useTour.getState().step };`,
  },
  {
    id: "tour-4",
    title: "Tour step 4/5: TOF histogram",
    setup: `useTour.getState().go(3); useResults.getState().setOpen(false); await sleep(900); return { step: useTour.getState().step };`,
  },
  {
    id: "tour-5",
    title: "Tour step 5/5: export CSV",
    setup: `useResults.getState().setTab('hist'); useTour.getState().go(4); await sleep(900); return { step: useTour.getState().step };`,
  },
  {
    id: "tour-done",
    title: "Tour finished card",
    setup: `useTour.getState().go(5); await sleep(900); return { step: useTour.getState().step };`,
  },
];
