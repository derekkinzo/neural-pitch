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
  },
  envPrefix: ["VITE_", "TAURI_ENV_"],
  build: {
    target: "esnext",
    sourcemap: !!process.env.TAURI_ENV_DEBUG,
  },
});
