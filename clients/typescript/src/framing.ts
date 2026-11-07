/**
 * GWP framing (architecture §7): 4-byte little-endian body length +
 * postcard-encoded body. `FrameReader` reassembles frames from an
 * incremental byte stream (TCP/WebSocket chunks may split or coalesce).
 */

import { DecodeError } from "./postcard.ts";

export const MAX_FRAME_BYTES = 64 * 1024 * 1024;

export function encodeFrame(payload: Uint8Array): Uint8Array {
  if (payload.length > MAX_FRAME_BYTES) {
    throw new Error(`frame body of ${payload.length} bytes exceeds the guard`);
  }
  const out = new Uint8Array(4 + payload.length);
  const view = new DataView(out.buffer);
  view.setUint32(0, payload.length, true);
  out.set(payload, 4);
  return out;
}

export function decodeFramePrefix(bytes: Uint8Array): number {
  if (bytes.length < 4) throw new DecodeError("length prefix truncated");
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  const len = view.getUint32(0, true);
  if (len > MAX_FRAME_BYTES) throw new DecodeError(`frame body of ${len} bytes exceeds the guard`);
  return len;
}

/** Incremental frame reassembler over a byte stream. */
export class FrameReader {
  private buf: Uint8Array = new Uint8Array(0);

  /** Feed raw stream bytes; returns every complete frame body (in order). */
  push(bytes: Uint8Array): Uint8Array[] {
    const merged = new Uint8Array(this.buf.length + bytes.length);
    merged.set(this.buf, 0);
    merged.set(bytes, this.buf.length);
    this.buf = merged;

    const frames: Uint8Array[] = [];
    for (;;) {
      if (this.buf.length < 4) break;
      const len = decodeFramePrefix(this.buf);
      if (this.buf.length < 4 + len) break;
      frames.push(this.buf.subarray(4, 4 + len));
      this.buf = this.buf.subarray(4 + len);
    }
    return frames;
  }
}
