import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// Tauri drives the dev server on a fixed port and expects it not to wander.
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    watch: {
      // Rust rebuilds are Cargo's job; watching these just causes spurious reloads.
      ignored: ["**/src-tauri/**", "**/target/**"],
    },
  },
  build: {
    // WKWebView on the oldest OS we support (macOS 12 / iOS 15).
    target: ["es2021", "safari15"],
    sourcemap: true,
  },
});
