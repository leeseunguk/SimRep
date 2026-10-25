import { installDemo } from "./ipc";

// Imported first by main.tsx: modules that check isTauri() when they load (scene/simulation.ts,
// results/trace.ts) must see the mocked desktop. Production builds drop this (the flag is unset).
if (import.meta.env.VITE_SIMREP_DEMO === "1") installDemo();
