// Generates vrecnu's PWA icons as raw PNGs, with no image dependency.
// A screen recorder's mark: dark slate rounded square + a red record dot.
import { writeFileSync, mkdirSync } from "node:fs";
import { join } from "node:path";
import { draw, encodePng } from "../../../scripts/icon-art.mjs";

const outDir = process.argv[2];
mkdirSync(outDir, { recursive: true });

const targets = [
  { name: "icon-192.png", size: 192, padding: 0, transparentBackground: true },
  { name: "icon-512.png", size: 512, padding: 0, transparentBackground: true },
  // Maskable icons get cropped to a safe zone, so the mark needs generous padding.
  { name: "icon-maskable-512.png", size: 512, padding: 0.0, transparentBackground: false },
  { name: "apple-touch-icon.png", size: 180, padding: 0, transparentBackground: false },
  { name: "favicon-32.png", size: 32, padding: 0, transparentBackground: true },
];

for (const t of targets) {
  const png = encodePng(t.size, t.size, draw(t.size, t));
  writeFileSync(join(outDir, t.name), png);
  console.log(`${t.name} ${t.size}x${t.size} ${png.length} bytes`);
}
