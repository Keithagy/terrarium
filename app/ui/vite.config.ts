import { defineConfig } from "vite";

// Tauri expects a fixed port and no HMR overlay noise in the webview.
export default defineConfig({
  clearScreen: false,
  server: { port: 5173, strictPort: true },
  build: { target: "safari17", sourcemap: true, outDir: "dist", emptyOutDir: true },
});
