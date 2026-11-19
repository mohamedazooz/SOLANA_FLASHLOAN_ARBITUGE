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
        configure: (proxy, _options) => {
          proxy.on("error", (err, _req, _res) => {
            // Suppress unhandled ECONNRESET / socket hang-up on client disconnect/refresh
            if (err.message && (err.message.includes("ECONNRESET") || err.message.includes("socket hang up"))) {
              return;
            }
            console.warn("[vite proxy warning]", err.message);
          });
        },
      },
    },
  },
  build: {
    outDir: "dist",
    sourcemap: false,
  },
});
