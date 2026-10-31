// Installed production app: exercise its native-path drag/drop import handler.
const ready = new Promise((resolve, reject) => {
  const check = () => {
    if (document.querySelectorAll('input[aria-label="Voltage"]').length === 2) {
      observer.disconnect();
      clearTimeout(timer);
      resolve(document.body.innerText);
    }
  };
  const observer = new MutationObserver(check);
  observer.observe(document.body, { childList: true, subtree: true });
  const timer = setTimeout(() => {
    observer.disconnect();
    reject(new Error(document.body.innerText));
  }, 45000);
  check();
});
await window.__TAURI_INTERNALS__.invoke("plugin:event|emit_to", {
  target: { kind: "AnyLabel", label: "main" },
  event: "tauri://drag-drop",
  payload: {
    paths: [(window.SIMREP_CAD_FIXTURE ?? "C:\\simrep-cad-fixtures\\pair.step")],
    position: { x: 500, y: 400 },
  },
});
return await ready;
