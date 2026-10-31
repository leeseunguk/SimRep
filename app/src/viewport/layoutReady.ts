import { useApp } from "../state/store";
import type { RendererBase } from "./rendererBase";

/** A capture barrier: lazy results committed, resize delivered, then a frame at that size. */
export async function settleLayout(renderer: RendererBase, canvas: HTMLCanvasElement): Promise<void> {
  const section = canvas.parentElement!;
  section.dataset.layoutSettled = "false";
  const hasResults = useApp.getState().objects.some((o) => o.kind === "pa" && o.flight.result !== null);
  const phone = document.documentElement.dataset.layout === "phone";
  const resultsShown = !phone || document.querySelector('[data-testid="phone-tab-results"][aria-selected="true"]');
  if (hasResults && resultsShown) {
    await new Promise<void>((resolve, reject) => {
      const observer = new MutationObserver(check);
      const timeout = setTimeout(() => {
        observer.disconnect();
        reject(new Error("results panel did not mount before capture"));
      }, 5000);
      function check() {
        if (!document.querySelector('[data-tour="results"]')) return;
        clearTimeout(timeout);
        observer.disconnect();
        resolve();
      }
      observer.observe(document.body, { childList: true, subtree: true });
      check();
    });
  }
  await Promise.all(document.getAnimations()
    .filter((animation) => Number.isFinite(animation.effect?.getComputedTiming().endTime))
    .map((animation) => animation.finished));
  await new Promise<void>((resolve, reject) => {
    let removeFrame: (() => void) | undefined;
    const timeout = setTimeout(() => {
      observer.disconnect();
      removeFrame?.();
      reject(new Error("viewport did not resize and draw before capture"));
    }, 5000);
    // The viewport's resize observer was registered first. Initial delivery of this
    // observer therefore follows its resize, even if the canvas size did not change.
    const observer = new ResizeObserver(() => {
      observer.disconnect();
      removeFrame = renderer.addFrameListener(() => {
        clearTimeout(timeout);
        removeFrame?.();
        section.dataset.layoutSettled = "true";
        resolve();
      });
      renderer.requestRender();
    });
    observer.observe(canvas);
  });
}
