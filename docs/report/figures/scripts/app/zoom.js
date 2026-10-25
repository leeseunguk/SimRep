const a = __args; const c = document.querySelector('canvas');
for (let i = 0; i < a.steps; i++) { c.dispatchEvent(new WheelEvent('wheel', { deltaY: a.delta, clientX: a.x, clientY: a.y, bubbles: true, cancelable: true })); await new Promise(r=>setTimeout(r,40)); }
await new Promise(r=>setTimeout(r,600)); return 'ok';