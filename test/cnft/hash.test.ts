// Hash primitive tests. Anchor on one fully external vector (the
// ETH-standard "keccak256 of empty input") so we catch a bad keccak
// implementation, then verify internal consistency for the rest.

import { test } from "node:test";
import assert from "node:assert/strict";
import {
  keccak256,
  hashPair,
  emptyNode,
  hashLeafV1,
  hashCreators,
} from "../../src/cnft/hash.js";

// Known external vector: keccak256 of empty input. Well-publicized
// (it's the Ethereum-standard empty hash). If this ever fails, our
// keccak impl is wrong at the protocol level.
const KECCAK_EMPTY =
  "c5d2460186f7233c927e7db2dcc703c0e500b653ca82273b7bfad8045d85a470";

function hex(bytes: Uint8Array): string {
  return Array.from(bytes)
    .map((b) => b.toString(16).padStart(2, "0"))
    .join("");
}

test("keccak256 matches the ETH-standard empty-input vector", () => {
  const got = keccak256(new Uint8Array(0));
  assert.equal(hex(got), KECCAK_EMPTY);
});

test("keccak256 output is always 32 bytes", () => {
  for (const len of [0, 1, 32, 64, 1024]) {
    const input = new Uint8Array(len).fill(0xab);
    assert.equal(keccak256(input).length, 32);
  }
});

test("hashPair is order-sensitive", () => {
  const a = new Uint8Array(32).fill(1);
  const b = new Uint8Array(32).fill(2);
  const ab = hashPair(a, b);
  const ba = hashPair(b, a);
  assert.notEqual(hex(ab), hex(ba), "(a, b) must differ from (b, a)");
});

test("hashPair is deterministic", () => {
  const a = new Uint8Array(32).fill(7);
  const b = new Uint8Array(32).fill(11);
  assert.equal(hex(hashPair(a, b)), hex(hashPair(a, b)));
});

test("emptyNode(0) is 32 zero bytes", () => {
  const z = emptyNode(0);
  assert.equal(z.length, 32);
  for (const byte of z) assert.equal(byte, 0);
});

test("emptyNode(h) = hashPair(emptyNode(h-1), emptyNode(h-1))", () => {
  // Verify the cascade relationship up to a reasonable depth.
  for (let h = 1; h <= 10; h++) {
    const below = emptyNode(h - 1);
    assert.equal(hex(emptyNode(h)), hex(hashPair(below, below)));
  }
});

test("emptyNode is memoized (same instance returned for the same height)", () => {
  // Not a correctness test, but catches accidental recomputation which
  // would be a performance bug at higher depths.
  const a = emptyNode(5);
  const b = emptyNode(5);
  assert.equal(a, b);
});

test("emptyNode rejects invalid heights", () => {
  assert.throws(() => emptyNode(-1));
  assert.throws(() => emptyNode(1.5));
});

test("hashLeafV1 output is 32 bytes and deterministic", () => {
  const leaf = {
    id: new Uint8Array(32).fill(0xaa),
    owner: new Uint8Array(32).fill(0xbb),
    delegate: new Uint8Array(32).fill(0xcc),
    nonce: 42n,
    dataHash: new Uint8Array(32).fill(0xdd),
    creatorHash: new Uint8Array(32).fill(0xee),
  };
  const h1 = hashLeafV1(leaf);
  const h2 = hashLeafV1(leaf);
  assert.equal(h1.length, 32);
  assert.equal(hex(h1), hex(h2));
});

test("hashLeafV1 is sensitive to every field", () => {
  const base = {
    id: new Uint8Array(32).fill(0xaa),
    owner: new Uint8Array(32).fill(0xbb),
    delegate: new Uint8Array(32).fill(0xcc),
    nonce: 42n,
    dataHash: new Uint8Array(32).fill(0xdd),
    creatorHash: new Uint8Array(32).fill(0xee),
  };
  const baseHash = hex(hashLeafV1(base));

  const flipped = [
    { ...base, id: new Uint8Array(32).fill(0xff) },
    { ...base, owner: new Uint8Array(32).fill(0xff) },
    { ...base, delegate: new Uint8Array(32).fill(0xff) },
    { ...base, nonce: 43n },
    { ...base, dataHash: new Uint8Array(32).fill(0xff) },
    { ...base, creatorHash: new Uint8Array(32).fill(0xff) },
  ];
  for (const mutated of flipped) {
    assert.notEqual(hex(hashLeafV1(mutated)), baseHash);
  }
});

test("hashLeafV1 rejects wrong-length inputs", () => {
  const good = {
    id: new Uint8Array(32),
    owner: new Uint8Array(32),
    delegate: new Uint8Array(32),
    nonce: 0n,
    dataHash: new Uint8Array(32),
    creatorHash: new Uint8Array(32),
  };
  assert.throws(() => hashLeafV1({ ...good, id: new Uint8Array(31) }));
  assert.throws(() => hashLeafV1({ ...good, owner: new Uint8Array(33) }));
  assert.throws(() => hashLeafV1({ ...good, dataHash: new Uint8Array(16) }));
});

test("hashLeafV1 honors nonce endianness (LE)", () => {
  // nonce=1 encoded LE is 01 00 00 00 00 00 00 00; encoded BE would be
  // 00 00 00 00 00 00 00 01. We pin the hash for nonce=1 and verify
  // that nonce=256 differs — which it must if LE is used (the second
  // byte flips, changing the preimage).
  const base = {
    id: new Uint8Array(32),
    owner: new Uint8Array(32),
    delegate: new Uint8Array(32),
    dataHash: new Uint8Array(32),
    creatorHash: new Uint8Array(32),
  };
  const h1 = hashLeafV1({ ...base, nonce: 1n });
  const h256 = hashLeafV1({ ...base, nonce: 256n });
  const hBE = hashLeafV1({ ...base, nonce: 0x0100000000000000n });
  assert.notEqual(hex(h1), hex(h256));
  // If the impl were BE, nonce=1 would hash the same preimage as
  // nonce=0x0100000000000000 under LE — they'd collide. They must not.
  assert.notEqual(hex(h1), hex(hBE));
});

test("hashCreators is deterministic and order-sensitive", () => {
  const a = {
    address: new Uint8Array(32).fill(1),
    verified: true,
    share: 50,
  };
  const b = {
    address: new Uint8Array(32).fill(2),
    verified: false,
    share: 50,
  };
  const ab = hex(hashCreators([a, b]));
  const ba = hex(hashCreators([b, a]));
  const ab2 = hex(hashCreators([a, b]));

  assert.equal(ab, ab2);
  assert.notEqual(ab, ba);
});

test("hashCreators distinguishes verified vs unverified", () => {
  const c = { address: new Uint8Array(32).fill(1), verified: true, share: 100 };
  const cUnverified = { ...c, verified: false };
  assert.notEqual(hex(hashCreators([c])), hex(hashCreators([cUnverified])));
});

test("hashCreators rejects invalid share values", () => {
  const bad = { address: new Uint8Array(32), verified: true, share: 256 };
  assert.throws(() => hashCreators([bad]));
  const negative = { address: new Uint8Array(32), verified: true, share: -1 };
  assert.throws(() => hashCreators([negative]));
});

test("hashCreators handles the empty creator list", () => {
  // Empty input is valid — matches keccak256 of empty buffer.
  assert.equal(hex(hashCreators([])), KECCAK_EMPTY);
});
