const button = (text) => [...document.querySelectorAll("button")].find((b) => b.innerText === text);
const waitFor = (predicate, timeoutMs = 90000) => new Promise((resolve, reject) => {
  const check = () => {
    if (!predicate()) return;
    observer.disconnect();
    clearTimeout(timer);
    resolve();
  };
  const observer = new MutationObserver(check);
  observer.observe(document.body, { childList: true, subtree: true, attributes: true });
  const timer = setTimeout(() => {
    observer.disconnect();
    reject(new Error(document.body.innerText));
  }, timeoutMs);
  check();
});
const partsReady = waitFor(() => document.querySelectorAll('input[aria-label="Voltage"]').length === 2);
await window.__TAURI_INTERNALS__.invoke("plugin:event|emit_to", {
  target: { kind: "AnyLabel", label: "main" },
  event: "tauri://drag-drop",
  payload: {
    paths: [(window.SIMREP_CAD_FIXTURE ?? "C:\\simrep-cad-fixtures\\pair.step")],
    position: { x: 500, y: 400 },
  },
});
await partsReady;
const parts = document.body.innerText;
const voltage = document.querySelector('input[aria-label="Voltage"]');
voltage.focus();
voltage.value = "100";
voltage.blur();
const gridReady = waitFor(() => document.body.innerText.includes("BOUNDARY METHOD"));
button("Next").click();
await gridReady;
const grid = document.body.innerText;
const outputReady = waitFor(() => document.querySelector('input[aria-label="File name"]'));
button("Next").click();
await outputReady;
const name = document.querySelector('input[aria-label="File name"]');
Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value").set.call(name, "rtx3070-smoke-d293d03");
name.dispatchEvent(new Event("input", { bubbles: true }));
await new Promise(requestAnimationFrame);
const refined = waitFor(() => button("Done"));
button("Build & refine").click();
await refined;
const refinement = document.body.innerText;
const flown = waitFor(() => button("Fly again"));
button("Fly ions").click();
await flown;
const finished = waitFor(() => !document.querySelector('aside[aria-label="Import geometry"]'));
button("Done").click();
await finished;
return { parts, grid, refinement, afterFlight: document.body.innerText };
