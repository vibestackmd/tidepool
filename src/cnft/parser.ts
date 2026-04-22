// Bubblegum instruction parser. Pure function: given raw ix data + the
// ix's account list, emit a CnftEvent (or indicate the ix isn't one we
// track). No I/O, no async, no references to the store.
//
// Scope: state-changing ixs we can reconstruct from the outer ix args +
// accounts alone. The Metaplex reference DAS indexer also parses the
// noop CPI (which carries a LeafSchemaEvent) for authoritative new-state
// hashes, which is how it handles verifyCreator / updateMetadata — those
// can't be reconstructed from ix args without prior state. We defer
// those to a follow-up step; this parser covers the happy-path flows
// (createTree, mint, transfer, burn, delegate) that are sufficient for
// the majority of mint-and-query workflows.

import type { Address } from "@solana/kit";
import {
  BURN_DISCRIMINATOR,
  getBurnInstructionDataDecoder,
} from "../generated/bubblegum/instructions/burn.js";
import {
  CREATE_TREE_DISCRIMINATOR,
  getCreateTreeInstructionDataDecoder,
} from "../generated/bubblegum/instructions/createTree.js";
import {
  DELEGATE_DISCRIMINATOR,
  getDelegateInstructionDataDecoder,
} from "../generated/bubblegum/instructions/delegate.js";
import {
  MINT_TO_COLLECTION_V1_DISCRIMINATOR,
  getMintToCollectionV1InstructionDataDecoder,
} from "../generated/bubblegum/instructions/mintToCollectionV1.js";
import {
  MINT_V1_DISCRIMINATOR,
  getMintV1InstructionDataDecoder,
} from "../generated/bubblegum/instructions/mintV1.js";
import {
  TRANSFER_DISCRIMINATOR,
  getTransferInstructionDataDecoder,
} from "../generated/bubblegum/instructions/transfer.js";
import {
  VERIFY_CREATOR_DISCRIMINATOR,
  getVerifyCreatorInstructionDataDecoder,
} from "../generated/bubblegum/instructions/verifyCreator.js";
import {
  UNVERIFY_CREATOR_DISCRIMINATOR,
  getUnverifyCreatorInstructionDataDecoder,
} from "../generated/bubblegum/instructions/unverifyCreator.js";
import {
  VERIFY_COLLECTION_DISCRIMINATOR,
  getVerifyCollectionInstructionDataDecoder,
} from "../generated/bubblegum/instructions/verifyCollection.js";
import {
  UNVERIFY_COLLECTION_DISCRIMINATOR,
  getUnverifyCollectionInstructionDataDecoder,
} from "../generated/bubblegum/instructions/unverifyCollection.js";
import {
  SET_AND_VERIFY_COLLECTION_DISCRIMINATOR,
  getSetAndVerifyCollectionInstructionDataDecoder,
} from "../generated/bubblegum/instructions/setAndVerifyCollection.js";
import {
  UPDATE_METADATA_DISCRIMINATOR,
  getUpdateMetadataInstructionDataDecoder,
} from "../generated/bubblegum/instructions/updateMetadata.js";
import { BUBBLEGUM_PROGRAM_ADDRESS } from "../generated/bubblegum/programs/index.js";
import type { MetadataArgs } from "../generated/bubblegum/types/metadataArgs.js";
import type { LeafSchemaEventDecoded } from "./leaf-event.js";
import type {
  CnftEvent,
  Creator,
  MintMetadata,
  NoopOverride,
  ParseError,
  ParseResult,
} from "./types.js";

// Re-export so callers don't have to reach into the generated tree.
export { BUBBLEGUM_PROGRAM_ADDRESS };

// Account positions per ix, pulled from Codama-generated `accounts: [...]`
// builder arrays. Positions here stay in lockstep with the pinned IDL —
// if the IDL changes account ordering, these need to be regenerated
// alongside the client. (If that ever happens the roundtrip tests in
// test/cnft/parser.test.ts fail loudly.)
const ACC = {
  createTree: { merkleTree: 1 },
  mintV1: { leafOwner: 1, leafDelegate: 2, merkleTree: 3 },
  mintToCollectionV1: {
    leafOwner: 1,
    leafDelegate: 2,
    merkleTree: 3,
    collectionMint: 8,
  },
  transfer: {
    leafOwner: 1,
    leafDelegate: 2,
    newLeafOwner: 3,
    merkleTree: 4,
  },
  burn: { leafOwner: 1, leafDelegate: 2, merkleTree: 3 },
  delegate: {
    leafOwner: 1,
    previousLeafDelegate: 2,
    newLeafDelegate: 3,
    merkleTree: 4,
  },
  // Verify/unverify ixs put `creator` as a signer account; position
  // comes from the Codama-generated `accounts: [...]` builder array.
  verifyCreator: { merkleTree: 3, creator: 5 },
  unverifyCreator: { merkleTree: 3, creator: 5 },
  verifyCollection: { merkleTree: 3, collectionMint: 8 },
  unverifyCollection: { merkleTree: 3, collectionMint: 8 },
  setAndVerifyCollection: { merkleTree: 3, collectionMint: 8 },
  // updateMetadata reorders accounts — leafOwner/leafDelegate/merkleTree
  // sit after the collection block (which is always present even when
  // not updating collection membership).
  updateMetadata: { merkleTree: 8 },
} as const;

// Minimum account count for each ix. The parser refuses to proceed if
// fewer accounts were supplied (corrupt tx, truncated record, etc).
const MIN_ACCOUNTS = {
  createTree: 7,
  mintV1: 9,
  mintToCollectionV1: 16,
  transfer: 8,
  burn: 7,
  delegate: 8,
  verifyCreator: 9,
  unverifyCreator: 9,
  verifyCollection: 16,
  unverifyCollection: 16,
  setAndVerifyCollection: 16,
  updateMetadata: 13,
} as const;

/**
 * Entry point. `data` must start with the 8-byte ix discriminator; `accounts`
 * is the list of Address in IDL order (same order the on-chain ix was built
 * with). `noopEvent` is the LeafSchemaEvent emitted as an inner CPI by this
 * ix, if any — required for ixs whose new state can't be reconstructed
 * from outer-ix args alone (verify*, updateMetadata), and used as an
 * authoritative override on the happy-path ixs where it's available.
 *
 * Returns:
 *   - { ok: true, value: CnftEvent }      — a tracked state transition
 *   - { ok: true, value: null }           — a Bubblegum ix we ignore by design
 *   - { ok: false, error: ParseError }    — malformed or unknown-to-us ix data
 */
export function parseBubblegumInstruction(input: {
  data: Uint8Array;
  accounts: Address[];
  noopEvent?: LeafSchemaEventDecoded;
}): ParseResult<CnftEvent | null, ParseError> {
  const { data, accounts, noopEvent } = input;
  if (data.length < 8) {
    return err({ kind: "truncated-data", expected: 8, actual: data.length });
  }
  const discriminator = data.slice(0, 8);

  if (eq(discriminator, CREATE_TREE_DISCRIMINATOR)) {
    return parseCreateTree(data, accounts);
  }
  if (eq(discriminator, MINT_V1_DISCRIMINATOR)) {
    return parseMintV1(data, accounts, noopEvent);
  }
  if (eq(discriminator, MINT_TO_COLLECTION_V1_DISCRIMINATOR)) {
    return parseMintToCollectionV1(data, accounts, noopEvent);
  }
  if (eq(discriminator, TRANSFER_DISCRIMINATOR)) {
    return parseTransfer(data, accounts, noopEvent);
  }
  if (eq(discriminator, BURN_DISCRIMINATOR)) {
    return parseBurn(data, accounts, noopEvent);
  }
  if (eq(discriminator, DELEGATE_DISCRIMINATOR)) {
    return parseDelegate(data, accounts, noopEvent);
  }
  if (eq(discriminator, VERIFY_CREATOR_DISCRIMINATOR)) {
    return parseVerifyCreator(data, accounts, noopEvent);
  }
  if (eq(discriminator, UNVERIFY_CREATOR_DISCRIMINATOR)) {
    return parseUnverifyCreator(data, accounts, noopEvent);
  }
  if (eq(discriminator, VERIFY_COLLECTION_DISCRIMINATOR)) {
    return parseVerifyCollection(data, accounts, noopEvent);
  }
  if (eq(discriminator, UNVERIFY_COLLECTION_DISCRIMINATOR)) {
    return parseUnverifyCollection(data, accounts, noopEvent);
  }
  if (eq(discriminator, SET_AND_VERIFY_COLLECTION_DISCRIMINATOR)) {
    return parseSetAndVerifyCollection(data, accounts, noopEvent);
  }
  if (eq(discriminator, UPDATE_METADATA_DISCRIMINATOR)) {
    return parseUpdateMetadata(data, accounts, noopEvent);
  }
  return err({ kind: "unknown-discriminator", discriminator });
}

// ─── per-ix parsers ──────────────────────────────────────────────────

function parseCreateTree(
  data: Uint8Array,
  accounts: Address[],
): ParseResult<CnftEvent, ParseError> {
  if (accounts.length < MIN_ACCOUNTS.createTree) {
    return err({
      kind: "insufficient-accounts",
      expected: MIN_ACCOUNTS.createTree,
      actual: accounts.length,
    });
  }
  const decoded = tryDecode(() => getCreateTreeInstructionDataDecoder().decode(data));
  if (!decoded.ok) return decoded;
  return ok({
    kind: "createTree",
    tree: accounts[ACC.createTree.merkleTree]!,
    depth: decoded.value.maxDepth,
    maxBufferSize: decoded.value.maxBufferSize,
  });
}

function parseMintV1(
  data: Uint8Array,
  accounts: Address[],
  noopEvent: LeafSchemaEventDecoded | undefined,
): ParseResult<CnftEvent, ParseError> {
  if (accounts.length < MIN_ACCOUNTS.mintV1) {
    return err({
      kind: "insufficient-accounts",
      expected: MIN_ACCOUNTS.mintV1,
      actual: accounts.length,
    });
  }
  const decoded = tryDecode(() => getMintV1InstructionDataDecoder().decode(data));
  if (!decoded.ok) return decoded;

  const metadata = decodeMintMetadata(decoded.value.message, data);
  return ok({
    kind: "mint",
    tree: accounts[ACC.mintV1.merkleTree]!,
    owner: addressToBytes(accounts[ACC.mintV1.leafOwner]!),
    delegate: addressToBytes(accounts[ACC.mintV1.leafDelegate]!),
    metadata,
    verifyCollection: null,
    noop: noopToOverride(noopEvent),
  });
}

function parseMintToCollectionV1(
  data: Uint8Array,
  accounts: Address[],
  noopEvent: LeafSchemaEventDecoded | undefined,
): ParseResult<CnftEvent, ParseError> {
  if (accounts.length < MIN_ACCOUNTS.mintToCollectionV1) {
    return err({
      kind: "insufficient-accounts",
      expected: MIN_ACCOUNTS.mintToCollectionV1,
      actual: accounts.length,
    });
  }
  const decoded = tryDecode(() =>
    getMintToCollectionV1InstructionDataDecoder().decode(data),
  );
  if (!decoded.ok) return decoded;

  // mintToCollectionV1 verifies the collection as part of the ix — the
  // leaf is stored with collection.verified = true regardless of what
  // the raw metadataArgs said. Synthesize that here so the apply step
  // doesn't have to know the rule.
  const verifiedCollection = addressToBytes(
    accounts[ACC.mintToCollectionV1.collectionMint]!,
  );
  const rawMeta = decoded.value.metadataArgs;
  const coerced: MetadataArgs = {
    ...rawMeta,
    // Force collection to Some(verified=true, key=collectionMint) so the
    // dataHash we compute downstream matches what Bubblegum stores.
    // Note: Option types in Kit are `null | T`; Some variant is the bare value.
    collection: { __option: "Some", value: { key: accounts[ACC.mintToCollectionV1.collectionMint]!, verified: true } } as unknown as MetadataArgs["collection"],
  };
  const metadata = decodeMintMetadata(coerced, data);
  return ok({
    kind: "mint",
    tree: accounts[ACC.mintToCollectionV1.merkleTree]!,
    owner: addressToBytes(accounts[ACC.mintToCollectionV1.leafOwner]!),
    delegate: addressToBytes(accounts[ACC.mintToCollectionV1.leafDelegate]!),
    metadata,
    verifyCollection: verifiedCollection,
    noop: noopToOverride(noopEvent),
  });
}

function parseTransfer(
  data: Uint8Array,
  accounts: Address[],
  noopEvent: LeafSchemaEventDecoded | undefined,
): ParseResult<CnftEvent, ParseError> {
  if (accounts.length < MIN_ACCOUNTS.transfer) {
    return err({
      kind: "insufficient-accounts",
      expected: MIN_ACCOUNTS.transfer,
      actual: accounts.length,
    });
  }
  const decoded = tryDecode(() => getTransferInstructionDataDecoder().decode(data));
  if (!decoded.ok) return decoded;

  // Bubblegum semantics: on transfer, delegate is reset to the new owner.
  const newOwner = addressToBytes(accounts[ACC.transfer.newLeafOwner]!);
  return ok({
    kind: "transfer",
    tree: accounts[ACC.transfer.merkleTree]!,
    leafIndex: BigInt(decoded.value.index),
    nonce: decoded.value.nonce,
    newOwner,
    newDelegate: newOwner,
    dataHash: new Uint8Array(decoded.value.dataHash),
    creatorHash: new Uint8Array(decoded.value.creatorHash),
    noop: noopToOverride(noopEvent),
  });
}

function parseBurn(
  data: Uint8Array,
  accounts: Address[],
  noopEvent: LeafSchemaEventDecoded | undefined,
): ParseResult<CnftEvent, ParseError> {
  if (accounts.length < MIN_ACCOUNTS.burn) {
    return err({
      kind: "insufficient-accounts",
      expected: MIN_ACCOUNTS.burn,
      actual: accounts.length,
    });
  }
  const decoded = tryDecode(() => getBurnInstructionDataDecoder().decode(data));
  if (!decoded.ok) return decoded;
  return ok({
    kind: "burn",
    tree: accounts[ACC.burn.merkleTree]!,
    leafIndex: BigInt(decoded.value.index),
    nonce: decoded.value.nonce,
    noop: noopToOverride(noopEvent),
  });
}

function parseDelegate(
  data: Uint8Array,
  accounts: Address[],
  noopEvent: LeafSchemaEventDecoded | undefined,
): ParseResult<CnftEvent, ParseError> {
  if (accounts.length < MIN_ACCOUNTS.delegate) {
    return err({
      kind: "insufficient-accounts",
      expected: MIN_ACCOUNTS.delegate,
      actual: accounts.length,
    });
  }
  const decoded = tryDecode(() => getDelegateInstructionDataDecoder().decode(data));
  if (!decoded.ok) return decoded;
  return ok({
    kind: "delegate",
    tree: accounts[ACC.delegate.merkleTree]!,
    leafIndex: BigInt(decoded.value.index),
    nonce: decoded.value.nonce,
    newDelegate: addressToBytes(accounts[ACC.delegate.newLeafDelegate]!),
    dataHash: new Uint8Array(decoded.value.dataHash),
    creatorHash: new Uint8Array(decoded.value.creatorHash),
    noop: noopToOverride(noopEvent),
  });
}

// ─── noop-required ixs ───────────────────────────────────────────────
// These ixs mutate dataHash + creatorHash in ways that depend on prior
// state (flipping a creator's `verified` flag, replacing MetadataArgs
// whole-cloth, etc). We can't reconstruct the new hashes from outer-ix
// args alone, so we require the LeafSchemaEvent to be present. Without
// it the parser returns `unsupported` and the indexer skips the ix —
// the indexed state remains consistent with what was applied up to
// that point.

function requireNoop(
  noopEvent: LeafSchemaEventDecoded | undefined,
  ixName: string,
): ParseResult<NoopOverride, ParseError> {
  if (!noopEvent) {
    return err({
      kind: "unsupported",
      reason: `${ixName} requires a paired noop LeafSchemaEvent to resolve new state; none found in tx`,
    });
  }
  const override = noopToOverride(noopEvent);
  if (!override) {
    return err({
      kind: "unsupported",
      reason: `${ixName} noop event was V2 — only LeafSchema V1 is supported in this release`,
    });
  }
  return { ok: true, value: override };
}

function parseVerifyCreator(
  data: Uint8Array,
  accounts: Address[],
  noopEvent: LeafSchemaEventDecoded | undefined,
): ParseResult<CnftEvent, ParseError> {
  if (accounts.length < MIN_ACCOUNTS.verifyCreator) {
    return err({
      kind: "insufficient-accounts",
      expected: MIN_ACCOUNTS.verifyCreator,
      actual: accounts.length,
    });
  }
  const decoded = tryDecode(() => getVerifyCreatorInstructionDataDecoder().decode(data));
  if (!decoded.ok) return decoded;
  const noop = requireNoop(noopEvent, "verifyCreator");
  if (!noop.ok) return noop;
  return ok({
    kind: "verifyCreator",
    tree: accounts[ACC.verifyCreator.merkleTree]!,
    creator: addressToBytes(accounts[ACC.verifyCreator.creator]!),
    noop: noop.value,
  });
}

function parseUnverifyCreator(
  data: Uint8Array,
  accounts: Address[],
  noopEvent: LeafSchemaEventDecoded | undefined,
): ParseResult<CnftEvent, ParseError> {
  if (accounts.length < MIN_ACCOUNTS.unverifyCreator) {
    return err({
      kind: "insufficient-accounts",
      expected: MIN_ACCOUNTS.unverifyCreator,
      actual: accounts.length,
    });
  }
  const decoded = tryDecode(() =>
    getUnverifyCreatorInstructionDataDecoder().decode(data),
  );
  if (!decoded.ok) return decoded;
  const noop = requireNoop(noopEvent, "unverifyCreator");
  if (!noop.ok) return noop;
  return ok({
    kind: "unverifyCreator",
    tree: accounts[ACC.unverifyCreator.merkleTree]!,
    creator: addressToBytes(accounts[ACC.unverifyCreator.creator]!),
    noop: noop.value,
  });
}

function parseVerifyCollection(
  data: Uint8Array,
  accounts: Address[],
  noopEvent: LeafSchemaEventDecoded | undefined,
): ParseResult<CnftEvent, ParseError> {
  if (accounts.length < MIN_ACCOUNTS.verifyCollection) {
    return err({
      kind: "insufficient-accounts",
      expected: MIN_ACCOUNTS.verifyCollection,
      actual: accounts.length,
    });
  }
  const decoded = tryDecode(() =>
    getVerifyCollectionInstructionDataDecoder().decode(data),
  );
  if (!decoded.ok) return decoded;
  const noop = requireNoop(noopEvent, "verifyCollection");
  if (!noop.ok) return noop;
  return ok({
    kind: "verifyCollection",
    tree: accounts[ACC.verifyCollection.merkleTree]!,
    collection: addressToBytes(accounts[ACC.verifyCollection.collectionMint]!),
    noop: noop.value,
  });
}

function parseUnverifyCollection(
  data: Uint8Array,
  accounts: Address[],
  noopEvent: LeafSchemaEventDecoded | undefined,
): ParseResult<CnftEvent, ParseError> {
  if (accounts.length < MIN_ACCOUNTS.unverifyCollection) {
    return err({
      kind: "insufficient-accounts",
      expected: MIN_ACCOUNTS.unverifyCollection,
      actual: accounts.length,
    });
  }
  const decoded = tryDecode(() =>
    getUnverifyCollectionInstructionDataDecoder().decode(data),
  );
  if (!decoded.ok) return decoded;
  const noop = requireNoop(noopEvent, "unverifyCollection");
  if (!noop.ok) return noop;
  return ok({
    kind: "unverifyCollection",
    tree: accounts[ACC.unverifyCollection.merkleTree]!,
    collection: addressToBytes(accounts[ACC.unverifyCollection.collectionMint]!),
    noop: noop.value,
  });
}

function parseSetAndVerifyCollection(
  data: Uint8Array,
  accounts: Address[],
  noopEvent: LeafSchemaEventDecoded | undefined,
): ParseResult<CnftEvent, ParseError> {
  if (accounts.length < MIN_ACCOUNTS.setAndVerifyCollection) {
    return err({
      kind: "insufficient-accounts",
      expected: MIN_ACCOUNTS.setAndVerifyCollection,
      actual: accounts.length,
    });
  }
  const decoded = tryDecode(() =>
    getSetAndVerifyCollectionInstructionDataDecoder().decode(data),
  );
  if (!decoded.ok) return decoded;
  const noop = requireNoop(noopEvent, "setAndVerifyCollection");
  if (!noop.ok) return noop;
  return ok({
    kind: "setAndVerifyCollection",
    tree: accounts[ACC.setAndVerifyCollection.merkleTree]!,
    collection: addressToBytes(
      accounts[ACC.setAndVerifyCollection.collectionMint]!,
    ),
    noop: noop.value,
  });
}

function parseUpdateMetadata(
  data: Uint8Array,
  accounts: Address[],
  noopEvent: LeafSchemaEventDecoded | undefined,
): ParseResult<CnftEvent, ParseError> {
  if (accounts.length < MIN_ACCOUNTS.updateMetadata) {
    return err({
      kind: "insufficient-accounts",
      expected: MIN_ACCOUNTS.updateMetadata,
      actual: accounts.length,
    });
  }
  const decoded = tryDecode(() =>
    getUpdateMetadataInstructionDataDecoder().decode(data),
  );
  if (!decoded.ok) return decoded;
  const noop = requireNoop(noopEvent, "updateMetadata");
  if (!noop.ok) return noop;

  // The ix args carry the new (or nullable) MetadataArgs fields. We
  // preserve the Borsh-encoded bytes for dataHashInput so re-hashing
  // downstream matches on-chain semantics — same pattern as mintV1.
  const rawNewMeta = decoded.value.updateArgs;
  const meta = decodeUpdateArgsAsMetadata(rawNewMeta, data);
  return ok({
    kind: "updateMetadata",
    tree: accounts[ACC.updateMetadata.merkleTree]!,
    newMetadata: meta,
    noop: noop.value,
  });
}

/**
 * Convert a decoded LeafSchemaEvent into our NoopOverride shape.
 * Returns undefined for V2 schemas — we don't support them in this
 * release (V2 ixs are out of v0.6 scope; they'd need their own apply
 * paths). Parsers using this on a noop-optional ix treat undefined as
 * "no authoritative state available, reconstruct from args."
 */
function noopToOverride(
  noopEvent: LeafSchemaEventDecoded | undefined,
): NoopOverride | undefined {
  if (!noopEvent) return undefined;
  if (noopEvent.schema.kind !== "V1") return undefined;
  const s = noopEvent.schema;
  return {
    leafIndex: s.nonce,
    nonce: s.nonce,
    owner: s.owner,
    delegate: s.delegate,
    dataHash: s.dataHash,
    creatorHash: s.creatorHash,
  };
}

/**
 * updateMetadata's `updateArgs` type doesn't match `MetadataArgs`
 * exactly — every field is Option-wrapped because partial updates are
 * allowed. For dataHashInput we want the Borsh-serialized *new*
 * MetadataArgs; but reconstructing that from the Option-wrapped diff
 * requires the old metadata, which we have in the store at apply time.
 *
 * For now we store the raw ix bytes as the preimage anchor and let
 * apply.ts handle the merge when it has access to the current
 * LeafRecord. The MintMetadata surface below carries `dataHashInput`
 * pointing at the whole ix payload (minus discriminator); a follow-up
 * can compute the authoritative new MetadataArgs bytes if we need
 * byte-for-byte fidelity. Since noop gives us the authoritative
 * dataHash directly, downstream consumers don't depend on recomputing
 * it from the preimage.
 */
function decodeUpdateArgsAsMetadata(
  updateArgs: unknown,
  ixData: Uint8Array,
): MintMetadata {
  // We read whatever fields are `Some` and fill the rest with empty
  // defaults. The store's existing mintMetadata wins for anything the
  // update didn't change — applied in apply.ts.
  const a = updateArgs as Partial<{
    name: { __option: "Some"; value: string } | { __option: "None" };
    symbol: { __option: "Some"; value: string } | { __option: "None" };
    uri: { __option: "Some"; value: string } | { __option: "None" };
    sellerFeeBasisPoints:
      | { __option: "Some"; value: number }
      | { __option: "None" };
    primarySaleHappened:
      | { __option: "Some"; value: boolean }
      | { __option: "None" };
    isMutable: { __option: "Some"; value: boolean } | { __option: "None" };
    creators:
      | { __option: "Some"; value: Array<{ address: Address; verified: boolean; share: number }> }
      | { __option: "None" };
  }>;

  const takeOpt = <T>(o: { __option: "Some"; value: T } | { __option: "None" } | undefined): T | null => {
    if (!o) return null;
    return o.__option === "Some" ? o.value : null;
  };

  const name = takeOpt(a.name) ?? "";
  const symbol = takeOpt(a.symbol) ?? "";
  const uri = takeOpt(a.uri) ?? "";
  const sellerFeeBasisPoints = takeOpt(a.sellerFeeBasisPoints) ?? 0;
  const primarySaleHappened = takeOpt(a.primarySaleHappened) ?? false;
  const isMutable = takeOpt(a.isMutable) ?? true;
  const creatorsDecoded = takeOpt(a.creators);
  const creators: Creator[] = creatorsDecoded
    ? creatorsDecoded.map((c) => ({
        address: addressToBytes(c.address),
        verified: c.verified,
        share: c.share,
      }))
    : [];

  return {
    name,
    symbol,
    uri,
    sellerFeeBasisPoints,
    primarySaleHappened,
    isMutable,
    creators,
    collection: null,
    dataHashInput: ixData.slice(8),
  };
}

// ─── helpers ─────────────────────────────────────────────────────────

/**
 * Extract the MintMetadata we persist for DAS reconstruction + keep the
 * encoded ix data bytes used as the dataHash input. We store the raw
 * ix bytes here (minus the 8-byte discriminator) because re-encoding
 * MetadataArgs after a partial mutation risks diverging from the
 * original if Kit's encoder changes alignment/optionality conventions.
 */
function decodeMintMetadata(meta: MetadataArgs, ixData: Uint8Array): MintMetadata {
  const creators: Creator[] = meta.creators.map((c) => ({
    address: addressToBytes(c.address),
    verified: c.verified,
    share: c.share,
  }));

  // Kit's Option<T> renders as `{ __option: "Some", value: T }` or
  // `{ __option: "None" }` at decode time. Unwrap once here so callers
  // don't need to know the encoding.
  const collection = unwrapOption(meta.collection);
  const coll = collection
    ? {
        key: addressToBytes(collection.key),
        verified: collection.verified,
      }
    : null;

  return {
    name: meta.name,
    symbol: meta.symbol,
    uri: meta.uri,
    sellerFeeBasisPoints: meta.sellerFeeBasisPoints,
    primarySaleHappened: meta.primarySaleHappened,
    isMutable: meta.isMutable,
    creators,
    collection: coll,
    // The dataHash per Bubblegum is keccak(MetadataArgs.try_to_vec()) —
    // the Borsh-encoded struct, not including the ix discriminator.
    // `ixData.slice(8)` gives us that directly for mintV1; for
    // mintToCollectionV1 the dataHash input differs (collection.verified
    // is toggled to true before hashing), which our caller has
    // pre-coerced before calling this helper. Storing these bytes here
    // preserves the exact preimage for re-hashing on future mutations.
    dataHashInput: ixData.slice(8),
  };
}

function tryDecode<T>(fn: () => T): ParseResult<T, ParseError> {
  try {
    return ok(fn());
  } catch (e) {
    return err({
      kind: "decoder-error",
      message: e instanceof Error ? e.message : String(e),
    });
  }
}

function unwrapOption<T>(opt: { __option: "Some"; value: T } | { __option: "None" } | T | null): T | null {
  if (opt === null || opt === undefined) return null;
  if (typeof opt === "object" && "__option" in opt) {
    return opt.__option === "Some" ? (opt.value as T) : null;
  }
  return opt as T;
}

function eq(a: Uint8Array, b: Uint8Array): boolean {
  if (a.length !== b.length) return false;
  for (let i = 0; i < a.length; i++) if (a[i] !== b[i]) return false;
  return true;
}

function ok<T>(value: T): ParseResult<T, ParseError> {
  return { ok: true, value };
}

function err<T>(error: ParseError): ParseResult<T, ParseError> {
  return { ok: false, error };
}

/**
 * Convert a Kit `Address` (base58 string) into its raw 32-byte form. Kit
 * doesn't expose a sync decoder directly; we reach for the base58
 * decoder from @solana/kit. Kept in one place so the rest of the module
 * never touches base58.
 */
function addressToBytes(addr: Address): Uint8Array {
  return base58Decode(addr as unknown as string);
}

// Minimal inline base58 decode sufficient for 32-byte Solana addresses.
// Keeping this self-contained avoids pulling a base58 lib into the
// cNFT module's portability surface. When the code ports to Rust,
// `bs58::decode(...)` replaces this outright.
const BS58_ALPHABET = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";
const BS58_MAP: Record<string, number> = (() => {
  const m: Record<string, number> = {};
  for (let i = 0; i < BS58_ALPHABET.length; i++) m[BS58_ALPHABET[i]!] = i;
  return m;
})();

function base58Decode(s: string): Uint8Array {
  if (s.length === 0) return new Uint8Array(0);
  const bytes: number[] = [0];
  for (const ch of s) {
    const v = BS58_MAP[ch];
    if (v === undefined) throw new Error(`base58: invalid char '${ch}'`);
    let carry = v;
    for (let i = 0; i < bytes.length; i++) {
      carry += bytes[i]! * 58;
      bytes[i] = carry & 0xff;
      carry >>= 8;
    }
    while (carry > 0) {
      bytes.push(carry & 0xff);
      carry >>= 8;
    }
  }
  // Leading '1's in the source = leading zero bytes in the output.
  for (let i = 0; i < s.length && s[i] === "1"; i++) bytes.push(0);
  return new Uint8Array(bytes.reverse());
}
