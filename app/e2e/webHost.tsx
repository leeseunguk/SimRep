// A browser host like the SimRep-Web build (src/host.ts): the assistant is switched off and the
// status bar shows the host's own items. Registered before the app modules load, then the app starts
// (in demo mode, so templates, refine and fly still answer). polish.spec.ts loads it instead of
// src/main.tsx.
import { registerHost } from "../src/host";
import { refusals } from "../src/refusals";

registerHost({
  unavailable: (feature) => (feature === "ai" ? refusals.aiDesktopOnly() : null),
  StatusItems: () => (
    <>
      <span>CPU fallback · 4 threads</span>
      <span>Browser · WebAssembly</span>
    </>
  ),
});

await import("../src/main");
