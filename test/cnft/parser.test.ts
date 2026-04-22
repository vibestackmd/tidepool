// Parser tests. Strategy: synthesize valid ix data using the generated
// Codama encoders, pass it through our parser, and assert the resulting
// CnftEvent shape. Accounts are stub addresses — the parser doesn't
// validate them on-chain, only uses them positionally.

import { test } from "node:test";
import assert from "node:assert/strict";
import { getAddressDecoder, type Address } from "@solana/kit";
import {
  BURN_DISCRIMINATOR,
  getBurnInstructionDataEncoder,
} from "../../src/generated/bubblegum/instructions/burn.js";
import {
  CREATE_TREE_DISCRIMINATOR,
  getCreateTreeInstructionDataEncoder,
} from "../../src/generated/bubblegum/instructions/createTree.js";
import {
  DELEGATE_DISCRIMINATOR,
  getDelegateInstructionDataEncoder,
} from "../../src/generated/bubblegum/instructions/delegate.js";
import {
  MINT_V1_DISCRIMINATOR,
  getMintV1InstructionDataEncoder,
} from "../../src/generated/bubblegum/instructions/mintV1.js";
import {
  MINT_TO_COLLECTION_V1_DISCRIMINATOR,
  getMintToCollectionV1InstructionDataEncoder,
} from "../../src/generated/bubblegum/instructions/mintToCollectionV1.js";
import {
  TRANSFER_DISCRIMINATOR,
  getTransferInstructionDataEncoder,
} from "../../src/generated/bubblegum/instructions/transfer.js";
import { getVerifyCreatorInstructionDataEncoder } from "../../src/generated/bubblegum/instructions/verifyCreator.js";
import { getVerifyCollectionInstructionDataEncoder } from "../../src/generated/bubblegum/instructions/verifyCollection.js";
import { getUpdateMetadataInstructionDataEncoder } from "../../src/generated/bubblegum/instructions/updateMetadata.js";
import { parseBubblegumInstruction } from "../../src/cnft/parser.js";
import type {
  CnftEvent,
  LeafSchemaEventDecoded,
} from "../../src/cnft/index.js";

// Stub addresses derived from raw 32-byte buffers. Each byte is unique
// enough that the encoded base58 is distinct across stubs. Using the
// Address decoder guarantees the result is valid — `"22222222…"` style
// short strings only decode to ~23 bytes, which Kit rejects when the
// address ends up inside a MetadataArgs or account meta.
const addrDecoder = getAddressDecoder();
function addressOfByte(b: number): Address {
  return addrDecoder.decode(new Uint8Array(32).fill(b));
}
const TREE = addressOfByte(0x11);
const OWNER = addressOfByte(0x22);
const DELEGATE = addressOfByte(0x33);
const NEW_OWNER = addressOfByte(0x44);
const NEW_DELEGATE = addressOfByte(0x55);
const COLLECTION_MINT = addressOfByte(0x66);
const FILLER = addressOfByte(0x77);
// Positional filler for account slots the parser never reads.
const A = (_n: number): Address => FILLER;

function concat(disc: Uint8Array, payload: Uint8Array): Uint8Array {
  const out = new Uint8Array(disc.length + payload.length);
  out.set(disc, 0);
  out.set(payload, disc.length);
  return out;
}

test("parser: createTree yields a createTree event with depth + buffer size", () => {
  const data = getCreateTreeInstructionDataEncoder().encode({
    maxDepth: 20,
    maxBufferSize: 64,
    public: { __option: "Some", value: false },
  });

  const accounts: Address[] = [
    A(0), // treeAuthority
    TREE, // merkleTree
    A(2), // payer
    A(3), // treeCreator
    A(4), // logWrapper
    A(5), // compressionProgram
    A(6), // systemProgram
  ];
  const res = parseBubblegumInstruction({ data: new Uint8Array(data), accounts });
  assert.ok(res.ok);
  const event = res.value as CnftEvent;
  assert.equal(event.kind, "createTree");
  if (event.kind !== "createTree") throw new Error("type narrowing");
  assert.equal(event.tree, TREE);
  assert.equal(event.depth, 20);
  assert.equal(event.maxBufferSize, 64);
});

test("parser: mintV1 yields a mint event with owner/delegate/metadata", () => {
  const metadataArgs = {
    name: "Test",
    symbol: "TST",
    uri: "https://example.com/t.json",
    sellerFeeBasisPoints: 500,
    primarySaleHappened: false,
    isMutable: true,
    editionNonce: { __option: "None" } as const,
    tokenStandard: { __option: "None" } as const,
    collection: { __option: "None" } as const,
    uses: { __option: "None" } as const,
    tokenProgramVersion: "Original" as const,
    creators: [
      { address: addressOfByte(0x99), verified: false, share: 100 },
    ],
  };
  const data = getMintV1InstructionDataEncoder().encode({ message: metadataArgs });
  const accounts: Address[] = [
    A(0), // treeAuthority
    OWNER,
    DELEGATE,
    TREE,
    A(4), A(5), A(6), A(7), A(8),
  ];
  const res = parseBubblegumInstruction({ data: new Uint8Array(data), accounts });
  assert.ok(res.ok);
  const event = res.value as CnftEvent;
  assert.equal(event.kind, "mint");
  if (event.kind !== "mint") throw new Error("type narrowing");
  assert.equal(event.tree, TREE);
  assert.equal(event.metadata.name, "Test");
  assert.equal(event.metadata.creators.length, 1);
  assert.equal(event.metadata.creators[0]!.share, 100);
  assert.equal(event.verifyCollection, null);
});

test("parser: mintToCollectionV1 marks verifyCollection with the collectionMint address", () => {
  const metadataArgs = {
    name: "Coll",
    symbol: "CLL",
    uri: "https://example.com/c.json",
    sellerFeeBasisPoints: 0,
    primarySaleHappened: false,
    isMutable: true,
    editionNonce: { __option: "None" } as const,
    tokenStandard: { __option: "None" } as const,
    // Collection args here are what the caller sent; Bubblegum flips
    // verified to true as part of the ix. Our parser reproduces that.
    collection: {
      __option: "Some",
      value: { key: COLLECTION_MINT, verified: false },
    } as const,
    uses: { __option: "None" } as const,
    tokenProgramVersion: "Original" as const,
    creators: [],
  };
  const data = getMintToCollectionV1InstructionDataEncoder().encode({
    metadataArgs,
  });
  const accounts: Address[] = [
    A(0),          // treeAuthority
    OWNER,         // leafOwner
    DELEGATE,      // leafDelegate
    TREE,          // merkleTree
    A(4), A(5), A(6), A(7),
    COLLECTION_MINT, // collectionMint — position 8
    A(9), A(10), A(11), A(12), A(13), A(14), A(15),
  ];
  const res = parseBubblegumInstruction({ data: new Uint8Array(data), accounts });
  assert.ok(res.ok);
  const event = res.value as CnftEvent;
  assert.equal(event.kind, "mint");
  if (event.kind !== "mint") throw new Error("type narrowing");
  assert.ok(event.verifyCollection);
  assert.equal(event.metadata.collection?.verified, true);
});

test("parser: transfer yields newOwner/newDelegate reset to newOwner", () => {
  const payload = getTransferInstructionDataEncoder().encode({
    root: new Uint8Array(32).fill(1),
    dataHash: new Uint8Array(32).fill(2),
    creatorHash: new Uint8Array(32).fill(3),
    nonce: 7n,
    index: 7,
  });
  const data = concat(TRANSFER_DISCRIMINATOR, new Uint8Array(0)); // encoder includes discriminator
  // Codama's InstructionDataEncoder already prepends the discriminator,
  // so we use `payload` directly. Guard: the encoded bytes must start
  // with the same discriminator.
  const raw = new Uint8Array(payload);
  assert.ok(bufStartsWith(raw, TRANSFER_DISCRIMINATOR));
  void data;

  const accounts: Address[] = [
    A(0),     // treeAuthority
    OWNER,    // leafOwner
    DELEGATE, // leafDelegate
    NEW_OWNER,// newLeafOwner
    TREE,     // merkleTree
    A(5), A(6), A(7),
  ];
  const res = parseBubblegumInstruction({ data: raw, accounts });
  assert.ok(res.ok);
  const event = res.value as CnftEvent;
  assert.equal(event.kind, "transfer");
  if (event.kind !== "transfer") throw new Error("type narrowing");
  assert.equal(event.leafIndex, 7n);
  assert.equal(event.nonce, 7n);
  // newDelegate should equal newOwner (Bubblegum transfer semantics).
  assert.equal(hex(event.newOwner), hex(event.newDelegate));
  assert.equal(event.dataHash[0], 2);
  assert.equal(event.creatorHash[0], 3);
});

test("parser: burn yields (tree, leafIndex, nonce)", () => {
  const payload = new Uint8Array(
    getBurnInstructionDataEncoder().encode({
      root: new Uint8Array(32),
      dataHash: new Uint8Array(32),
      creatorHash: new Uint8Array(32),
      nonce: 42n,
      index: 42,
    }),
  );
  assert.ok(bufStartsWith(payload, BURN_DISCRIMINATOR));

  const accounts: Address[] = [
    A(0), OWNER, DELEGATE, TREE, A(4), A(5), A(6),
  ];
  const res = parseBubblegumInstruction({ data: payload, accounts });
  assert.ok(res.ok);
  const event = res.value as CnftEvent;
  assert.equal(event.kind, "burn");
  if (event.kind !== "burn") throw new Error("type narrowing");
  assert.equal(event.tree, TREE);
  assert.equal(event.leafIndex, 42n);
  assert.equal(event.nonce, 42n);
});

test("parser: delegate yields new delegate from accounts", () => {
  const payload = new Uint8Array(
    getDelegateInstructionDataEncoder().encode({
      root: new Uint8Array(32),
      dataHash: new Uint8Array(32).fill(0xaa),
      creatorHash: new Uint8Array(32).fill(0xbb),
      nonce: 3n,
      index: 3,
    }),
  );
  assert.ok(bufStartsWith(payload, DELEGATE_DISCRIMINATOR));

  const accounts: Address[] = [
    A(0),         // treeAuthority
    OWNER,        // leafOwner
    DELEGATE,     // previousLeafDelegate
    NEW_DELEGATE, // newLeafDelegate
    TREE,         // merkleTree
    A(5), A(6), A(7),
  ];
  const res = parseBubblegumInstruction({ data: payload, accounts });
  assert.ok(res.ok);
  const event = res.value as CnftEvent;
  assert.equal(event.kind, "delegate");
  if (event.kind !== "delegate") throw new Error("type narrowing");
  assert.equal(event.tree, TREE);
  assert.equal(event.leafIndex, 3n);
  assert.equal(hex(event.dataHash), "aa".repeat(32));
  assert.equal(hex(event.creatorHash), "bb".repeat(32));
});

test("parser: unknown discriminator returns unknown-discriminator", () => {
  const data = new Uint8Array([99, 99, 99, 99, 99, 99, 99, 99]);
  const res = parseBubblegumInstruction({ data, accounts: [] });
  assert.ok(!res.ok);
  assert.equal(res.error.kind, "unknown-discriminator");
});

test("parser: truncated data returns truncated-data", () => {
  const res = parseBubblegumInstruction({
    data: new Uint8Array([1, 2, 3]),
    accounts: [],
  });
  assert.ok(!res.ok);
  assert.equal(res.error.kind, "truncated-data");
});

test("parser: insufficient accounts for a known ix returns insufficient-accounts", () => {
  const payload = new Uint8Array(
    getBurnInstructionDataEncoder().encode({
      root: new Uint8Array(32),
      dataHash: new Uint8Array(32),
      creatorHash: new Uint8Array(32),
      nonce: 0n,
      index: 0,
    }),
  );
  const res = parseBubblegumInstruction({
    data: payload,
    accounts: [A(0), A(1)], // too few
  });
  assert.ok(!res.ok);
  assert.equal(res.error.kind, "insufficient-accounts");
});

test("parser: CREATE_TREE and MINT_V1 discriminators are distinct (smoke)", () => {
  assert.notEqual(hex(CREATE_TREE_DISCRIMINATOR), hex(MINT_V1_DISCRIMINATOR));
  assert.notEqual(hex(MINT_V1_DISCRIMINATOR), hex(MINT_TO_COLLECTION_V1_DISCRIMINATOR));
});

// ─── noop-required ixs ──────────────────────────────────────────────

function fakeNoop(args?: Partial<{
  owner: Uint8Array;
  delegate: Uint8Array;
  dataHash: Uint8Array;
  creatorHash: Uint8Array;
  nonce: bigint;
}>): LeafSchemaEventDecoded {
  return {
    op: "Transfer",
    schema: {
      kind: "V1",
      id: new Uint8Array(32).fill(0x99),
      owner: args?.owner ?? new Uint8Array(32).fill(0xaa),
      delegate: args?.delegate ?? new Uint8Array(32).fill(0xbb),
      nonce: args?.nonce ?? 0n,
      dataHash: args?.dataHash ?? new Uint8Array(32).fill(0xdd),
      creatorHash: args?.creatorHash ?? new Uint8Array(32).fill(0xee),
    },
  };
}

test("parser: verifyCreator without noop event returns unsupported", () => {
  const payload = new Uint8Array(
    getVerifyCreatorInstructionDataEncoder().encode({
      root: new Uint8Array(32),
      dataHash: new Uint8Array(32),
      creatorHash: new Uint8Array(32),
      nonce: 0n,
      index: 0,
      message: {
        name: "N",
        symbol: "S",
        uri: "u",
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
  const accounts: Address[] = [
    FILLER, OWNER, DELEGATE, TREE, FILLER,
    addressOfByte(0x99), // creator
    FILLER, FILLER, FILLER,
  ];
  const res = parseBubblegumInstruction({ data: payload, accounts });
  assert.ok(!res.ok);
  assert.equal(res.error.kind, "unsupported");
});

test("parser: verifyCreator with noop event returns a verifyCreator event", () => {
  const payload = new Uint8Array(
    getVerifyCreatorInstructionDataEncoder().encode({
      root: new Uint8Array(32),
      dataHash: new Uint8Array(32),
      creatorHash: new Uint8Array(32),
      nonce: 5n,
      index: 5,
      message: {
        name: "N",
        symbol: "S",
        uri: "u",
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
  const accounts: Address[] = [
    FILLER, OWNER, DELEGATE, TREE, FILLER,
    addressOfByte(0x99), // creator at position 5
    FILLER, FILLER, FILLER,
  ];
  const noop = fakeNoop({ nonce: 5n });
  const res = parseBubblegumInstruction({ data: payload, accounts, noopEvent: noop });
  assert.ok(res.ok);
  const event = res.value as CnftEvent;
  assert.equal(event.kind, "verifyCreator");
  if (event.kind !== "verifyCreator") throw new Error("narrow");
  assert.equal(event.tree, TREE);
  assert.equal(hex(event.creator), "99".repeat(32));
  assert.equal(event.noop.nonce, 5n);
});

test("parser: verifyCollection with noop event carries collection mint + new hashes", () => {
  const payload = new Uint8Array(
    getVerifyCollectionInstructionDataEncoder().encode({
      root: new Uint8Array(32),
      dataHash: new Uint8Array(32),
      creatorHash: new Uint8Array(32),
      nonce: 7n,
      index: 7,
      message: {
        name: "N",
        symbol: "S",
        uri: "u",
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
  const accounts: Address[] = [
    FILLER, OWNER, DELEGATE, TREE, FILLER, FILLER, FILLER, FILLER,
    COLLECTION_MINT, // position 8
    FILLER, FILLER, FILLER, FILLER, FILLER, FILLER, FILLER,
  ];
  const noop = fakeNoop({
    nonce: 7n,
    dataHash: new Uint8Array(32).fill(0xfe),
    creatorHash: new Uint8Array(32).fill(0xfd),
  });
  const res = parseBubblegumInstruction({ data: payload, accounts, noopEvent: noop });
  assert.ok(res.ok);
  const event = res.value as CnftEvent;
  assert.equal(event.kind, "verifyCollection");
  if (event.kind !== "verifyCollection") throw new Error("narrow");
  assert.equal(hex(event.collection), "66".repeat(32));
  assert.equal(hex(event.noop.dataHash), "fe".repeat(32));
  assert.equal(hex(event.noop.creatorHash), "fd".repeat(32));
});

test("parser: updateMetadata with noop event carries new metadata preimage", () => {
  const payload = new Uint8Array(
    getUpdateMetadataInstructionDataEncoder().encode({
      root: new Uint8Array(32),
      nonce: 2n,
      index: 2,
      currentMetadata: {
        name: "Old",
        symbol: "O",
        uri: "u",
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
      updateArgs: {
        name: { __option: "Some", value: "New" } as const,
        symbol: { __option: "None" } as const,
        uri: { __option: "None" } as const,
        creators: { __option: "None" } as const,
        sellerFeeBasisPoints: { __option: "None" } as const,
        primarySaleHappened: { __option: "None" } as const,
        isMutable: { __option: "None" } as const,
      },
    }),
  );
  const accounts: Address[] = [
    FILLER, FILLER, FILLER, FILLER, FILLER,
    OWNER, DELEGATE, FILLER,
    TREE, // position 8
    FILLER, FILLER, FILLER, FILLER,
  ];
  const noop = fakeNoop({
    nonce: 2n,
    dataHash: new Uint8Array(32).fill(0x11),
  });
  const res = parseBubblegumInstruction({ data: payload, accounts, noopEvent: noop });
  assert.ok(res.ok);
  const event = res.value as CnftEvent;
  assert.equal(event.kind, "updateMetadata");
  if (event.kind !== "updateMetadata") throw new Error("narrow");
  assert.equal(event.newMetadata.name, "New");
  assert.equal(hex(event.noop.dataHash), "11".repeat(32));
});

function bufStartsWith(buf: Uint8Array, prefix: Uint8Array): boolean {
  if (buf.length < prefix.length) return false;
  for (let i = 0; i < prefix.length; i++) {
    if (buf[i] !== prefix[i]) return false;
  }
  return true;
}

function hex(bytes: Uint8Array): string {
  return Array.from(bytes)
    .map((b) => b.toString(16).padStart(2, "0"))
    .join("");
}
