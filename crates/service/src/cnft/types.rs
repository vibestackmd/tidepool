//! cNFT event and state types.
//!
//! Everything here is plain data — no methods that do I/O, no Solana-
//! client lifetimes, no async. Bytes are `[u8; 32]` where the
//! protocol demands 32-byte values. The `noop` field tracks
//! authoritative leaf state emitted by Bubblegum's LeafSchemaEvent
//! CPI — optional on the mint/transfer/burn/delegate family (where we
//! can reconstruct state from ix args if needed), **required** on the
//! verify/update family (where we can't).
//!
//! The TypeScript version encoded the "noop required vs optional"
//! distinction at runtime; Rust's type system encodes it structurally:
//! `VerifyCreator { noop: NoopOverride }` simply cannot be
//! constructed without one.

use tidepool_rpc_core::Creator;

/// Authoritative leaf state from a LeafSchemaEvent. Whenever present,
/// `apply_event` uses these values directly instead of reconstructing
/// from ix args + prior store state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoopOverride {
    pub leaf_index: u64,
    pub nonce: u64,
    pub owner: [u8; 32],
    pub delegate: [u8; 32],
    pub data_hash: [u8; 32],
    pub creator_hash: [u8; 32],
}

/// Per-tree metadata captured at `createTree`. `num_minted` starts at
/// zero and increments on every applied mint. The indexer reaches for
/// it to assign fresh leaf indices when no noop override is available.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeInfo {
    pub tree: [u8; 32],
    pub depth: u8,
    pub max_buffer_size: u32,
    pub num_minted: u64,
}

/// Metadata captured at mint time, preserved verbatim so DAS responses
/// can be reconstructed without re-reading the chain. `data_hash_input`
/// is the Borsh-encoded MetadataArgs preimage used to compute the
/// dataHash — kept around so `updateMetadata` can diff cleanly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MintMetadata {
    pub name: String,
    pub symbol: String,
    pub uri: String,
    pub seller_fee_basis_points: u16,
    pub primary_sale_happened: bool,
    pub is_mutable: bool,
    pub creators: Vec<Creator>,
    /// `Some((collection_key, verified))` when the metadata args
    /// referenced a collection; `None` otherwise.
    pub collection: Option<([u8; 32], bool)>,
    pub data_hash_input: Vec<u8>,
}

/// Durable per-asset record in `CnftStore`. Immutable-after-mint
/// fields sit above the mutable ones to make the distinction visually
/// obvious.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeafRecord {
    // Immutable once minted.
    pub asset_id: [u8; 32],
    pub tree: [u8; 32],
    pub nonce: u64,
    pub leaf_index: u64,
    pub mint_metadata: MintMetadata,

    // Mutates on transfer / delegate / verify* / update / burn.
    pub owner: [u8; 32],
    pub delegate: [u8; 32],
    pub data_hash: [u8; 32],
    pub creator_hash: [u8; 32],
    pub leaf_hash: [u8; 32],
    pub burned: bool,
}

/// Every on-chain event we replay. Noop-required variants (verify*,
/// updateMetadata) carry a non-optional `noop: NoopOverride` —
/// compile-time guarantee that apply will see authoritative state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CnftEvent {
    CreateTree {
        tree: [u8; 32],
        depth: u8,
        max_buffer_size: u32,
    },
    Mint {
        tree: [u8; 32],
        owner: [u8; 32],
        delegate: [u8; 32],
        metadata: MintMetadata,
        /// Set by `mintToCollectionV1` to the collection mint; `None`
        /// for plain `mintV1` even if metadata references a collection.
        verify_collection: Option<[u8; 32]>,
        /// Authoritative state from the paired LeafSchemaEvent, if any.
        noop: Option<NoopOverride>,
    },
    Transfer {
        tree: [u8; 32],
        leaf_index: u64,
        nonce: u64,
        new_owner: [u8; 32],
        /// Bubblegum resets delegate to newOwner on transfer; the
        /// parser carries that rule on our behalf.
        new_delegate: [u8; 32],
        data_hash: [u8; 32],
        creator_hash: [u8; 32],
        noop: Option<NoopOverride>,
    },
    Burn {
        tree: [u8; 32],
        leaf_index: u64,
        nonce: u64,
        noop: Option<NoopOverride>,
    },
    Delegate {
        tree: [u8; 32],
        leaf_index: u64,
        nonce: u64,
        new_delegate: [u8; 32],
        data_hash: [u8; 32],
        creator_hash: [u8; 32],
        noop: Option<NoopOverride>,
    },

    // ─── noop-required family ───────────────────────────────────────

    VerifyCreator {
        tree: [u8; 32],
        creator: [u8; 32],
        noop: NoopOverride,
    },
    UnverifyCreator {
        tree: [u8; 32],
        creator: [u8; 32],
        noop: NoopOverride,
    },
    VerifyCollection {
        tree: [u8; 32],
        collection: [u8; 32],
        noop: NoopOverride,
    },
    UnverifyCollection {
        tree: [u8; 32],
        collection: [u8; 32],
        noop: NoopOverride,
    },
    SetAndVerifyCollection {
        tree: [u8; 32],
        collection: [u8; 32],
        noop: NoopOverride,
    },
    UpdateMetadata {
        tree: [u8; 32],
        new_metadata: MintMetadata,
        noop: NoopOverride,
    },
}
