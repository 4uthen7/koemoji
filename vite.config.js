import { defineConfig } from "vite";

// Tauri 用の標準的な Vite 設定
export default defineConfig({
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    watch: {
      ignored: ["**/src-tauri/**"],
    },
  },
});
