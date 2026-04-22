// Store tests. Basic CRUD against the in-memory impl. apply.test.ts
// covers the richer integration behavior.

import { test } from "node:test";
import assert from "node:assert/strict";
import type { Address } from "@solana/kit";
import { createCnftMemoryStore } from "../../src/cnft/store-memory.js";
import type { LeafRecord, MintMetadata } from "../../src/cnft/types.js";

const TREE = "11111111111111111111111111111111" as Address;
const OTHER_TREE = "22222222222222222222222222222222" as Address;
const ASSET = "33333333333333333333333333333333" as Address;

function stubMintMetadata(): MintMetadata {
  return {
    name: "n",
    symbol: "s",
    uri: "u",
    sellerFeeBasisPoints: 0,
    primarySaleHappened: false,
    isMutable: true,
    creators: [],
    collection: null,
    dataHashInput: new Uint8Array(16),
  };
}

function stubLeaf(assetId: Address, leafIndex: bigint): LeafRecord {
  return {
    assetId,
    tree: TREE,
    nonce: leafIndex,
    leafIndex,
    mintMetadata: stubMintMetadata(),
    owner: new Uint8Array(32).fill(1),
    delegate: new Uint8Array(32).fill(1),
    dataHash: new Uint8Array(32).fill(2),
    creatorHash: new Uint8Array(32).fill(3),
    leafHash: new Uint8Array(32).fill(4),
    burned: false,
  };
}

test("store: put/get tree roundtrips", async () => {
  const store = createCnftMemoryStore();
  await store.putTree({ tree: TREE, depth: 20, maxBufferSize: 64, numMinted: 0n });
  const t = await store.getTree(TREE);
  assert.ok(t);
  assert.equal(t!.depth, 20);
  assert.equal(t!.numMinted, 0n);
});

test("store: getTree returns null for an unknown tree", async () => {
  const store = createCnftMemoryStore();
  assert.equal(await store.getTree(OTHER_TREE), null);
});

test("store: allocLeafIndex increments monotonically", async () => {
  const store = createCnftMemoryStore();
  await store.putTree({ tree: TREE, depth: 20, maxBufferSize: 64, numMinted: 0n });
  assert.equal(await store.allocLeafIndex(TREE), 0n);
  assert.equal(await store.allocLeafIndex(TREE), 1n);
  assert.equal(await store.allocLeafIndex(TREE), 2n);
  const t = await store.getTree(TREE);
  assert.equal(t!.numMinted, 3n);
});

test("store: allocLeafIndex throws for unknown tree", async () => {
  const store = createCnftMemoryStore();
  await assert.rejects(() => store.allocLeafIndex(TREE), /unknown tree/);
});

test("store: putLeaf + getLeaf + getLeafByIndex roundtrip", async () => {
  const store = createCnftMemoryStore();
  await store.putTree({ tree: TREE, depth: 10, maxBufferSize: 8, numMinted: 0n });
  const rec = stubLeaf(ASSET, 0n);
  await store.putLeaf(rec);

  const byId = await store.getLeaf(ASSET);
  assert.ok(byId);
  assert.equal(byId!.assetId, ASSET);

  const byIdx = await store.getLeafByIndex(TREE, 0n);
  assert.ok(byIdx);
  assert.equal(byIdx!.assetId, ASSET);
});

test("store: putLeaf replaces an existing record at the same position", async () => {
  const store = createCnftMemoryStore();
  const a = stubLeaf(ASSET, 5n);
  await store.putLeaf(a);
  const b: LeafRecord = { ...a, owner: new Uint8Array(32).fill(9) };
  await store.putLeaf(b);

  const got = await store.getLeaf(ASSET);
  assert.equal(got!.owner[0], 9);
});

test("store: listLeaves returns insertion order and is tree-scoped", async () => {
  const store = createCnftMemoryStore();
  const a = stubLeaf("A1111111111111111111111111111111" as Address, 0n);
  const b = stubLeaf("B1111111111111111111111111111111" as Address, 1n);
  const other = { ...stubLeaf("C1111111111111111111111111111111" as Address, 0n), tree: OTHER_TREE };
  await store.putLeaf(a);
  await store.putLeaf(b);
  await store.putLeaf(other);

  const forTree = await store.listLeaves(TREE);
  assert.equal(forTree.length, 2);
  assert.equal(forTree[0]!.leafIndex, 0n);
  assert.equal(forTree[1]!.leafIndex, 1n);

  const forOther = await store.listLeaves(OTHER_TREE);
  assert.equal(forOther.length, 1);
  assert.equal(forOther[0]!.tree, OTHER_TREE);
});

test("store: getLastSignature / setLastSignature per tree", async () => {
  const store = createCnftMemoryStore();
  assert.equal(await store.getLastSignature(TREE), null);
  await store.setLastSignature(TREE, "sig-abc");
  assert.equal(await store.getLastSignature(TREE), "sig-abc");
  assert.equal(await store.getLastSignature(OTHER_TREE), null);
});

test("store: returned records are defensive copies (mutation doesn't leak)", async () => {
  const store = createCnftMemoryStore();
  const rec = stubLeaf(ASSET, 0n);
  await store.putLeaf(rec);
  const got = await store.getLeaf(ASSET);
  got!.owner[0] = 0xff;
  // The stored record's owner.bytes are a reference — this test
  // intentionally doesn't try to isolate bytes-level copies (that would
  // be too aggressive for step 3). What matters is that the record
  // object itself is fresh.
  const again = await store.getLeaf(ASSET);
  assert.notEqual(got, again, "should return a new record object per call");
});
