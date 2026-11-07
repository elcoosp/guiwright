/**
 * postcard primitives (LEB128 varints, raw little-endian floats) — the
 * exact codec rules the published `gwp-schema.json` documents.
 *
 * Rules (postcard, https://docs.rs/postcard):
 *  - unsigned ints (u16/u32/u64/usize) are LEB128 varints (7 bits/byte,
 *    MSB = continuation);
 *  - signed ints are zigzag-varints (none appear in GWP V1 payloads);
 *  - f32/f64 are raw little-endian bytes;
 *  - bool is one byte (0x00/0x01);
 *  - option<T> is a 0x00/0x01 tag byte followed by T when present;
 *  - string / seq<T> are a varint length prefix followed by the items;
 *  - enum variants are the varint of the variant index;
 *  - structs encode fields in declaration order.
 */

export class EncodeError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "EncodeError";
  }
}

export class DecodeError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "DecodeError";
  }
}

export class Encoder {
  private buf: number[] = [];

  /** LEB128 varint of a non-negative integer below 2^53. */
  varint(value: number): this {
    if (!Number.isSafeInteger(value) || value < 0) {
      throw new EncodeError(`varint needs a safe non-negative integer, got ${value}`);
    }
    let v = value;
    do {
      let byte = v & 0x7f;
      v = Math.floor(v / 128);
      if (v > 0) byte |= 0x80;
      this.buf.push(byte);
    } while (v > 0);
    return this;
  }

  /** LEB128 varint of a u64 that may exceed Number.MAX_SAFE_INTEGER. */
  varint64(value: bigint): this {
    if (value < 0n) {
      throw new EncodeError(`varint needs a non-negative bigint, got ${value}`);
    }
    let v = value;
    do {
      let byte = Number(v & 0x7fn);
      v >>= 7n;
      if (v > 0n) byte |= 0x80;
      this.buf.push(byte);
    } while (v > 0n);
    return this;
  }

  u32(value: number): this {
    return this.varint(value >>> 0);
  }

  u64(value: number | bigint): this {
    return this.varint64(typeof value === "bigint" ? value : BigInt(value));
  }

  bool(value: boolean): this {
    this.buf.push(value ? 1 : 0);
    return this;
  }

  /** The option tag; follow with the value when present. */
  optionTag(present: boolean): this {
    this.buf.push(present ? 1 : 0);
    return this;
  }

  f64(value: number): this {
    const view = new DataView(new ArrayBuffer(8));
    view.setFloat64(0, value, true);
    for (let i = 0; i < 8; i += 1) this.buf.push(view.getUint8(i));
    return this;
  }

  /** seq<T> / string length prefix. */
  seqLen(n: number): this {
    return this.varint(n);
  }

  str(value: string): this {
    const bytes = new TextEncoder().encode(value);
    this.varint(bytes.length);
    for (const b of bytes) this.buf.push(b);
    return this;
  }

  /** Fixed-size raw bytes (e.g. the `[u8; 16]` tree id). */
  raw(bytes: Uint8Array): this {
    for (const b of bytes) this.buf.push(b);
    return this;
  }

  finish(): Uint8Array {
    return Uint8Array.from(this.buf);
  }
}

export class Decoder {
  private pos = 0;
  private readonly buf: Uint8Array;

  constructor(buf: Uint8Array) {
    this.buf = buf;
  }

  remaining(): number {
    return this.buf.length - this.pos;
  }

  done(): boolean {
    return this.pos === this.buf.length;
  }

  private take(n: number): Uint8Array {
    if (this.remaining() < n) {
      throw new DecodeError(`need ${n} bytes, ${this.remaining()} remain`);
    }
    const out = this.buf.subarray(this.pos, this.pos + n);
    this.pos += n;
    return out;
  }

  varint(): number {
    let result = 0;
    let shift = 0;
    for (;;) {
      if (this.remaining() === 0) throw new DecodeError("varint truncated");
      if (shift >= 32) throw new DecodeError("varint too large for u32");
      const byte = this.take(1)[0];
      result += (byte & 0x7f) * 2 ** shift;
      if ((byte & 0x80) === 0) return result;
      shift += 7;
    }
  }

  varint64(): bigint {
    let result = 0n;
    let shift = 0n;
    for (;;) {
      if (this.remaining() === 0) throw new DecodeError("varint truncated");
      if (shift >= 64n) throw new DecodeError("varint too large for u64");
      const byte = this.take(1)[0];
      result |= BigInt(byte & 0x7f) << shift;
      if ((byte & 0x80) === 0) return result;
      shift += 7n;
    }
  }

  u32(): number {
    return this.varint();
  }

  u64(): bigint {
    return this.varint64();
  }

  /** u64 fields whose values always fit a double (seqs, ms, counts, §7).
   * Asserts the safe range so silent precision loss cannot happen. */
  u64n(): number {
    const v = this.varint64();
    if (v > BigInt(Number.MAX_SAFE_INTEGER)) {
      throw new DecodeError(`u64 ${v} exceeds the safe integer range`);
    }
    return Number(v);
  }

  bool(): boolean {
    const b = this.take(1)[0];
    if (b !== 0 && b !== 1) throw new DecodeError(`bool is 0x00/0x01, got 0x${b.toString(16)}`);
    return b === 1;
  }

  optionTag(): boolean {
    return this.bool();
  }

  f64(): number {
    const bytes = this.take(8);
    const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
    return view.getFloat64(0, true);
  }

  str(): string {
    const len = this.varint();
    return new TextDecoder().decode(this.take(len));
  }

  /** Fixed-size raw bytes. */
  raw(n: number): Uint8Array {
    return this.take(n);
  }
}
