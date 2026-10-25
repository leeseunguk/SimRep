const { useApp } = await import('/src/state/store.ts'); const s = useApp.getState();
const o = s.objects.find(o=>o.kind==='pa' && o.visible); for (const l of __args.labels) s.toggleElectrode(o.id, l);
await new Promise(r=>setTimeout(r,500)); return useApp.getState().objects.find(x=>x.id===o.id).hiddenElectrodes ?? Object.keys(useApp.getState().objects.find(x=>x.id===o.id));