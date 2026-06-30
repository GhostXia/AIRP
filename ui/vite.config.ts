import { defineConfig } from "vitest/config";
import vue from "@vitejs/plugin-vue";

// AIRP UI dev server + vitest config. Port is fixed so the Tauri shell can
// point at it.
export default defineConfig({
  plugins: [vue()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
  },
  test: {
    environment: "node",
    include: ["src/**/*.test.ts", "examples/**/*.test.ts"],
  },
});
