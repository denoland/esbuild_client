export interface Packet {
  id: number;
  isRequest: boolean;
  value: Value;
}

const decoder = new TextDecoder();
export type Value =
  | null
  | boolean
  | number
  | string
  | Uint8Array
  | Value[]
  | { [key: string]: Value };

export function writeUInt32LE(
  buffer: Uint8Array,
  value: number,
  offset: number,
): void {
  buffer[offset++] = value;
  buffer[offset++] = value >> 8;
  buffer[offset++] = value >> 16;
  buffer[offset++] = value >> 24;
}

export function readUInt32LE(buffer: Uint8Array, offset: number): number {
  return buffer[offset++] |
    (buffer[offset++] << 8) |
    (buffer[offset++] << 16) |
    (buffer[offset++] << 24);
}

export const decodeUTF8: (bytes: Uint8Array) => string = (bytes) =>
  decoder.decode(bytes);

const encoder = new TextEncoder();
export const encodeUTF8 = (text: string) => encoder.encode(text);

export class ByteBuffer {
  len = 0;
  ptr = 0;

  constructor(public buf = new Uint8Array(1024)) {
  }

  private _write(delta: number): number {
    if (this.len + delta > this.buf.length) {
      const clone = new Uint8Array((this.len + delta) * 2);
      clone.set(this.buf);
      this.buf = clone;
    }
    this.len += delta;
    return this.len - delta;
  }

  write8(value: number): void {
    const offset = this._write(1);
    this.buf[offset] = value;
  }

  write32(value: number): void {
    const offset = this._write(4);
    writeUInt32LE(this.buf, value, offset);
  }

  write(bytes: Uint8Array): void {
    const offset = this._write(4 + bytes.length);
    writeUInt32LE(this.buf, bytes.length, offset);
    this.buf.set(bytes, offset + 4);
  }

  private _read(delta: number): number {
    if (this.ptr + delta > this.buf.length) {
      throw new Error("Invalid packet");
    }
    this.ptr += delta;
    return this.ptr - delta;
  }

  read8(): number {
    return this.buf[this._read(1)];
  }

  read32(): number {
    return readUInt32LE(this.buf, this._read(4));
  }

  read(): Uint8Array {
    const length = this.read32();
    const bytes = new Uint8Array(length);
    const ptr = this._read(bytes.length);
    bytes.set(this.buf.subarray(ptr, ptr + length));
    return bytes;
  }
}
