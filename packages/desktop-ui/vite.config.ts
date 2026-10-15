import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";

// vrecnu-shell-tauri embeds dist/ at compile time (tauri.conf.json → build.frontendDist), so
// this must be built before cargo compiles the desktop workspace. WebView2 is Chromium, so
// ES2022 needs no down-levelling.
export default defineConfig({
  plugins: [react(), tailwindcss()],
  clearScreen: false,
  build: { target: "es2022" },
});
