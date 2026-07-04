import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// base "./" — собранная статика работает с любого пути (GitHub Pages, поддиректория).
// Порт: уважаем PORT из окружения (его назначает preview-харнесс при занятом 5173).
export default defineConfig({
  plugins: [react()],
  base: "./",
  server: {
    port: Number(process.env.PORT) || 5173,
  },
});
