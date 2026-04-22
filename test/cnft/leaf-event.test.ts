// Encode/decode roundtrips for LeafSchemaEvent. We synthesize the wire
// bytes ourselves (event_type + version + LeafSchema + InstructionName)
// and assert our decoder reconstructs the expected struct. Also covers
// the ChangeLogEvent discriminator-byte reject path so we don't false-
// positive on spl-account-compression CPIs.

import { test } from "node:test";
import assert from "node:assert/strict";
import {
  getAddressDecoder,
  getAddressEncoder,
  type Address,
} from "@solana/kit";
import { getLeafSchemaEncoder } from "../../src/generated/bubblegum/types/leafSchema.js";
import { decodeLeafSchemaEvent } from "../../src/cnft/leaf-event.js";

const addrDecoder = getAddressDecoder();
const addrEncoder = getAddressEncoder();

function addressOfByte(b: number): Address {
  return addrDecoder.decode(new Uint8Array(32).fill(b));
}

function addrToBytes(a: Address): Uint8Array {
  return addrEncoder.encode(a) as Uint8Array;
}

function buildLeafSchemaV1Event(args: {
  id: Address;
  owner: Address;
  delegate: Address;
  nonce: bigint;
  dataHash: Uint8Array;
  creatorHash: Uint8Array;
  opByte?: number; // InstructionName enum value; default 4 (Transfer)
}): Uint8Array {
  const schemaBytes = getLeafSchemaEncoder().encode({
    __kind: "V1",
    id: args.id,
    owner: args.owner,
    delegate: args.delegate,
    nonce: args.nonce,
    dataHash: args.dataHash,
    creatorHash: args.creatorHash,
  });
  const out = new Uint8Array(1 + 1 + schemaBytes.length + 1);
  out[0] = 1; // BubblegumEventType::LeafSchemaEvent
  out[1] = 0; // version
  out.set(schemaBytes, 2);
  out[out.length - 1] = args.opByte ?? 4;
  return out;
}

function hex(bytes: Uint8Array): string {
  return Array.from(bytes)
    .map((b) => b.toString(16).padStart(2, "0"))
    .join("");
}

test("leaf-event: V1 roundtrip preserves every field", () => {
  const id = addressOfByte(0x11);
  const owner = addressOfByte(0x22);
  const delegate = addressOfByte(0x33);
  const bytes = buildLeafSchemaV1Event({
    id,
    owner,
    delegate,
    nonce: 42n,
    dataHash: new Uint8Array(32).fill(0xaa),
    creatorHash: new Uint8Array(32).fill(0xbb),
  });
  const out = decodeLeafSchemaEvent(bytes);
  assert.ok(out);
  assert.equal(out!.schema.kind, "V1");
  if (out!.schema.kind !== "V1") throw new Error("narrow");
  assert.equal(hex(out!.schema.id), hex(addrToBytes(id)));
  assert.equal(hex(out!.schema.owner), hex(addrToBytes(owner)));
  assert.equal(hex(out!.schema.delegate), hex(addrToBytes(delegate)));
  assert.equal(out!.schema.nonce, 42n);
  assert.equal(hex(out!.schema.dataHash), "aa".repeat(32));
  assert.equal(hex(out!.schema.creatorHash), "bb".repeat(32));
});

test("leaf-event: first-byte != 1 is rejected cheaply (ChangeLogEvent path)", () => {
  const bytes = new Uint8Array(100);
  bytes[0] = 0; // Uninitialized / ChangeLogEvent-like discriminator
  assert.equal(decodeLeafSchemaEvent(bytes), null);
});

test("leaf-event: truncated data returns null without throwing", () => {
  assert.equal(decodeLeafSchemaEvent(new Uint8Array(0)), null);
  assert.equal(decodeLeafSchemaEvent(new Uint8Array([1])), null);
  assert.equal(decodeLeafSchemaEvent(new Uint8Array([1, 0])), null);
});

test("leaf-event: op byte is surfaced", () => {
  // InstructionName::Transfer = 4
  const bytes = buildLeafSchemaV1Event({
    id: addressOfByte(1),
    owner: addressOfByte(2),
    delegate: addressOfByte(3),
    nonce: 0n,
    dataHash: new Uint8Array(32),
    creatorHash: new Uint8Array(32),
    opByte: 4,
  });
  const out = decodeLeafSchemaEvent(bytes);
  assert.ok(out);
  // The decoder returns InstructionName as a string label when it
  // recognizes the variant; Transfer is a known enum member.
  assert.match(String(out!.op), /Transfer|4/);
});
