const { useResults } = await import('/src/results/store.ts');
const a=__args; useResults.getState().setTab(a.tab); useResults.getState().setHeight(a.height);
if (a.hist) useResults.setState(s => ({ hist: { ...s.hist, ...a.hist } }));
await new Promise(r=>setTimeout(r,900)); return useResults.getState().hist;