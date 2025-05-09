import {
  ByteBuffer,
  encodeUTF8,
  Packet,
  Value,
  writeUInt32LE,
} from "./common.ts";

function encodeValue(value: Value, bb: ByteBuffer) {
  if (value === null) {
    bb.write8(0);
  } else if (typeof value === "boolean") {
    bb.write8(1);
    bb.write8(+value);
  } else if (typeof value === "number") {
    bb.write8(2);
    bb.write32(value | 0);
  } else if (typeof value === "string") {
    bb.write8(3);
    bb.write(encodeUTF8(value));
  } else if (value instanceof Uint8Array) {
    bb.write8(4);
    bb.write(value);
  } else if (value instanceof Array) {
    bb.write8(5);
    bb.write32(value.length);
    for (const item of value) {
      encodeValue(item, bb);
    }
  } else {
    const keys = Object.keys(value);
    bb.write8(6);
    bb.write32(keys.length);
    for (const key of keys) {
      bb.write(encodeUTF8(key));
      encodeValue(value[key], bb);
    }
  }
}

export function encodePacket(packet: Packet): Uint8Array {
  const bb = new ByteBuffer();
  bb.write32(0); // Reserve space for the length
  bb.write32((packet.id << 1) | +!packet.isRequest);
  encodeValue(packet.value, bb);

  writeUInt32LE(bb.buf, bb.len - 4, 0); // Patch the length in
  const res = bb.buf.subarray(0, bb.len);
  return res;
}

const packet = {
  id: 0,
  isRequest: false,
  value: {
    errors: [],
    warnings: [],
  },
};

const encoded = encodePacket(packet);

const s = Array.from(encoded).map((v) => v.toString()).join(", ");

console.log(`vec![${s}]`);
