import { defineConfig } from "vite";

// Tauri expects a fixed port and no HMR overlay noise in the webview.
export default defineConfig({
  clearScreen: false,
  server: { port: 5173, strictPort: true },
  // three.js is one large chunk by design; the app loads it from disk, not a network.
  build: { target: "safari17", sourcemap: true, outDir: "dist", emptyOutDir: true, chunkSizeWarningLimit: 1500 },
});
