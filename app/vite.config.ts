import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// Tauri expects a fixed dev-server port (see src-tauri/tauri.conf.json -> build.devUrl).
// The server binds 127.0.0.1 so WSL2 localhost forwarding exposes it to the Windows-side
// Tauri webview without opening it to the LAN.
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: {
    host: "127.0.0.1",
    port: 1420,
    strictPort: true,
    watch: { ignored: ["**/src-tauri/**"] },
  },
  envPrefix: ["VITE_", "TAURI_ENV_"],
  build: {
    target: "es2022",
    sourcemap: true,
    rolldownOptions: {
      // React rarely changes: its own chunk keeps the app chunk small and cacheable.
      output: { codeSplitting: { groups: [{ name: "react", test: /node_modules[\\/](react|react-dom|scheduler)[\\/]/ }] } },
    },
  },
});
