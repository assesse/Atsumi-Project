import { defineConfig } from "vitest/config";
import react from "@vitejs/plugin-react";

const runtimeProcess = (globalThis as typeof globalThis & {
  process?: { env?: Record<string, string | undefined> };
}).process;
const environment = runtimeProcess?.env ?? {};
const host = environment.TAURI_DEV_HOST;

export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  // Native build artifacts contain tool documentation HTML, not app entries.
  // Include the isolated UI previews without scanning src-tauri/target.
  optimizeDeps: {
    entries: ["index.html", "tools/*-preview.html"],
  },
  server: {
    host: host || "127.0.0.1",
    port: 1420,
    strictPort: true,
    hmr: host
      ? {
          protocol: "ws",
          host,
          port: 1421,
        }
      : undefined,
    watch: {
      ignored: ["**/src-tauri/**"],
    },
  },
  build: {
    target: "es2021",
    minify: environment.TAURI_DEBUG ? false : "esbuild",
    sourcemap: Boolean(environment.TAURI_DEBUG),
  },
  test: {
    environment: "jsdom",
    setupFiles: "./src/test/setup.ts",
    css: true,
    globals: true,
  },
});
