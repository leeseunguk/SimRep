import { invoke, isTauri } from "@tauri-apps/api/core";
import { computeDevice, reasonMessage } from "./scene/backend";
import { useApp } from "./state/store";

/** Mirrors `BackendInfo` in src-tauri/src/lib.rs. */
interface BackendInfo {
  appName: string;
  appVersion: string;
  coreCrate: string;
}

/** Detects the Tauri shell and checks the IPC round trip to the native core. */
export async function connectRuntime() {
  const app = useApp.getState();
  if (!isTauri()) {
    app.log("info", { key: "log.appStarted", params: { runtime: { key: "status.runtime.browser" } } });
    return;
  }
  app.setRuntime({ kind: "tauri" });
  // The native core creates the GPU flyer in the background at start-up; this resolves when it is ready.
  void computeDevice().then(
    (d) => {
      if (d.kind === "gpu") {
        app.setCompute({ state: "gpu", name: d.name, initMs: d.initMs });
        app.log("success", { key: "log.computeReady", params: { device: d.name, ms: Math.round(d.initMs) } });
      } else {
        const reason = d.reason ? reasonMessage(d.reason) : { key: "reason.noVulkan" as const };
        app.setCompute({ state: "cpu", name: d.name, reason });
        app.log("warning", { key: "log.computeCpu", params: { reason } });
      }
    },
    (err) => app.setCompute({ state: "cpu", name: "CPU", reason: { key: "error.technical", params: { detail: String(err) } } }),
  );
  try {
    const info = await invoke<BackendInfo>("backend_info");
    app.setRuntime({ kind: "tauri", version: info.appVersion });
    app.log("info", {
      key: "log.appStarted",
      params: { runtime: { key: "status.runtime.tauri", params: { version: info.appVersion } } },
    });
    app.log("success", {
      key: "log.backendInfo",
      params: { app: info.appName, version: info.appVersion, core: info.coreCrate },
    });
  } catch (err) {
    app.log("error", { key: "log.backendFailed", params: { reason: String(err) } });
  }
}
