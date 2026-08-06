import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";

/**
 * Builds the standalone publish page. The desktop app embeds this `dist/` in its exe and
 * serves it over the loopback origin (ADR-002), so asset URLs must be relative — the port
 * is not known at build time.
 */
export default defineConfig({
  base: "./",
  plugins: [react(), tailwindcss()],
  build: {
    // The Rust build embeds these by exact name via a compile-time map, so predictable
    // output beats aggressive splitting here.
    assetsInlineLimit: 0,
  },
});
