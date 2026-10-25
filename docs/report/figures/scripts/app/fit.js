const { useApp } = await import('/src/state/store.ts');
useApp.setState({ toasts: [] });
useApp.getState().requestView({ type: 'fit' });
await new Promise(r=>setTimeout(r,1200));
const r = (sel) => { const e = document.querySelector(sel); if (!e) return null; const b = e.getBoundingClientRect(); return [b.x,b.y,b.width,b.height]; };
return { canvas: r('canvas'), main: r('main'), aside: [...document.querySelectorAll('aside')].map(a=>{const b=a.getBoundingClientRect(); return [b.x,b.y,b.width,b.height];}) };