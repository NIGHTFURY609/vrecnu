/**
 * Enforces the PWA rows of AGENTS.md §1. Those numbers are described as "a blocking review
 * finding, not a discussion", and ADR-004 notes budgets checked by hand will drift — so they
 * are checked here, in CI, against a real build.
 *
 * Only the *initial* load is measured: the entry HTML, the CSS and JS it references directly,
 * and nothing else. Lazily-loaded chunks and the service worker's precache are deliberately
 * excluded — they are not what the user waits for on first paint.
 */
import { readFileSync, readdirSync, statSync } from "node:fs";
import { gzipSync } from "node:zlib";
import { join } from "node:path";

const BUDGETS = {
  initialJsGzipBytes: 150 * 1024,
  totalFirstLoadGzipBytes: 400 * 1024,
};

const distDir = process.argv[2] ?? "dist";

function gzipSize(path) {
  return gzipSync(readFileSync(path), { level: 9 }).length;
}

const html = readFileSync(join(distDir, "index.html"), "utf8");

// Everything index.html pulls in synchronously, which is what "first load" means.
const referenced = [...html.matchAll(/(?:src|href)="([^"]+)"/g)]
  .map((m) => m[1])
  .filter((href) => href.endsWith(".js") || href.endsWith(".css"))
  .map((href) => href.replace(/^\.?\//, ""));

if (referenced.length === 0) {
  console.error("Budget check found no referenced assets in index.html — is the build broken?");
  process.exit(1);
}

let initialJs = 0;
let total = gzipSize(join(distDir, "index.html"));
const rows = [];

for (const rel of referenced) {
  const path = join(distDir, rel);
  if (!statSync(path, { throwIfNoEntry: false })) {
    console.error(`Budget check: index.html references ${rel}, which is not in the build output.`);
    process.exit(1);
  }
  const size = gzipSize(path);
  total += size;
  if (rel.endsWith(".js")) {
    initialJs += size;
  }
  rows.push([rel, size]);
}

const kb = (n) => `${(n / 1024).toFixed(1)} KB`;
for (const [name, size] of rows) {
  console.log(`  ${name.padEnd(48)} ${kb(size).padStart(10)} gzip`);
}

const checks = [
  ["initial JS (gzip)", initialJs, BUDGETS.initialJsGzipBytes],
  ["total first load (gzip)", total, BUDGETS.totalFirstLoadGzipBytes],
];

let failed = false;
console.log("");
for (const [label, actual, budget] of checks) {
  const pct = Math.round((actual / budget) * 100);
  const status = actual <= budget ? "OK  " : "OVER";
  if (actual > budget) {
    failed = true;
  }
  console.log(`${status} ${label.padEnd(26)} ${kb(actual).padStart(10)} / ${kb(budget).padStart(10)} (${pct}%)`);
}

// Reported for visibility only — AGENTS.md §1 sets no budget for precached bytes, and the
// service worker downloads them after first paint.
const swAssets = readdirSync(join(distDir, "assets"), { withFileTypes: true })
  .filter((e) => e.isFile())
  .reduce((sum, e) => sum + gzipSize(join(distDir, "assets", e.name)), 0);
console.log(`     (all built assets, gzip: ${kb(swAssets)} — not budgeted)`);

if (failed) {
  console.error("\nA budget in AGENTS.md §1 was exceeded. That is a blocking finding: either bring the");
  console.error("size back down, or amend the budget via an ADR with measurements attached.");
  process.exit(1);
}
