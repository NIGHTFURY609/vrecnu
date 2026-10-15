// Generates vrecnu-shell-tauri's icons from the same art as the PWA, with no image dependency.
// tauri-build embeds icons/icon.ico as the exe's resource, and the tray uses the same image.
import { writeFileSync, mkdirSync } from "node:fs";
import { join } from "node:path";
import { draw, encodePng, encodeIco } from "../../../scripts/icon-art.mjs";

const outDir = process.argv[2];
mkdirSync(outDir, { recursive: true });
const png = (size) => encodePng(size, size, draw(size, { padding: 0, transparentBackground: true }));

for (const [name, size] of [["32x32.png", 32], ["128x128.png", 128], ["128x128@2x.png", 256], ["icon.png", 512]]) {
  writeFileSync(join(outDir, name), png(size));
}
const ico = encodeIco([16, 24, 32, 48, 64, 256].map((size) => ({ size, png: png(size) })));
writeFileSync(join(outDir, "icon.ico"), ico);
console.log(`icons written to ${outDir}`);
