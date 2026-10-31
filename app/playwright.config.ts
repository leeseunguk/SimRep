import { defineConfig, devices } from "@playwright/test";

// End-to-end smoke test on demo mode (src/demo/ipc.ts): the real UI in headless Chromium against a
// mocked Tauri IPC layer serving recorded native-core packets. No GPU and no native window needed
// (without a WebGPU adapter the 3D view draws with its WebGL2 fallback, see e2e/webgl.spec.ts).
const PORT = 1430;

export default defineConfig({
  testDir: "e2e",
  timeout: 60_000,
  expect: { timeout: 15_000 },
  fullyParallel: false,
  reporter: [["list"]],
  use: {
    baseURL: `http://127.0.0.1:${PORT}`,
    locale: "en-US",
    viewport: { width: 1280, height: 800 },
    trace: "retain-on-failure",
  },
  projects: [{ name: "chromium", use: { ...devices["Desktop Chrome"], viewport: { width: 1280, height: 800 } } }],
  webServer: {
    command: `npx vite --mode demo --port ${PORT} --strictPort`,
    url: `http://127.0.0.1:${PORT}`,
    reuseExistingServer: false,
    timeout: 60_000,
  },
});
