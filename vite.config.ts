import { defineConfig } from "vitest/config";
import react from "@vitejs/plugin-react";
import { readFile } from "node:fs/promises";

const runtimeProcess = (globalThis as typeof globalThis & {
  process?: { env?: Record<string, string | undefined> };
}).process;
const environment = runtimeProcess?.env ?? {};
const host = environment.TAURI_DEV_HOST;

export default defineConfig({
  plugins: [react(), {
    name: "isolated-original-player-assets",
    configureServer(server) {
      // Only this directory's public assets are readable by the opaque player frame.
      server.middlewares.use((request, response, next) => {
        const path = request.url?.split("?")[0] ?? "";
        // One generated, non-user fixture for opaque-frame visual regression.
        // Leave real recordings, source files and all other routes unchanged.
        if (path === "/.runtime/chzzk-original-player/synthetic.mp4" && request.headers.origin === "null") {
          response.setHeader("Access-Control-Allow-Origin", "null");
          return next();
        }
        if (!/^\/original-player\/(?:assets\/)?[a-zA-Z0-9_.-]+\.(html|js|css|png|woff2)$/.test(path)) return next();
        void readFile(new URL(`./public${path}`, import.meta.url)).then(bytes => {
          response.setHeader("Access-Control-Allow-Origin", "*");
          response.setHeader("Content-Type", path.endsWith(".js") ? "text/javascript" : path.endsWith(".css") ? "text/css" : path.endsWith(".png") ? "image/png" : path.endsWith(".woff2") ? "font/woff2" : "text/html");
          response.setHeader("Cache-Control", "no-store"); response.end(bytes);
        }).catch(() => { response.statusCode = 404; response.end(); });
      });
    },
  }],
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
    // Runtime audit snapshots are not another copy of the test suite.
    include: ["src/**/*.{test,spec}.{ts,tsx}"],
    environment: "jsdom",
    setupFiles: "./src/test/setup.ts",
    css: true,
    globals: true,
  },
});
