import { inspect } from "node:util";
import { ByteBuffer, decodeUTF8, Packet, Value } from "./common.ts";

function decodeValue(bb: ByteBuffer): Value {
  const value = bb.read8();
  switch (value) {
    case 0: // null
      return null;
    case 1: // boolean
      return !!bb.read8();
    case 2: // number
      return bb.read32();
    case 3: // string
      return decodeUTF8(bb.read());
    case 4: // Uint8Array
      return bb.read();
    case 5: { // Value[]
      const count = bb.read32();
      const value: Value[] = [];
      for (let i = 0; i < count; i++) {
        value.push(decodeValue(bb));
      }
      return value;
    }
    case 6: { // { [key: string]: Value }
      const count = bb.read32();
      const value: { [key: string]: Value } = {};
      for (let i = 0; i < count; i++) {
        value[decodeUTF8(bb.read())] = decodeValue(bb);
      }
      return value;
    }
    default:
      return `[ERROR ${value}]`;
      // throw new Error("Invalid packet");
  }
}

export function decodePacketLengthPrefixed(bytes: Uint8Array): Packet {
  const bb = new ByteBuffer(bytes as any);
  const length = bb.read32();
  const packet = decodePacketInner(bb);
  if (bb.ptr - 4 !== length) {
    throw new Error(
      `Invalid packet: ${bb.ptr} !== ${bytes.length};\n${
        inspect({
          bytes: bytes.slice(bb.ptr),
        })
      }`,
    );
  }
  return packet;
}

function decodePacketInner(bb: ByteBuffer): Packet {
  let id = bb.read32();
  const isRequest = (id & 1) === 0;
  id >>>= 1;
  const value = decodeValue(bb);
  if (bb.ptr !== bytes.length) {
    throw new Error(
      `Invalid packet: ${bb.ptr} !== ${bytes.length};\n${
        inspect({
          id,
          isRequest,
          value,
          bytes: bytes.slice(bb.ptr),
        })
      }`,
    );
  }
  return { id, isRequest, value };
}

export function decodePacket(bytes: Uint8Array): Packet {
  // deno-lint-ignore no-explicit-any
  const bb = new ByteBuffer(bytes as any);
  return decodePacketInner(bb);
}

function parseInput(input: string): Uint8Array {
  //  [41, 0, 0, 0]
  input = input.trim();
  input = input.replace(/[\[\]]/g, "");
  const parts = input.split(",");
  const bytes = new Uint8Array(parts.length);
  for (let i = 0; i < parts.length; i++) {
    if (parts[i].trim() === "") {
      continue;
    }
    bytes[i] = parseInt(parts[i].trim());
  }
  return bytes;
}

const input = Deno.args[0];
const bytes = parseInput(input);
const packet = decodePacketLengthPrefixed(bytes);
console.log(packet);
