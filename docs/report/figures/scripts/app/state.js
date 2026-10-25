const { useApp } = await import('/src/state/store.ts');
const pa = useApp.getState().objects.find((x) => x.kind === 'pa' && x.visible); const h = pa.flight.result.header;
return { voltages: pa.voltages, header: { ions: h.ions, device: h.device?.name, precision: h.precision, fallback: h.fallback, tof: h.tof, flyMs: h.flyMs, surfaces: h.surfaces.map(s=>[s.surface, s.count]) } };