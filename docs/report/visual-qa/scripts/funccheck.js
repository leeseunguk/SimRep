// Functional checks for the behaviour fixes (run in the app page with an einzel template open and selected):
// keyboard resizing of the side panel and the results panel, F1/Escape routing while the export dialog is open,
// and the camera jump under emulated prefers-reduced-motion (the driver sets the media feature).
const { useApp } = await import('/src/state/store.ts');
const { useExport } = await import('/src/state/exportDialog.ts');
const { useResults } = await import('/src/results/store.ts');
const { cameraState } = await import('/src/viewport/viewState.ts');
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const key = (el, k, extra = {}) => el.dispatchEvent(new KeyboardEvent('keydown', { key: k, bubbles: true, cancelable: true, ...extra }));
const out = {};

const pa = useApp.getState().objects.find((o) => o.kind === 'pa');
useApp.getState().select(pa.id);
useResults.getState().setOpen(true);
await sleep(300);

// Right panel resizer: ArrowLeft widens the right panel by 16 px (Shift: 64).
const sep = [...document.querySelectorAll('[role=separator][aria-orientation=vertical]')].at(-1);
const right = sep.parentElement;
const w0 = right.getBoundingClientRect().width;
sep.focus();
key(sep, 'ArrowLeft');
await sleep(150);
const w1 = right.getBoundingClientRect().width;
key(sep, 'ArrowRight', { shiftKey: true });
await sleep(150);
out.panel = { focusable: document.activeElement === sep, before: w0, afterLeft: w1, afterShiftRight: right.getBoundingClientRect().width };

// Results resizer: ArrowUp grows the panel by 16 px, Home restores the default.
const hs = document.querySelector('[role=separator][aria-orientation=horizontal]');
const panel = hs.parentElement;
const h0 = panel.getBoundingClientRect().height;
hs.focus();
key(hs, 'ArrowUp');
await sleep(150);
const h1 = panel.getBoundingClientRect().height;
key(hs, 'Home');
await sleep(150);
out.results = { focusable: document.activeElement === hs, before: h0, afterUp: h1, afterHome: panel.getBoundingClientRect().height, stored: useResults.getState().height };

// Export dialog open: F1 must not stack Help; one Escape closes only the export dialog, selection kept.
useExport.getState().open(pa.id);
await sleep(300);
key(document.body, 'F1');
await sleep(150);
const helpAfterF1 = useApp.getState().helpOpen;
key(document.body, 'Escape');
await sleep(200);
out.modal = { helpAfterF1, exportOpenAfterEscape: useExport.getState().objectId !== null, selectionKept: useApp.getState().selectedId === pa.id };

// Camera: under reduced motion a preset view is applied at once (no 320 ms fly-over).
const c0 = JSON.stringify(cameraState());
useApp.getState().requestView({ type: 'preset', view: 'top' });
await sleep(60);
const c1 = JSON.stringify(cameraState());
await sleep(500);
const c2 = JSON.stringify(cameraState());
out.camera = { reducedMotion: matchMedia('(prefers-reduced-motion: reduce)').matches, movedBy60ms: c1 !== c0, settledBy60ms: c1 === c2 };
useApp.getState().requestView({ type: 'preset', view: 'front' });
return out;
