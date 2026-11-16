import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// The built app is served by the Rust Axum backend (crates/arb-bot web_api).
// In development, /api is proxied to the locally running bot API.
export default defineConfig({
  plugins: [react()],
  server: {
    proxy: {
      "/api": {
        target: "http://127.0.0.1:8080",
        changeOrigin: true,
        ws: true,
      },
    },
  },
  build: {
    outDir: "dist",
    sourcemap: false,
  },
});
