import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// public/ holds the karlsen-wasm SDK and the prebuilt wallet (public/wallet/),
// both copied in by scripts/build-wallet.sh.
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: { port: 5174, strictPort: true },
  build: { target: "es2022", outDir: "dist", emptyOutDir: true },
});
