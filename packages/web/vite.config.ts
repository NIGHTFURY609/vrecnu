import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";
import { VitePWA } from "vite-plugin-pwa";

/**
 * Declared locally rather than pulling in `@types/node`: this file runs in Node, but adding
 * node types to the package would also make them available to application code, which must
 * stay browser-only.
 */
declare const process: { env: Record<string, string | undefined> };

/**
 * `base` is configurable so a project-page deploy (e.g. GitHub Pages at /vrecnu/) works.
 * It was previously unset, which silently breaks every asset URL on a subpath host.
 */
const base = process.env.VRECNU_BASE ?? "/";

export default defineConfig({
  base,
  plugins: [
    react(),
    tailwindcss(),
    VitePWA({
      registerType: "prompt",
      includeAssets: ["favicon-32.png", "apple-touch-icon.png"],
      manifest: {
        name: "vrecnu — screen recorder",
        short_name: "vrecnu",
        description:
          "Record your screen and upload it straight to your own Google Drive. Nothing is stored on our servers.",
        theme_color: "#0f172a",
        background_color: "#ffffff",
        display: "standalone",
        orientation: "any",
        start_url: base,
        scope: base,
        icons: [
          { src: "icon-192.png", sizes: "192x192", type: "image/png" },
          { src: "icon-512.png", sizes: "512x512", type: "image/png" },
          { src: "icon-maskable-512.png", sizes: "512x512", type: "image/png", purpose: "maskable" },
        ],
      },
      workbox: {
        // The app shell is what needs to work offline. Recordings live in OPFS and are never
        // cached by the service worker — caching user video would duplicate gigabytes into
        // the Cache API for no benefit.
        globPatterns: ["**/*.{js,css,html,png,svg,woff2}"],
        // A recording can be large; the SW must not try to precache a stray media file.
        maximumFileSizeToCacheInBytes: 4 * 1024 * 1024,
        navigateFallback: "index.html",
        cleanupOutdatedCaches: true,
        // Google's identity script must always come from the network — a cached OAuth
        // endpoint is both useless offline and a source of stale-token bugs.
        navigateFallbackDenylist: [/^\/oauth/],
        runtimeCaching: [
          {
            urlPattern: /^https:\/\/accounts\.google\.com\/.*/i,
            handler: "NetworkOnly",
          },
          {
            urlPattern: /^https:\/\/(www\.)?googleapis\.com\/.*/i,
            handler: "NetworkOnly",
          },
        ],
      },
      devOptions: { enabled: false },
    }),
  ],
  worker: { format: "es" },
});
