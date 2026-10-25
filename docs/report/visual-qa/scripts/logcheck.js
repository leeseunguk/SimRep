// B1 check: with the log drawer open the results panel must end at or above the drawer's top edge
// (no overlap), and the 3D view keeps its minimum height. Run after the plan's einzel setup.
const { useApp } = await import('/src/state/store.ts');
const { useResults } = await import('/src/results/store.ts');
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const rect = (el) => { const r = el.getBoundingClientRect(); return { top: Math.round(r.top), bottom: Math.round(r.bottom), h: Math.round(r.height) }; };
const measure = () => {
  const panel = document.querySelector('section[data-tour=results]');
  const drawer = document.querySelector('[class*=_drawer_]');
  const view = panel.parentElement.firstElementChild;
  const p = rect(panel), v = rect(view), d = drawer ? rect(drawer) : null;
  return { results: p, view: v, drawer: d, overlapPx: d ? Math.max(0, p.bottom - d.top) : 0 };
};
useResults.getState().setOpen(true);
useApp.getState().setLogOpen(false);
await sleep(400);
const closed = measure();
useApp.getState().setLogOpen(true);
await sleep(400);
const open = measure();
return { viewport: `${innerWidth}x${innerHeight}`, logClosed: closed, logOpen: open };
