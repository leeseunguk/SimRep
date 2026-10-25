
const { useApp } = await import('/src/state/store.ts'); const { useExport } = await import('/src/state/exportDialog.ts');
window.__h6 = { busySeen: false, logs0: useApp.getState().logs.length };
const id = setInterval(() => {
  const dlg = document.querySelector('[role=dialog][aria-modal=true]');
  if (!dlg || window.__h6.busySeen) return;
  const btns = [...dlg.querySelectorAll('button')];
  if (btns.at(-1)?.textContent.includes('내보내는 중')) {
    window.__h6.busySeen = true;
    window.__h6.closeEnabled = !dlg.querySelector('[aria-label="닫기"]').disabled && !btns.at(-2).disabled;
    window.__h6.cancelLabel = btns.at(-2).textContent.trim();
    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
    setTimeout(() => { window.__h6.closedWhileBusy = useExport.getState().objectId === null; }, 150);
  }
}, 1);
setTimeout(() => clearInterval(id), 120000);
return 'armed';
