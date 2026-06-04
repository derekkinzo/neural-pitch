import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import path from "node:path";

// https://v2.tauri.app/start/migrate/from-tauri-1/#frontend-dist
export default defineConfig({
  plugins: [react()],
  resolve: {
    alias: {
      "@": path.resolve(__dirname, "./src"),
    },
  },
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    host: false,
    watch: {
      // The Rust target/ directory contains 50k+ build artifacts that would
      // otherwise blow past the system inotify watcher limit (default 65k on
      // most Linux). Excluding it (and other Rust build outputs) keeps the
      // dev server boot below ENOSPC.
      ignored: ["**/target/**", "**/src-tauri/target/**", "**/node_modules/**", "**/.git/**"],
    },
  },
  envPrefix: ["VITE_", "TAURI_ENV_"],
  build: {
    target: "esnext",
    sourcemap: !!process.env.TAURI_ENV_DEBUG,
  },
});
