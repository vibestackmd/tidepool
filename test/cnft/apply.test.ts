// apply-event integration tests. Feed sequences of CnftEvents through
// applyEvent, then check the resulting store state + merkle math.

import { test } from "node:test";
import assert from "node:assert/strict";
import type { Address } from "@solana/kit";
import { applyEvent, deriveAssetId } from "../../src/cnft/apply.js";
import { computeProof, verifyProof } from "../../src/cnft/proof.js";
import { createCnftMemoryStore } from "../../src/cnft/store-memory.js";
import type { CnftEvent, MintMetadata, TreeState } from "../../src/cnft/types.js";

const TREE = "11111111111111111111111111111111" as Address;

function stubMintMetadata(): MintMetadata {
  return {
    name: "Asset",
    symbol: "AST",
    uri: "https://example.com/a.json",
    sellerFeeBasisPoints: 500,
    primarySaleHappened: false,
    isMutable: true,
    creators: [
      { address: new Uint8Array(32).fill(1), verified: false, share: 100 },
    ],
    collection: null,
    // Stand-in preimage — the content doesn't matter as long as keccak
    // over these bytes is deterministic across test runs.
    dataHashInput: new TextEncoder().encode('{"name":"Asset"}'),
  };
}

async function buildTreeState(
  store: ReturnType<typeof createCnftMemoryStore>,
  tree: Address,
): Promise<TreeState> {
  const info = await store.getTree(tree);
  if (!info) throw new Error("tree not in store");
  const leaves = new Map<bigint, Uint8Array>();
  for (const rec of await store.listLeaves(tree)) {
    if (!rec.burned) leaves.set(rec.leafIndex, rec.leafHash);
  }
  return { tree, depth: info.depth, leaves };
}

test("apply: createTree populates tree info with zero mints", async () => {
  const store = createCnftMemoryStore();
  await applyEvent(store, {
    kind: "createTree",
    tree: TREE,
    depth: 10,
    maxBufferSize: 16,
  });
  const t = await store.getTree(TREE);
  assert.ok(t);
  assert.equal(t!.depth, 10);
  assert.equal(t!.numMinted, 0n);
});

test("apply: mint allocates leaf index 0, derives asset id, computes leaf hash", async () => {
  const store = createCnftMemoryStore();
  await applyEvent(store, { kind: "createTree", tree: TREE, depth: 10, maxBufferSize: 16 });

  const owner = new Uint8Array(32).fill(7);
  const delegate = new Uint8Array(32).fill(8);
  const event: CnftEvent = {
    kind: "mint",
    tree: TREE,
    owner,
    delegate,
    metadata: stubMintMetadata(),
    verifyCollection: null,
  };
  await applyEvent(store, event);

  const t = await store.getTree(TREE);
  assert.equal(t!.numMinted, 1n);

  const expectedId = await deriveAssetId(TREE, 0n);
  const rec = await store.getLeaf(expectedId);
  assert.ok(rec, "leaf should exist at derived asset id");
  assert.equal(rec!.leafIndex, 0n);
  assert.equal(rec!.nonce, 0n);
  assert.equal(rec!.burned, false);
  // leafHash is 32 bytes and non-zero.
  assert.equal(rec!.leafHash.length, 32);
  assert.ok(rec!.leafHash.some((b) => b !== 0));
});

test("apply: mint on unknown tree throws", async () => {
  const store = createCnftMemoryStore();
  await assert.rejects(
    applyEvent(store, {
      kind: "mint",
      tree: TREE,
      owner: new Uint8Array(32),
      delegate: new Uint8Array(32),
      metadata: stubMintMetadata(),
      verifyCollection: null,
    }),
    /unknown tree/,
  );
});

test("apply: sequential mints receive monotonic leaf indexes", async () => {
  const store = createCnftMemoryStore();
  await applyEvent(store, { kind: "createTree", tree: TREE, depth: 10, maxBufferSize: 16 });

  for (let i = 0; i < 3; i++) {
    await applyEvent(store, {
      kind: "mint",
      tree: TREE,
      owner: new Uint8Array(32).fill(10 + i),
      delegate: new Uint8Array(32).fill(20 + i),
      metadata: stubMintMetadata(),
      verifyCollection: null,
    });
  }
  const leaves = await store.listLeaves(TREE);
  assert.deepEqual(leaves.map((l) => l.leafIndex), [0n, 1n, 2n]);
});

test("apply: transfer updates owner + delegate, re-hashes leaf", async () => {
  const store = createCnftMemoryStore();
  await applyEvent(store, { kind: "createTree", tree: TREE, depth: 10, maxBufferSize: 16 });

  const owner = new Uint8Array(32).fill(1);
  const delegate = new Uint8Array(32).fill(2);
  await applyEvent(store, {
    kind: "mint",
    tree: TREE,
    owner,
    delegate,
    metadata: stubMintMetadata(),
    verifyCollection: null,
  });

  const mid = await store.getLeafByIndex(TREE, 0n);
  assert.ok(mid);
  const originalHash = mid!.leafHash;

  const newOwner = new Uint8Array(32).fill(9);
  await applyEvent(store, {
    kind: "transfer",
    tree: TREE,
    leafIndex: 0n,
    nonce: 0n,
    newOwner,
    newDelegate: newOwner, // per Bubblegum semantics
    dataHash: mid!.dataHash,
    creatorHash: mid!.creatorHash,
  });

  const after = await store.getLeafByIndex(TREE, 0n);
  assert.ok(after);
  assert.equal(after!.owner[0], 9);
  assert.equal(after!.delegate[0], 9);
  assert.notEqual(hex(after!.leafHash), hex(originalHash));
});

test("apply: transfer with wrong dataHash is a no-op (stale state)", async () => {
  const store = createCnftMemoryStore();
  await applyEvent(store, { kind: "createTree", tree: TREE, depth: 10, maxBufferSize: 16 });
  await applyEvent(store, {
    kind: "mint",
    tree: TREE,
    owner: new Uint8Array(32),
    delegate: new Uint8Array(32),
    metadata: stubMintMetadata(),
    verifyCollection: null,
  });
  const before = await store.getLeafByIndex(TREE, 0n);
  assert.ok(before);

  await applyEvent(store, {
    kind: "transfer",
    tree: TREE,
    leafIndex: 0n,
    nonce: 0n,
    newOwner: new Uint8Array(32).fill(1),
    newDelegate: new Uint8Array(32).fill(1),
    dataHash: new Uint8Array(32).fill(0xff), // intentionally wrong
    creatorHash: before!.creatorHash,
  });

  const after = await store.getLeafByIndex(TREE, 0n);
  assert.ok(after);
  assert.equal(hex(after!.leafHash), hex(before!.leafHash), "should not mutate on mismatch");
});

test("apply: delegate updates just the delegate", async () => {
  const store = createCnftMemoryStore();
  await applyEvent(store, { kind: "createTree", tree: TREE, depth: 10, maxBufferSize: 16 });
  await applyEvent(store, {
    kind: "mint",
    tree: TREE,
    owner: new Uint8Array(32).fill(1),
    delegate: new Uint8Array(32).fill(2),
    metadata: stubMintMetadata(),
    verifyCollection: null,
  });
  const mid = await store.getLeafByIndex(TREE, 0n);
  assert.ok(mid);

  await applyEvent(store, {
    kind: "delegate",
    tree: TREE,
    leafIndex: 0n,
    nonce: 0n,
    newDelegate: new Uint8Array(32).fill(5),
    dataHash: mid!.dataHash,
    creatorHash: mid!.creatorHash,
  });
  const after = await store.getLeafByIndex(TREE, 0n);
  assert.equal(after!.owner[0], 1, "owner unchanged");
  assert.equal(after!.delegate[0], 5, "delegate updated");
});

test("apply: burn marks leaf as burned with an all-zero leaf hash", async () => {
  const store = createCnftMemoryStore();
  await applyEvent(store, { kind: "createTree", tree: TREE, depth: 10, maxBufferSize: 16 });
  await applyEvent(store, {
    kind: "mint",
    tree: TREE,
    owner: new Uint8Array(32),
    delegate: new Uint8Array(32),
    metadata: stubMintMetadata(),
    verifyCollection: null,
  });
  await applyEvent(store, { kind: "burn", tree: TREE, leafIndex: 0n, nonce: 0n });

  const rec = await store.getLeafByIndex(TREE, 0n);
  assert.ok(rec);
  assert.equal(rec!.burned, true);
  for (const b of rec!.leafHash) assert.equal(b, 0);
});

test("apply: full flow — mint two, transfer one, compute verifiable proofs", async () => {
  const store = createCnftMemoryStore();
  await applyEvent(store, { kind: "createTree", tree: TREE, depth: 8, maxBufferSize: 16 });

  // Mint two leaves with distinct owners so the tree has observable state.
  await applyEvent(store, {
    kind: "mint",
    tree: TREE,
    owner: new Uint8Array(32).fill(1),
    delegate: new Uint8Array(32).fill(1),
    metadata: stubMintMetadata(),
    verifyCollection: null,
  });
  await applyEvent(store, {
    kind: "mint",
    tree: TREE,
    owner: new Uint8Array(32).fill(2),
    delegate: new Uint8Array(32).fill(2),
    metadata: stubMintMetadata(),
    verifyCollection: null,
  });

  // Transfer leaf 0 to a new owner.
  const leaf0 = await store.getLeafByIndex(TREE, 0n);
  assert.ok(leaf0);
  const newOwner = new Uint8Array(32).fill(9);
  await applyEvent(store, {
    kind: "transfer",
    tree: TREE,
    leafIndex: 0n,
    nonce: 0n,
    newOwner,
    newDelegate: newOwner,
    dataHash: leaf0!.dataHash,
    creatorHash: leaf0!.creatorHash,
  });

  // Pull resulting state, compute proofs for both leaves, verify.
  const state = await buildTreeState(store, TREE);
  for (const idx of [0n, 1n]) {
    const p = computeProof(state, idx);
    assert.ok(
      verifyProof({ leaf: p.leaf, proof: p.proof, leafIndex: idx, root: p.root }),
      `proof failed for leaf ${idx}`,
    );
  }

  // After burning leaf 1, its slot should verify as the empty leaf.
  await applyEvent(store, { kind: "burn", tree: TREE, leafIndex: 1n, nonce: 1n });
  const state2 = await buildTreeState(store, TREE);
  const p1 = computeProof(state2, 1n);
  assert.equal(hex(p1.leaf), "00".repeat(32), "burned slot = empty leaf");
  assert.ok(verifyProof({ leaf: p1.leaf, proof: p1.proof, leafIndex: 1n, root: p1.root }));
});

function hex(bytes: Uint8Array): string {
  return Array.from(bytes)
    .map((b) => b.toString(16).padStart(2, "0"))
    .join("");
}

// ─── noop-authoritative path ─────────────────────────────────────────

async function seedMint(
  store: ReturnType<typeof createCnftMemoryStore>,
): Promise<{ tree: Address; assetId: Address }> {
  const tree = "11111111111111111111111111111111" as Address;
  await applyEvent(store, { kind: "createTree", tree, depth: 8, maxBufferSize: 16 });
  await applyEvent(store, {
    kind: "mint",
    tree,
    owner: new Uint8Array(32).fill(1),
    delegate: new Uint8Array(32).fill(2),
    metadata: stubMintMetadata(),
    verifyCollection: null,
  });
  const assetId = await deriveAssetId(tree, 0n);
  return { tree, assetId };
}

test("apply: verifyCreator flips the creator's verified flag and adopts noop hashes", async () => {
  const store = createCnftMemoryStore();
  const { tree, assetId } = await seedMint(store);
  const creatorBytes = (await store.getLeaf(assetId))!.mintMetadata.creators[0]!.address;

  const newOwner = new Uint8Array(32).fill(1);
  const newDelegate = new Uint8Array(32).fill(2);
  const newDataHash = new Uint8Array(32).fill(0xaa);
  const newCreatorHash = new Uint8Array(32).fill(0xbb);

  await applyEvent(store, {
    kind: "verifyCreator",
    tree,
    creator: creatorBytes,
    noop: {
      leafIndex: 0n,
      nonce: 0n,
      owner: newOwner,
      delegate: newDelegate,
      dataHash: newDataHash,
      creatorHash: newCreatorHash,
    },
  });

  const rec = await store.getLeaf(assetId);
  assert.ok(rec);
  assert.equal(hex(rec!.dataHash), hex(newDataHash));
  assert.equal(hex(rec!.creatorHash), hex(newCreatorHash));
  assert.equal(rec!.mintMetadata.creators[0]!.verified, true,
    "the matching creator should now be verified");
});

test("apply: unverifyCreator flips it back to false", async () => {
  const store = createCnftMemoryStore();
  const { tree, assetId } = await seedMint(store);
  const creatorBytes = (await store.getLeaf(assetId))!.mintMetadata.creators[0]!.address;
  const noop = {
    leafIndex: 0n,
    nonce: 0n,
    owner: new Uint8Array(32).fill(1),
    delegate: new Uint8Array(32).fill(2),
    dataHash: new Uint8Array(32).fill(0xaa),
    creatorHash: new Uint8Array(32).fill(0xbb),
  };
  await applyEvent(store, { kind: "verifyCreator", tree, creator: creatorBytes, noop });
  await applyEvent(store, {
    kind: "unverifyCreator",
    tree,
    creator: creatorBytes,
    noop: { ...noop, dataHash: new Uint8Array(32).fill(0xcc), creatorHash: new Uint8Array(32).fill(0xdd) },
  });
  const rec = await store.getLeaf(assetId);
  assert.equal(rec!.mintMetadata.creators[0]!.verified, false);
  assert.equal(hex(rec!.dataHash), "cc".repeat(32));
});

test("apply: setAndVerifyCollection marks collection + updates leaf hashes", async () => {
  const store = createCnftMemoryStore();
  const { tree, assetId } = await seedMint(store);
  const collectionKey = new Uint8Array(32).fill(0x77);

  await applyEvent(store, {
    kind: "setAndVerifyCollection",
    tree,
    collection: collectionKey,
    noop: {
      leafIndex: 0n,
      nonce: 0n,
      owner: new Uint8Array(32).fill(1),
      delegate: new Uint8Array(32).fill(2),
      dataHash: new Uint8Array(32).fill(0xfe),
      creatorHash: new Uint8Array(32).fill(0xfd),
    },
  });

  const rec = await store.getLeaf(assetId);
  assert.ok(rec);
  assert.equal(rec!.mintMetadata.collection?.verified, true);
  assert.equal(hex(rec!.mintMetadata.collection!.key), "77".repeat(32));
  assert.equal(hex(rec!.dataHash), "fe".repeat(32));
});

test("apply: updateMetadata picks non-default fields from newMetadata, keeps prior for None", async () => {
  const store = createCnftMemoryStore();
  const { tree, assetId } = await seedMint(store);

  await applyEvent(store, {
    kind: "updateMetadata",
    tree,
    newMetadata: {
      name: "UpdatedName",
      symbol: "", // "not provided" sentinel — parser produces empty strings for None
      uri: "",
      sellerFeeBasisPoints: 0,
      primarySaleHappened: false,
      isMutable: true,
      creators: [],
      collection: null,
      dataHashInput: new Uint8Array(16),
    },
    noop: {
      leafIndex: 0n,
      nonce: 0n,
      owner: new Uint8Array(32).fill(1),
      delegate: new Uint8Array(32).fill(2),
      dataHash: new Uint8Array(32).fill(0x42),
      creatorHash: new Uint8Array(32).fill(0x43),
    },
  });

  const rec = await store.getLeaf(assetId);
  assert.equal(rec!.mintMetadata.name, "UpdatedName");
  assert.equal(rec!.mintMetadata.symbol, "AST",
    "None-for-symbol should preserve the prior mint value");
  assert.equal(hex(rec!.dataHash), "42".repeat(32));
});

test("apply: mint with noop override uses authoritative hashes over reconstruction", async () => {
  const store = createCnftMemoryStore();
  const tree = "11111111111111111111111111111111" as Address;
  await applyEvent(store, { kind: "createTree", tree, depth: 8, maxBufferSize: 16 });

  const authoritativeData = new Uint8Array(32).fill(0x1f);
  const authoritativeCreator = new Uint8Array(32).fill(0x2f);
  await applyEvent(store, {
    kind: "mint",
    tree,
    owner: new Uint8Array(32).fill(1),
    delegate: new Uint8Array(32).fill(2),
    metadata: stubMintMetadata(),
    verifyCollection: null,
    noop: {
      leafIndex: 0n,
      nonce: 0n,
      owner: new Uint8Array(32).fill(1),
      delegate: new Uint8Array(32).fill(2),
      dataHash: authoritativeData,
      creatorHash: authoritativeCreator,
    },
  });

  const assetId = await deriveAssetId(tree, 0n);
  const rec = await store.getLeaf(assetId);
  assert.equal(hex(rec!.dataHash), hex(authoritativeData));
  assert.equal(hex(rec!.creatorHash), hex(authoritativeCreator));
});
