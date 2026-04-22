// End-to-end handler tests. Drive cNFT state into the context's store
// via applyEvent (bypassing the indexer — tested separately in
// indexer.test.ts), then dispatch JSON-RPC requests through
// handleJsonRpcBody and verify response shape.

import { test } from "node:test";
import assert from "node:assert/strict";
import {
  getAddressDecoder,
  getBase58Decoder,
  getBase58Encoder,
  type Address,
} from "@solana/kit";
import {
  applyEvent,
  deriveAssetId,
  verifyProof,
} from "../../src/cnft/index.js";
import {
  createFixtureUpstream,
  createHeliusContext,
  handleJsonRpcBody,
} from "../../src/index.js";

const addrDecoder = getAddressDecoder();
const base58Encoder = getBase58Encoder();
// getBase58Decoder() is bytes→string; no direct use in this file but
// referenced through assertion paths below. Kept imported for symmetry.
void getBase58Decoder;

function addressOfByte(b: number): Address {
  return addrDecoder.decode(new Uint8Array(32).fill(b));
}

function b58ToBytes(s: string): Uint8Array {
  return base58Encoder.encode(s) as Uint8Array;
}

async function seedTreeWithOneMint(ctx: import("../../src/index.js").RequestContext) {
  const tree = addressOfByte(0x11);
  const owner = new Uint8Array(32).fill(0x22);
  const delegate = new Uint8Array(32).fill(0x33);
  await applyEvent(ctx.cnft, {
    kind: "createTree",
    tree,
    depth: 6,
    maxBufferSize: 8,
  });
  await applyEvent(ctx.cnft, {
    kind: "mint",
    tree,
    owner,
    delegate,
    metadata: {
      name: "Compressed",
      symbol: "CMP",
      uri: "https://example.com/cnft.json",
      sellerFeeBasisPoints: 250,
      primarySaleHappened: false,
      isMutable: true,
      creators: [
        { address: new Uint8Array(32).fill(0x44), verified: false, share: 100 },
      ],
      collection: null,
      dataHashInput: new TextEncoder().encode('{"name":"Compressed"}'),
    },
    verifyCollection: null,
  });
  return { tree, assetId: await deriveAssetId(tree, 0n) };
}

test("handlers: getAsset returns a cNFT shape when the asset is indexed", async () => {
  const ctx = createHeliusContext({ upstream: createFixtureUpstream() });
  const { tree, assetId } = await seedTreeWithOneMint(ctx);

  const resp = await handleJsonRpcBody(ctx, {
    method: "getAsset",
    id: 1,
    params: { id: assetId as string },
  });
  assert.ok(resp);
  const r = resp as { result: Record<string, unknown>; id: unknown };
  assert.equal(r.id, 1);
  const asset = r.result as Record<string, unknown> & {
    compression?: Record<string, unknown>;
    ownership?: Record<string, unknown>;
  };
  assert.equal(asset.id, assetId as string);
  assert.equal(asset.interface, "V1_NFT");
  assert.ok(asset.compression, "compression block should exist");
  assert.equal(asset.compression!.compressed, true);
  assert.equal(asset.compression!.eligible, true);
  assert.equal(asset.compression!.tree, tree as string);
  assert.equal(asset.compression!.leaf_id, 0);
  assert.equal(typeof asset.compression!.data_hash, "string");
  assert.equal(typeof asset.compression!.creator_hash, "string");
  assert.equal(typeof asset.compression!.asset_hash, "string");
  assert.equal((asset.ownership as { delegated: boolean }).delegated, true,
    "owner (0x22…) != delegate (0x33…) implies delegated");
});

test("handlers: getAsset falls through to 'not found' for an id that's not in the cNFT store and has no on-chain account", async () => {
  const ctx = createHeliusContext({ upstream: createFixtureUpstream() });
  // Fixture upstream defaults getAccountInfo to null for unknown
  // addresses, which the existing get-asset path surfaces as an error.
  const resp = await handleJsonRpcBody(ctx, {
    method: "getAsset",
    id: 7,
    params: { id: addressOfByte(0xee) as string },
  });
  assert.ok(resp);
  const r = resp as { error?: { code: number; message: string } };
  assert.ok(r.error);
  assert.equal(r.error!.code, -32000);
});

test("handlers: getAssetProof returns a proof that verifies against the computed root", async () => {
  const ctx = createHeliusContext({ upstream: createFixtureUpstream() });
  const { tree, assetId } = await seedTreeWithOneMint(ctx);

  const resp = await handleJsonRpcBody(ctx, {
    method: "getAssetProof",
    id: 2,
    params: { id: assetId as string },
  });
  assert.ok(resp);
  const r = resp as {
    result: { root: string; proof: string[]; node_index: number; leaf: string; tree_id: string };
  };
  assert.equal(r.result.tree_id, tree as string);
  assert.equal(r.result.node_index, 64 + 0); // 2^6 + 0 for a depth-6 tree
  assert.equal(r.result.proof.length, 6);

  // Cross-check: the returned proof + leaf must verify against the
  // returned root under leafIndex 0.
  assert.ok(
    verifyProof({
      leaf: b58ToBytes(r.result.leaf),
      proof: r.result.proof.map(b58ToBytes),
      leafIndex: 0n,
      root: b58ToBytes(r.result.root),
    }),
    "returned proof should verify",
  );
});

test("handlers: getAssetProof returns an error when the asset is not indexed", async () => {
  const ctx = createHeliusContext({ upstream: createFixtureUpstream() });
  const resp = await handleJsonRpcBody(ctx, {
    method: "getAssetProof",
    id: 3,
    params: { id: addressOfByte(0x99) as string },
  });
  const r = resp as { error?: { code: number; message: string } };
  assert.ok(r?.error);
  assert.match(r.error!.message, /not found/);
});

test("handlers: getAssetProofBatch returns a keyed map with null for misses", async () => {
  const ctx = createHeliusContext({ upstream: createFixtureUpstream() });
  const { assetId } = await seedTreeWithOneMint(ctx);
  const unknownId = addressOfByte(0xaa) as string;

  const resp = await handleJsonRpcBody(ctx, {
    method: "getAssetProofBatch",
    id: 4,
    params: { ids: [assetId as string, unknownId] },
  });
  const r = resp as { result: Record<string, unknown> };
  assert.equal(Object.keys(r.result).length, 2);
  assert.ok(r.result[assetId as string], "known id should have a proof");
  assert.equal(r.result[unknownId], null, "unknown id should be null");
});

test("handlers: surfpoolHeliusIndexTree with missing param returns invalid-params error", async () => {
  const ctx = createHeliusContext({ upstream: createFixtureUpstream() });
  const resp = await handleJsonRpcBody(ctx, {
    method: "surfpoolHeliusIndexTree",
    id: 5,
    params: {},
  });
  const r = resp as { error?: { code: number; message: string } };
  assert.ok(r?.error);
  assert.equal(r.error!.code, -32602);
});

test("handlers: surfpoolHeliusInfo lists getAssetProof as LOCAL_INDEX", async () => {
  const ctx = createHeliusContext({ upstream: createFixtureUpstream() });
  const resp = await handleJsonRpcBody(ctx, {
    method: "surfpoolHeliusInfo",
    id: 6,
    params: [],
  });
  const r = resp as { result: { methods: Array<{ method: string; compat: string }> } };
  const entry = r.result.methods.find((m) => m.method === "getAssetProof");
  assert.ok(entry);
  assert.equal(entry!.compat, "LOCAL_INDEX");
});
