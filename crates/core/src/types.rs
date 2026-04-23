//! Shared data structures for Tidepool core.
//!
//! Deliberately plain: no methods that do I/O, no trait impls that
//! reach outside the crate, no lifetimes. Bytes are `[u8; 32]` where
//! the protocol demands 32-byte values (hashes, pubkeys). 64-bit
//! indices are `u64`.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Creator entry as it appears in Bubblegum's metadata args and in the
/// creator-hash computation. Matches `mpl-bubblegum::Creator`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Creator {
    pub address: [u8; 32],
    pub verified: bool,
    pub share: u8,
}

/// Sparse merkle-tree state snapshot for a Bubblegum tree. Unset leaf
/// positions are implicitly the empty-node value at height 0 — i.e.
/// 32 zero bytes.
///
/// `BTreeMap` rather than `HashMap` so iteration order is deterministic:
/// proof computation relies on visiting populated positions, and tests
/// get reproducible output for free.
#[derive(Debug, Clone)]
pub struct TreeState {
    /// Depth of the tree from leaves to root. Valid range: 1..=30.
    pub depth: u8,
    /// Populated leaves, keyed by position. Absent entries are empty.
    pub leaves: BTreeMap<u64, [u8; 32]>,
}

impl TreeState {
    /// Build an empty tree at the given depth.
    #[must_use]
    pub fn new(depth: u8) -> Self {
        Self {
            depth,
            leaves: BTreeMap::new(),
        }
    }
}

/// LeafSchema V1 fields in their raw on-chain form. Hashed by
/// [`crate::hash::hash_leaf_v1`] to produce the merkle leaf node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeafSchemaV1 {
    pub id: [u8; 32],
    pub owner: [u8; 32],
    pub delegate: [u8; 32],
    pub nonce: u64,
    pub data_hash: [u8; 32],
    pub creator_hash: [u8; 32],
}

/// Result of computing a merkle proof. Maps 1:1 onto the DAS
/// `getAssetProof` wire shape once base58-encoded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MerkleProof {
    pub leaf: [u8; 32],
    pub proof: Vec<[u8; 32]>,
    pub root: [u8; 32],
    pub node_index: u64,
}
