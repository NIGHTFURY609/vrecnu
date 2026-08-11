// Generates vrecnu's PWA icons as raw PNGs, with no image dependency.
// A screen recorder's mark: dark slate rounded square + a red record dot.
import { deflateSync } from "node:zlib";
import { writeFileSync, mkdirSync } from "node:fs";
import { join } from "node:path";

const CRC_TABLE = (() => {
  const table = new Int32Array(256);
  for (let n = 0; n < 256; n++) {
    let c = n;
    for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
    table[n] = c;
  }
  return table;
})();

function crc32(buf) {
  let c = -1;
  for (const b of buf) c = CRC_TABLE[(c ^ b) & 0xff] ^ (c >>> 8);
  return (c ^ -1) >>> 0;
}

function chunk(type, data) {
  const len = Buffer.alloc(4);
  len.writeUInt32BE(data.length);
  const body = Buffer.concat([Buffer.from(type, "ascii"), data]);
  const crc = Buffer.alloc(4);
  crc.writeUInt32BE(crc32(body));
  return Buffer.concat([len, body, crc]);
}

function encodePng(width, height, rgba) {
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(width, 0);
  ihdr.writeUInt32BE(height, 4);
  ihdr[8] = 8; // bit depth
  ihdr[9] = 6; // truecolour + alpha
  // scanlines, each prefixed with filter type 0
  const raw = Buffer.alloc(height * (width * 4 + 1));
  for (let y = 0; y < height; y++) {
    raw[y * (width * 4 + 1)] = 0;
    rgba.copy(raw, y * (width * 4 + 1) + 1, y * width * 4, (y + 1) * width * 4);
  }
  return Buffer.concat([
    Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
    chunk("IHDR", ihdr),
    chunk("IDAT", deflateSync(raw, { level: 9 })),
    chunk("IEND", Buffer.alloc(0)),
  ]);
}

/** Anti-aliased by supersampling the coverage of each pixel. */
function draw(size, { padding, transparentBackground }) {
  const rgba = Buffer.alloc(size * size * 4);
  const bg = [0x0f, 0x17, 0x2a];
  const dot = [0xdc, 0x26, 0x26];
  const inset = size * padding;
  const boxRadius = size * 0.22;
  const centre = size / 2;
  const dotRadius = (size - inset * 2) * 0.26;
  const SS = 3;

  const insideRoundedRect = (x, y) => {
    const minX = inset;
    const maxX = size - inset;
    if (x < minX || x > maxX || y < minX || y > maxX) return false;
    const cx = Math.min(Math.max(x, minX + boxRadius), maxX - boxRadius);
    const cy = Math.min(Math.max(y, minX + boxRadius), maxX - boxRadius);
    return (x - cx) ** 2 + (y - cy) ** 2 <= boxRadius ** 2;
  };

  for (let y = 0; y < size; y++) {
    for (let x = 0; x < size; x++) {
      let boxHits = 0;
      let dotHits = 0;
      for (let sy = 0; sy < SS; sy++) {
        for (let sx = 0; sx < SS; sx++) {
          const px = x + (sx + 0.5) / SS;
          const py = y + (sy + 0.5) / SS;
          if (insideRoundedRect(px, py)) boxHits++;
          if ((px - centre) ** 2 + (py - centre) ** 2 <= dotRadius ** 2) dotHits++;
        }
      }
      const total = SS * SS;
      const boxA = transparentBackground ? boxHits / total : 1;
      const dotA = dotHits / total;
      const base = transparentBackground ? bg : bg;
      const r = Math.round(base[0] * (1 - dotA) + dot[0] * dotA);
      const g = Math.round(base[1] * (1 - dotA) + dot[1] * dotA);
      const b = Math.round(base[2] * (1 - dotA) + dot[2] * dotA);
      const i = (y * size + x) * 4;
      rgba[i] = r;
      rgba[i + 1] = g;
      rgba[i + 2] = b;
      rgba[i + 3] = Math.round(255 * Math.max(boxA, dotA));
    }
  }
  return rgba;
}

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
