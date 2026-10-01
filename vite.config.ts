import { defineConfig } from "vite";
import { resolve } from "node:path";

export default defineConfig({
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    host: "127.0.0.1",
    // Rust build output and the reference clone are not front-end sources.
    watch: { ignored: ["**/target/**", "**/src-tauri/**", "**/hook/**", "**/coucou-ref/**"] },
  },
  envPrefix: ["VITE_", "TAURI_ENV_"],
  build: {
    target: "chrome110",
    emptyOutDir: true,
    rollupOptions: {
      input: {
        island: resolve(__dirname, "index.html"),
        installer: resolve(__dirname, "installer.html"),
      },
    },
  },
});
