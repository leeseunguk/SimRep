import { defineConfig } from "vitest/config";

// Unit tests of the pure frontend logic (packets, analysis, CSV, plots, stores, i18n, option
// sync) in Node: no browser, GPU or native core. The e2e smoke test is Playwright (e2e/).
export default defineConfig({
  test: {
    include: ["src/**/*.test.ts"],
    environment: "node",
    setupFiles: ["src/test/setup.ts"],
  },
});
