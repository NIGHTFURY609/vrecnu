/**
 * "Save a local copy" (ADR-003 amendment — the save prompt moved from *before* recording to
 * an optional action at the end).
 *
 * This matters more than it looks. Without it, a user with no Drive configured is handed a
 * `MockStorageProvider` link to a file that does not exist, and their recording is then
 * unreachable. A download path means the recording is never trapped in the browser.
 *
 * `showSaveFilePicker` streams straight to the chosen file, so a multi-gigabyte recording
 * never passes through memory. The anchor fallback is used only where the picker is missing
 * (Firefox, Safari), and there the browser still streams the Blob from disk itself.
 */

interface SaveFilePickerOptions {
  suggestedName?: string;
  types?: { description: string; accept: Record<string, string[]> }[];
}

/**
 * `FileSystemFileHandle` (with `createWritable`) is already in lib.dom; only the picker
 * entry point is missing, so that is all we declare.
 */
interface WindowWithSavePicker {
  showSaveFilePicker?: (options?: SaveFilePickerOptions) => Promise<FileSystemFileHandle>;
}

export type SaveOutcome = "saved" | "cancelled" | "unsupported-fallback";

function extensionOf(fileName: string): string {
  const dot = fileName.lastIndexOf(".");
  return dot >= 0 ? fileName.slice(dot) : ".webm";
}

/** Triggers a normal browser download. Used where `showSaveFilePicker` is unavailable. */
function downloadViaAnchor(blob: Blob, fileName: string): void {
  const url = URL.createObjectURL(blob);
  const anchor = document.createElement("a");
  anchor.href = url;
  anchor.download = fileName;
  document.body.appendChild(anchor);
  anchor.click();
  anchor.remove();
  // Revoking immediately can cancel the download in some browsers; give it a moment.
  window.setTimeout(() => URL.revokeObjectURL(url), 60_000);
}

export async function saveRecordingToDisk(
  blob: Blob,
  fileName: string,
  mimeType: string,
): Promise<SaveOutcome> {
  const picker = (window as unknown as WindowWithSavePicker).showSaveFilePicker;
  if (typeof picker !== "function") {
    downloadViaAnchor(blob, fileName);
    return "unsupported-fallback";
  }

  try {
    const handle = await picker({
      suggestedName: fileName,
      types: [
        {
          description: "Video",
          accept: { [mimeType.split(";")[0] ?? "video/webm"]: [extensionOf(fileName)] },
        },
      ],
    });
    const writable = await handle.createWritable();
    await writable.write(blob);
    await writable.close();
    return "saved";
  } catch (err) {
    if (err instanceof DOMException && err.name === "AbortError") {
      return "cancelled";
    }
    // A picker that exists but fails (sandboxed iframe, policy) should still let the user
    // get their file rather than dead-ending.
    downloadViaAnchor(blob, fileName);
    return "unsupported-fallback";
  }
}

/** Characters Windows and macOS reject in a filename. Spaces and hyphens are legal. */
const ILLEGAL_FILENAME_CHARS = /["*/:<>?\\|]/g;

/**
 * Makes a user-supplied name safe to save. Deliberately preserves spaces and hyphens — they
 * are legal on every target platform and are part of our own default naming.
 */
export function sanitizeFileName(name: string, fallback: string): string {
  const cleaned = name
    .replace(ILLEGAL_FILENAME_CHARS, "")
    .replace(/\s+/g, " ")
    .trim()
    .replace(/^\.+/, "");
  return cleaned.length > 0 ? cleaned.slice(0, 120) : fallback;
}
