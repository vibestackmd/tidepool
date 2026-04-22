// Indexer integration test. Drive the orchestrator against a fully
// synthetic upstream (no network, no Surfpool) and assert the resulting
// store state matches what the ix stream implies.

import { test } from "node:test";
import assert from "node:assert/strict";
import {
  getAddressDecoder,
  getAddressEncoder,
  getBase58Decoder,
  type Address,
} from "@solana/kit";
import {
  CREATE_TREE_DISCRIMINATOR,
  getCreateTreeInstructionDataEncoder,
} from "../../src/generated/bubblegum/instructions/createTree.js";
import {
  MINT_V1_DISCRIMINATOR,
  getMintV1InstructionDataEncoder,
} from "../../src/generated/bubblegum/instructions/mintV1.js";
import {
  BURN_DISCRIMINATOR,
  getBurnInstructionDataEncoder,
} from "../../src/generated/bubblegum/instructions/burn.js";
import { indexTree } from "../../src/cnft/indexer.js";
import { BUBBLEGUM_PROGRAM_ADDRESS } from "../../src/cnft/parser.js";
import { createCnftMemoryStore } from "../../src/cnft/store-memory.js";
import { createFixtureUpstream } from "../../src/fixtures.js";
import type { UpstreamClient } from "../../src/upstream.js";

const addrDecoder = getAddressDecoder();
const addrEncoder = getAddressEncoder();
const base58 = getBase58Decoder();

function addressOfByte(b: number): Address {
  return addrDecoder.decode(new Uint8Array(32).fill(b));
}

function encBase58(bytes: Uint8Array): string {
  return base58.decode(bytes);
}

const BG = BUBBLEGUM_PROGRAM_ADDRESS as unknown as string;

// Stub accounts. Only positions the parser inspects matter; others just
// need to be valid 32-byte addresses.
const TREE = addressOfByte(0x11);
const OWNER = addressOfByte(0x22);
const DELEGATE = addressOfByte(0x33);
const FILLER = addressOfByte(0x77);

// A mini "blockchain" for the indexer: signatures map 1:1 to txs, and
// we track insertion order. getSignaturesForAddress serves them newest-
// first (real RPC semantics); getTransaction serves the stored tx.
interface FixtureChain {
  append(signature: string, tx: unknown): void;
  upstream(): UpstreamClient;
}

function createFixtureChain(): FixtureChain {
  const sigs: { signature: string; slot: number; err: unknown }[] = [];
  const txs = new Map<string, unknown>();
  let slot = 1;

  return {
    append(signature, tx) {
      sigs.push({ signature, slot: slot++, err: null });
      txs.set(signature, tx);
    },
    upstream() {
      return createFixtureUpstream({
        rpcResponses: {
          getSignaturesForAddress: (params) => {
            const [, opts] = params as [string, { limit?: number; before?: string; until?: string }];
            // RPC returns newest-first. Slice between "before" and "until".
            let list = [...sigs].reverse();
            if (opts?.before) {
              const idx = list.findIndex((s) => s.signature === opts.before);
              if (idx >= 0) list = list.slice(idx + 1);
            }
            if (opts?.until) {
              const idx = list.findIndex((s) => s.signature === opts.until);
              if (idx >= 0) list = list.slice(0, idx);
            }
            const limit = opts?.limit ?? 1000;
            return list.slice(0, limit);
          },
          getTransaction: (params) => {
            const [signature] = params as [string, unknown];
            return txs.get(signature) ?? null;
          },
        },
      });
    },
  };
}

function bubblegumTx(ixData: Uint8Array, accounts: Address[]): unknown {
  return {
    meta: { err: null, innerInstructions: [] },
    transaction: {
      message: {
        accountKeys: [...accounts.map((a) => a as unknown as string), BG],
        instructions: [
          {
            programIdIndex: accounts.length, // BG lives at the end
            accounts: accounts.map((_, i) => i),
            data: encBase58(ixData),
          },
        ],
      },
    },
  };
}

function createTreeIxData(): Uint8Array {
  return new Uint8Array(
    getCreateTreeInstructionDataEncoder().encode({
      maxDepth: 10,
      maxBufferSize: 16,
      public: { __option: "Some", value: false },
    }),
  );
}

function mintV1IxData(): Uint8Array {
  return new Uint8Array(
    getMintV1InstructionDataEncoder().encode({
      message: {
        name: "X",
        symbol: "X",
        uri: "https://x",
        sellerFeeBasisPoints: 0,
        primarySaleHappened: false,
        isMutable: true,
        editionNonce: { __option: "None" } as const,
        tokenStandard: { __option: "None" } as const,
        collection: { __option: "None" } as const,
        uses: { __option: "None" } as const,
        tokenProgramVersion: "Original" as const,
        creators: [],
      },
    }),
  );
}

function burnIxData(leafIndex: bigint): Uint8Array {
  return new Uint8Array(
    getBurnInstructionDataEncoder().encode({
      root: new Uint8Array(32),
      dataHash: new Uint8Array(32),
      creatorHash: new Uint8Array(32),
      nonce: leafIndex,
      index: Number(leafIndex),
    }),
  );
}

test("indexer: single createTree tx populates the store", async () => {
  const chain = createFixtureChain();
  chain.append(
    "sig1",
    bubblegumTx(createTreeIxData(), [
      FILLER, // treeAuthority
      TREE,   // merkleTree
      FILLER, FILLER, FILLER, FILLER, FILLER,
    ]),
  );
  const store = createCnftMemoryStore();

  const result = await indexTree({ upstream: chain.upstream(), store }, TREE);

  assert.equal(result.processed, 1);
  assert.equal(result.applied, 1);
  assert.equal(result.skipped, 0);
  assert.equal(result.newestApplied, "sig1");

  const info = await store.getTree(TREE);
  assert.ok(info);
  assert.equal(info!.depth, 10);
});

test("indexer: createTree + two mints applies in chronological order", async () => {
  const chain = createFixtureChain();
  chain.append(
    "sig-create",
    bubblegumTx(createTreeIxData(), [FILLER, TREE, FILLER, FILLER, FILLER, FILLER, FILLER]),
  );
  chain.append(
    "sig-mint1",
    bubblegumTx(mintV1IxData(), [
      FILLER, OWNER, DELEGATE, TREE, FILLER, FILLER, FILLER, FILLER, FILLER,
    ]),
  );
  chain.append(
    "sig-mint2",
    bubblegumTx(mintV1IxData(), [
      FILLER, OWNER, DELEGATE, TREE, FILLER, FILLER, FILLER, FILLER, FILLER,
    ]),
  );
  const store = createCnftMemoryStore();

  const result = await indexTree({ upstream: chain.upstream(), store }, TREE);

  assert.equal(result.processed, 3);
  assert.equal(result.applied, 3);

  const info = await store.getTree(TREE);
  assert.equal(info!.numMinted, 2n);

  const leaves = await store.listLeaves(TREE);
  assert.equal(leaves.length, 2);
  assert.deepEqual(leaves.map((l) => l.leafIndex), [0n, 1n]);
});

test("indexer: incremental call resumes from the stored cursor", async () => {
  const chain = createFixtureChain();
  chain.append(
    "sig-create",
    bubblegumTx(createTreeIxData(), [FILLER, TREE, FILLER, FILLER, FILLER, FILLER, FILLER]),
  );
  chain.append(
    "sig-mint1",
    bubblegumTx(mintV1IxData(), [
      FILLER, OWNER, DELEGATE, TREE, FILLER, FILLER, FILLER, FILLER, FILLER,
    ]),
  );
  const store = createCnftMemoryStore();
  const upstream = chain.upstream();

  // First pass indexes both txs.
  const first = await indexTree({ upstream, store }, TREE);
  assert.equal(first.processed, 2);
  assert.equal(await store.getLastSignature(TREE), "sig-mint1");

  // New tx appears later — only that one should be processed next call.
  chain.append(
    "sig-mint2",
    bubblegumTx(mintV1IxData(), [
      FILLER, OWNER, DELEGATE, TREE, FILLER, FILLER, FILLER, FILLER, FILLER,
    ]),
  );
  const second = await indexTree({ upstream, store }, TREE);
  assert.equal(second.processed, 1, "should only process the new sig");
  assert.equal(second.newestApplied, "sig-mint2");

  const info = await store.getTree(TREE);
  assert.equal(info!.numMinted, 2n);
});

test("indexer: failed tx advances cursor but doesn't apply state", async () => {
  const chain = createFixtureChain();
  chain.append(
    "sig-create",
    bubblegumTx(createTreeIxData(), [FILLER, TREE, FILLER, FILLER, FILLER, FILLER, FILLER]),
  );
  // Synthesize a failed tx — indexer should advance past it.
  chain.upstream(); // warm the closure (no-op)
  const failedTx = {
    meta: { err: { InstructionError: [0, "Custom"] } },
    transaction: {
      message: {
        accountKeys: [BG],
        instructions: [{ programIdIndex: 0, accounts: [], data: encBase58(mintV1IxData()) }],
      },
    },
  };
  chain.append("sig-failed", failedTx);

  const store = createCnftMemoryStore();
  const result = await indexTree({ upstream: chain.upstream(), store }, TREE);

  assert.equal(result.processed, 2);
  assert.equal(result.applied, 1, "only createTree applies; failed mint doesn't");
  const info = await store.getTree(TREE);
  assert.equal(info!.numMinted, 0n, "no mint should have run");
  assert.equal(await store.getLastSignature(TREE), "sig-failed");
});

test("indexer: burn of an indexed leaf marks it burned", async () => {
  const chain = createFixtureChain();
  chain.append(
    "sig-create",
    bubblegumTx(createTreeIxData(), [FILLER, TREE, FILLER, FILLER, FILLER, FILLER, FILLER]),
  );
  chain.append(
    "sig-mint",
    bubblegumTx(mintV1IxData(), [
      FILLER, OWNER, DELEGATE, TREE, FILLER, FILLER, FILLER, FILLER, FILLER,
    ]),
  );
  chain.append(
    "sig-burn",
    bubblegumTx(burnIxData(0n), [FILLER, OWNER, DELEGATE, TREE, FILLER, FILLER, FILLER]),
  );

  const store = createCnftMemoryStore();
  const result = await indexTree({ upstream: chain.upstream(), store }, TREE);

  assert.equal(result.applied, 3);
  const leaf = await store.getLeafByIndex(TREE, 0n);
  assert.ok(leaf);
  assert.equal(leaf!.burned, true);
});

test("indexer: onEventApplied callback fires once per applied ix", async () => {
  const chain = createFixtureChain();
  chain.append(
    "sig-create",
    bubblegumTx(createTreeIxData(), [FILLER, TREE, FILLER, FILLER, FILLER, FILLER, FILLER]),
  );
  chain.append(
    "sig-mint",
    bubblegumTx(mintV1IxData(), [
      FILLER, OWNER, DELEGATE, TREE, FILLER, FILLER, FILLER, FILLER, FILLER,
    ]),
  );
  const store = createCnftMemoryStore();
  const applied: string[] = [];
  await indexTree({ upstream: chain.upstream(), store }, TREE, {
    onEventApplied: (sig) => applied.push(sig),
  });
  assert.deepEqual(applied, ["sig-create", "sig-mint"]);
});

test("indexer: empty chain is a no-op", async () => {
  const chain = createFixtureChain();
  const store = createCnftMemoryStore();
  const result = await indexTree({ upstream: chain.upstream(), store }, TREE);
  assert.equal(result.processed, 0);
  assert.equal(result.applied, 0);
  assert.equal(result.newestApplied, null);
});

// Silence unused-import warning for the encoder helper — we use it
// only inside test helpers above.
void addrEncoder;
