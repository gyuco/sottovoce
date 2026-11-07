import path from "node:path";
import process from "node:process";
import tailwindcss from "@tailwindcss/vite";
import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";

const host = process.env.TAURI_DEV_HOST;

// https://vite.dev/config/
export default defineConfig(() => ({
  plugins: [react(), tailwindcss()],
  resolve: {
    alias: { "@": path.resolve(__dirname, "./src") },
  },
  // Opzioni per Tauri (`tauri dev` / `tauri build`):
  // 1. non nascondere gli errori Rust
  clearScreen: false,
  // 2. Tauri si aspetta una porta fissa
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host ? { protocol: "ws", host, port: 1421 } : undefined,
    // 3. non osservare src-tauri
    watch: { ignored: ["**/src-tauri/**", "**/target/**", "**/crates/**"] },
  },
}));
