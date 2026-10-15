/** "m:ss", or "h:mm:ss" from an hour up. Matches the tray tooltip (vrecnu-shell-tauri lifecycle.rs). */
export function formatElapsed(ms: number): string {
  const total = Math.floor(Math.max(0, ms) / 1000);
  const hours = Math.floor(total / 3600);
  const minutes = Math.floor(total / 60) % 60;
  const seconds = String(total % 60).padStart(2, "0");
  return hours > 0 ? `${hours}:${String(minutes).padStart(2, "0")}:${seconds}` : `${minutes}:${seconds}`;
}

const UNITS = ["KB", "MB", "GB", "TB"] as const;

/** Decimal units (1 KB = 1000 B), one decimal place from KB up. */
export function formatBytes(bytes: number): string {
  let value = Math.max(0, bytes);
  if (value < 1000) return `${value} B`;
  let unit: (typeof UNITS)[number] = "KB";
  for (const next of UNITS) {
    value /= 1000;
    unit = next;
    if (value < 1000) break;
  }
  return `${value.toFixed(1)} ${unit}`;
}

/** The last segment of a Windows or POSIX path. */
export function fileName(path: string): string {
  return path.split(/[\\/]/).pop() ?? path;
}
