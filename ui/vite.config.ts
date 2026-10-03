import { defineConfig, type Plugin } from "vite";
import react from "@vitejs/plugin-react";
import fs from "node:fs";
import path from "node:path";

// public/ holds the karlsen-wasm SDK and the prebuilt wallet (public/wallet/),
// both copied in by scripts/build-wallet.sh.

// The Vite dev server refuses to serve JS files from public/ as ES modules
// ("This file is in /public ... should not be imported from source code").
// The SDK (karlsen.js) and the prebuilt wallet bundle are loaded exactly that way,
// so in dev we serve these prebuilt folders verbatim, ahead of Vite's transform
// pipeline. Production builds are unaffected: Tauri serves ui/dist directly.
const RAW_DIRS = ["/karlsen-wasm/", "/wallet/"];

const MIME: Record<string, string> = {
  ".js": "text/javascript",
  ".mjs": "text/javascript",
  ".wasm": "application/wasm",
  ".css": "text/css",
  ".html": "text/html",
  ".json": "application/json",
  ".webmanifest": "application/manifest+json",
  ".svg": "image/svg+xml",
  ".png": "image/png",
  ".ico": "image/x-icon",
  ".ts": "text/plain",
};

function servePrebuiltPublic(): Plugin {
  return {
    name: "serve-prebuilt-public",
    apply: "serve",
    configureServer(server) {
      const publicDir = path.resolve(server.config.publicDir);
      server.middlewares.use((req, res, next) => {
        const urlPath = decodeURIComponent((req.url ?? "").split("?")[0]);
        if (!RAW_DIRS.some((d) => urlPath.startsWith(d) || urlPath === d.slice(0, -1))) {
          return next();
        }
        let file = path.join(publicDir, urlPath);
        // Stay inside public/ (no ../ escapes).
        if (!file.startsWith(publicDir)) return next();
        if (fs.existsSync(file) && fs.statSync(file).isDirectory()) {
          file = path.join(file, "index.html");
        }
        // SPA fallback for the wallet's client-side routes.
        if (!fs.existsSync(file) && urlPath.startsWith("/wallet/") && !path.extname(urlPath)) {
          file = path.join(publicDir, "wallet", "index.html");
        }
        if (!fs.existsSync(file)) return next();
        res.setHeader("Content-Type", MIME[path.extname(file).toLowerCase()] ?? "application/octet-stream");
        res.setHeader("Cache-Control", "no-cache");
        fs.createReadStream(file).pipe(res);
      });
    },
  };
}

export default defineConfig({
  plugins: [servePrebuiltPublic(), react()],
  clearScreen: false,
  server: { port: 5174, strictPort: true },
  build: { target: "es2022", outDir: "dist", emptyOutDir: true },
});
