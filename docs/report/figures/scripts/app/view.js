const { useApp } = await import('/src/state/store.ts');
const a = __args || {}; const s = useApp.getState();
const id = s.objects.find(o=>o.kind==='pa' && o.visible)?.id;
if (a.slice !== undefined && id) s.setSlice(id, { visible: a.slice });
if (a.view) s.requestView({ type: 'preset', view: a.view, fit: true });
if (a.camera) s.requestView({ type: 'camera', state: a.camera });
if (a.results === false) { const { useResults } = await import('/src/results/store.ts'); useResults.getState().setOpen(false); }
useApp.setState({ toasts: [] });
await new Promise(r=>setTimeout(r,1000));
const { cameraState } = await import('/src/viewport/viewState.ts');
return cameraState();