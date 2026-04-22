// cNFT core types. Everything here is plain data — no methods, no classes,
// no I/O — so it ports cleanly to Rust. The shapes intentionally mirror
// what Bubblegum's on-chain state transitions describe, not what DAS
// returns; DAS shaping happens at the handler boundary.
//
// Bytes are always `Uint8Array` (never hex). 64-bit integers are `bigint`
// (never `number`) — Rust's `u64` doesn't fit in JS's safe integer range.

import type { Address } from "@solana/kit";

// Creator entry as it appears in Bubblegum's metadata args and in the
// creator-hash computation. Matches `mpl-bubblegum::Creator`.
export interface Creator {
  address: Uint8Array; // 32 bytes
  verified: boolean;
  share: number; // u8
}

// Tree metadata recorded at createTree time. Depth is the one piece we
// need for proof computation; maxBufferSize is informational. `numMinted`
// increments on every mint and is how we assign leaf indexes — Bubblegum
// stores it on the tree authority account, but tracking it locally
// avoids an extra on-chain read per mint.
export interface TreeInfo {
  tree: Address;
  depth: number;
  maxBufferSize: number;
  numMinted: bigint;
}

// Tree state we maintain per merkle tree. `leaves` is sparse — unset
// positions are empty (all-zeros leaf). `depth` comes from the tree's
// createTree instruction and never changes afterwards (we reject trees
// that attempt to resize).
export interface TreeState {
  tree: Address;
  depth: number;
  /** Sparse: leafIndex → current leaf hash. Absent entries are empty. */
  leaves: Map<bigint, Uint8Array>;
}

// Leaf hash input for LeafSchema V1. Every field is the raw on-chain
// representation — no base58, no JSON. The derived hash is what lives in
// the merkle tree as the leaf node.
export interface LeafSchemaV1 {
  id: Uint8Array; // 32 bytes — the asset ID (Bubblegum PDA)
  owner: Uint8Array; // 32 bytes
  delegate: Uint8Array; // 32 bytes
  nonce: bigint; // u64, same as leafIndex at mint time
  dataHash: Uint8Array; // 32 bytes
  creatorHash: Uint8Array; // 32 bytes
}

// Durable per-asset record the store keeps. Separates "things that never
// change after mint" (id, tree, nonce, leafIndex, mint metadata) from
// "things that can change" (owner, delegate, dataHash, creatorHash,
// leafHash, and whether the leaf has been burned).
export interface LeafRecord {
  // Immutable once minted.
  assetId: Address;
  tree: Address;
  nonce: bigint;
  leafIndex: bigint;
  mintMetadata: MintMetadata;

  // Mutates on transfer, delegate, verify*, updateMetadata, burn.
  owner: Uint8Array;
  delegate: Uint8Array;
  dataHash: Uint8Array;
  creatorHash: Uint8Array;
  leafHash: Uint8Array; // keccak of the above via hashLeafV1
  burned: boolean;
}

// What we captured from the original mint — preserved verbatim so DAS
// handlers can reconstruct a response without re-reading the chain. The
// fields we care about map cleanly onto DAS's asset shape; the encoded
// bytes are retained so updateMetadata can diff.
export interface MintMetadata {
  name: string;
  symbol: string;
  uri: string;
  sellerFeeBasisPoints: number;
  primarySaleHappened: boolean;
  isMutable: boolean;
  creators: Creator[];
  /** Optional collection key from the metadata args (32 bytes) + whether it's marked verified. */
  collection: { key: Uint8Array; verified: boolean } | null;
  /**
   * The Borsh-serialized MetadataArgs bytes keccaked for dataHash. Kept so we
   * can recompute on mutation without re-Borsh-encoding the struct.
   */
  dataHashInput: Uint8Array;
}

// Result of computing a merkle proof for a leaf. Matches the shape DAS's
// `getAssetProof` returns, minus the base58 encoding + tree_id field which
// the handler layers on. `proof[]` goes from sibling-of-leaf up to
// sibling-of-root.
export interface MerkleProof {
  leaf: Uint8Array; // 32 bytes — the leaf hash being proved
  proof: Uint8Array[]; // each 32 bytes — sibling hashes, leaf → root
  root: Uint8Array; // 32 bytes — the computed root
  nodeIndex: number; // position in the tree, 2^depth + leafIndex
}

// Result variant used by every parser / fallible-pure-function in this
// module. Explicit Result<T, E> style so it ports straight to Rust, and
// so "this ix isn't one we care about" is a normal return, not an
// exception we have to catch.
export type ParseResult<T, E = string> =
  | { ok: true; value: T }
  | { ok: false; error: E };

/**
 * Authoritative leaf state pulled from the noop-CPI LeafSchemaEvent.
 * When present on any CnftEvent, apply-events uses these values
 * directly instead of reconstructing from ix args + stored state. For
 * ixs where reconstruction is impossible (verifyCreator etc.), the
 * parser requires this to be present — otherwise it surfaces an
 * `unsupported` error that the indexer skips.
 */
export interface NoopOverride {
  leafIndex: bigint;
  nonce: bigint;
  owner: Uint8Array; // 32
  delegate: Uint8Array; // 32
  dataHash: Uint8Array; // 32
  creatorHash: Uint8Array; // 32
}

// Discriminated union of every on-chain event we care to replay. Each
// variant carries exactly what applyEvent needs — no more. Shared fields
// (tree, leafIndex, slot, txSig) live at the top of each variant for
// consistency with how the indexer reads them.
//
// The `noop` field is optional on mint/transfer/burn/delegate (we can
// reconstruct their state without it, but trust noop when present as a
// cross-check) and REQUIRED on verifyCreator/verifyCollection/
// updateMetadata (no reconstruction path exists — new dataHash +
// creatorHash come straight from the event).
export type CnftEvent =
  | {
      kind: "createTree";
      tree: Address;
      depth: number;
      maxBufferSize: number;
    }
  | {
      kind: "mint";
      tree: Address;
      owner: Uint8Array; // 32
      delegate: Uint8Array; // 32
      metadata: MintMetadata;
      /**
       * If present, the collection this mint is explicitly verified into.
       * mintToCollectionV1 sets this; mintV1 leaves it null even if the
       * metadata args include an unverified collection reference.
       */
      verifyCollection: Uint8Array | null;
      /** Authoritative state from the paired LeafSchemaEvent, if available. */
      noop?: NoopOverride;
    }
  | {
      kind: "transfer";
      tree: Address;
      leafIndex: bigint;
      nonce: bigint;
      newOwner: Uint8Array; // 32
      // On transfer, the on-chain program resets delegate to newOwner.
      // We carry it explicitly so the apply step doesn't have to know
      // that rule.
      newDelegate: Uint8Array; // 32
      // dataHash + creatorHash stay the same on transfer; they're
      // asserted by the caller in the ix args and we pass them through
      // as a correctness check.
      dataHash: Uint8Array; // 32
      creatorHash: Uint8Array; // 32
      noop?: NoopOverride;
    }
  | {
      kind: "burn";
      tree: Address;
      leafIndex: bigint;
      nonce: bigint;
      noop?: NoopOverride;
    }
  | {
      kind: "delegate";
      tree: Address;
      leafIndex: bigint;
      nonce: bigint;
      newDelegate: Uint8Array; // 32
      // Same caller-asserted hashes as transfer, unchanged by delegate.
      dataHash: Uint8Array; // 32
      creatorHash: Uint8Array; // 32
      noop?: NoopOverride;
    }
  // The remaining variants exist only because their new-state hashes
  // can't be reconstructed from outer-ix args. They carry the noop
  // override as a non-optional field (enforced by the parser).
  | {
      kind: "verifyCreator";
      tree: Address;
      /** The creator whose `verified` flag was flipped to true. */
      creator: Uint8Array; // 32
      noop: NoopOverride;
    }
  | {
      kind: "unverifyCreator";
      tree: Address;
      creator: Uint8Array; // 32
      noop: NoopOverride;
    }
  | {
      kind: "verifyCollection";
      tree: Address;
      /** The collection key whose `verified` flag was flipped to true. */
      collection: Uint8Array; // 32
      noop: NoopOverride;
    }
  | {
      kind: "unverifyCollection";
      tree: Address;
      collection: Uint8Array; // 32
      noop: NoopOverride;
    }
  | {
      kind: "setAndVerifyCollection";
      tree: Address;
      collection: Uint8Array; // 32
      noop: NoopOverride;
    }
  | {
      kind: "updateMetadata";
      tree: Address;
      /** Whole new MetadataArgs preimage — the Borsh bytes used for the dataHash. */
      newMetadata: MintMetadata;
      noop: NoopOverride;
    };

// Bubblegum-ix-level parse failure reasons. These are strings at the
// TypeScript level but map to a Rust enum. Callers typically log and
// skip on any failure — a malformed ix is never fatal for the indexer.
export type ParseError =
  | { kind: "unknown-discriminator"; discriminator: Uint8Array }
  | { kind: "truncated-data"; expected: number; actual: number }
  | { kind: "insufficient-accounts"; expected: number; actual: number }
  | { kind: "decoder-error"; message: string }
  | { kind: "unsupported"; reason: string };
