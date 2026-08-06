/**
 * Minimal EBML reader — only what is needed to locate (or make room for) the WebM
 * Info/Duration element. Deliberately not a general Matroska parser.
 *
 * Pure functions over `Uint8Array`; no DOM, no worker globals. Lives in `packages/web`
 * rather than `packages/core` because it is a browser-recorder concern, but it imports
 * nothing platform-specific and is directly unit-testable.
 */

export const EBML_HEADER_ID = 0x1a45dfa3;
export const SEGMENT_ID = 0x18538067;
export const SEEK_HEAD_ID = 0x114d9b74;
export const INFO_ID = 0x1549a966;
export const TRACKS_ID = 0x1654ae6b;
export const CLUSTER_ID = 0x1f43b675;
export const CUES_ID = 0x1c53bb6b;
export const TIMECODE_SCALE_ID = 0x2ad7b1;
export const DURATION_ID = 0x4489;

/** Matroska's default TimecodeScale: 1 ms expressed in nanoseconds. */
export const DEFAULT_TIMECODE_SCALE_NS = 1_000_000;

export interface ElementId {
  readonly id: number;
  readonly length: number;
}

export interface ElementSize {
  /** Data bits only, marker stripped. Meaningless when `unknown` is true. */
  readonly value: number;
  readonly length: number;
  /** All data bits set — Matroska's "size not yet known", used for live-muxed Segments. */
  readonly unknown: boolean;
}

/**
 * Element IDs keep their marker bits (Segment really is the four bytes `18 53 80 67`),
 * which is why this cannot share an implementation with {@link readElementSize}.
 */
export function readElementId(bytes: Uint8Array, offset: number): ElementId | null {
  const first = bytes[offset];
  if (first === undefined || first === 0) {
    return null;
  }
  let length = 1;
  for (let mask = 0x80; mask > 0; mask >>= 1) {
    if (first & mask) {
      break;
    }
    length++;
  }
  if (length > 4 || offset + length > bytes.length) {
    return null;
  }
  let id = 0;
  for (let i = 0; i < length; i++) {
    id = id * 0x100 + (bytes[offset + i] ?? 0);
  }
  return { id, length };
}

/** Sizes are VINTs whose marker bit is stripped from the value. */
export function readElementSize(bytes: Uint8Array, offset: number): ElementSize | null {
  const first = bytes[offset];
  if (first === undefined || first === 0) {
    return null;
  }
  let length = 1;
  for (let mask = 0x80; mask > 0; mask >>= 1) {
    if (first & mask) {
      break;
    }
    length++;
  }
  if (length > 8 || offset + length > bytes.length) {
    return null;
  }
  const dataMask = length >= 8 ? 0 : 0xff >> length;
  let value = first & dataMask;
  let unknown = value === dataMask;
  for (let i = 1; i < length; i++) {
    const byte = bytes[offset + i] ?? 0;
    if (byte !== 0xff) {
      unknown = false;
    }
    value = value * 0x100 + byte;
  }
  return { value, length, unknown };
}

/**
 * Encodes `value` as a fixed 8-byte VINT. Always used when rewriting a size that has to grow:
 * the widest encoding always has room, so a rewrite can never fail to fit.
 */
export function writeFixedWidthSize(value: number): Uint8Array {
  const out = new Uint8Array(8);
  out[0] = 0x01;
  let remaining = value;
  for (let i = 7; i >= 1; i--) {
    out[i] = remaining % 0x100;
    remaining = Math.floor(remaining / 0x100);
  }
  return out;
}

export interface ElementHeader {
  readonly id: number;
  /** Offset of the ID's first byte. */
  readonly start: number;
  /** Offset of the size VINT's first byte. */
  readonly sizeOffset: number;
  readonly sizeLength: number;
  readonly size: number;
  readonly unknownSize: boolean;
  /** Offset of the first content byte. */
  readonly contentStart: number;
}

export function readElementHeader(bytes: Uint8Array, offset: number): ElementHeader | null {
  const id = readElementId(bytes, offset);
  if (!id) {
    return null;
  }
  const sizeOffset = offset + id.length;
  const size = readElementSize(bytes, sizeOffset);
  if (!size) {
    return null;
  }
  return {
    id: id.id,
    start: offset,
    sizeOffset,
    sizeLength: size.length,
    size: size.value,
    unknownSize: size.unknown,
    contentStart: sizeOffset + size.length,
  };
}

/** Reads an unsigned-integer element body (1–8 bytes, big-endian). */
export function readUintBody(bytes: Uint8Array, offset: number, length: number): number | null {
  if (length < 1 || length > 8 || offset + length > bytes.length) {
    return null;
  }
  let value = 0;
  for (let i = 0; i < length; i++) {
    value = value * 0x100 + (bytes[offset + i] ?? 0);
  }
  return value;
}
