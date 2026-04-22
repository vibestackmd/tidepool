//! Keccak primitives + Bubblegum leaf / creator / metadata hashing.
//!
//! Every hash Bubblegum and spl-account-compression compute is
//! keccak256 (Ethereum-style keccak, not NIST SHA-3). The node-pair
//! hash, leaf-schema hash, data hash, and creator hash all reduce to
//! the same two primitives in this module.

use std::sync::{Mutex, OnceLock};

use sha3::{Digest, Keccak256};

use crate::types::{Creator, LeafSchemaV1};

/// keccak256 over a single buffer.
#[must_use]
pub fn keccak256(data: &[u8]) -> [u8; 32] {
    let mut hasher = Keccak256::new();
    hasher.update(data);
    let out = hasher.finalize();
    let mut bytes = [0u8; 32];
    bytes.copy_from_slice(&out);
    bytes
}

/// `keccak256(left || right)` — the node-pair hash used by
/// spl-account-compression's `hashv(&[left, right])`.
#[must_use]
pub fn hash_pair(left: &[u8; 32], right: &[u8; 32]) -> [u8; 32] {
    let mut hasher = Keccak256::new();
    hasher.update(left);
    hasher.update(right);
    let out = hasher.finalize();
    let mut bytes = [0u8; 32];
    bytes.copy_from_slice(&out);
    bytes
}

/// Hash of an all-empty subtree at the given height.
///
/// `height(0)` is an empty leaf slot — 32 zero bytes.
/// `height(n) = hash_pair(height(n-1), height(n-1))` by construction.
///
/// The cascade is memoized process-wide; callers at any depth pay the
/// computation once. A depth-30 tree needs 31 entries — small.
#[must_use]
pub fn empty_node(height: usize) -> [u8; 32] {
    static CASCADE: OnceLock<Mutex<Vec<[u8; 32]>>> = OnceLock::new();
    let cascade = CASCADE.get_or_init(|| Mutex::new(vec![[0u8; 32]]));
    let mut guard = cascade.lock().expect("empty_node cascade mutex poisoned");
    while guard.len() <= height {
        let prev = *guard.last().expect("cascade seeded with height 0");
        guard.push(hash_pair(&prev, &prev));
    }
    guard[height]
}

/// Leaf hash per LeafSchema::V1. Bubblegum computes:
///
/// ```text
/// keccak256(
///   0x01 || id || owner || delegate || nonce.to_le_bytes() ||
///   data_hash || creator_hash
/// )
/// ```
///
/// The `0x01` prefix is the schema-version discriminator; V2 leaves
/// use their own format and aren't covered here.
#[must_use]
pub fn hash_leaf_v1(leaf: &LeafSchemaV1) -> [u8; 32] {
    // Single allocation, exact size.
    let mut buf = Vec::with_capacity(1 + 32 + 32 + 32 + 8 + 32 + 32);
    buf.push(0x01);
    buf.extend_from_slice(&leaf.id);
    buf.extend_from_slice(&leaf.owner);
    buf.extend_from_slice(&leaf.delegate);
    buf.extend_from_slice(&leaf.nonce.to_le_bytes());
    buf.extend_from_slice(&leaf.data_hash);
    buf.extend_from_slice(&leaf.creator_hash);
    keccak256(&buf)
}

/// Data hash: `keccak256(borsh_serialize(MetadataArgs))`.
///
/// Takes the already-serialized bytes so this module doesn't have to
/// depend on Bubblegum's metadata types. Callers pass the Borsh-encoded
/// preimage.
#[must_use]
pub fn hash_metadata_args_bytes(metadata_args_bytes: &[u8]) -> [u8; 32] {
    keccak256(metadata_args_bytes)
}

/// Creator hash: keccak256 over concatenated `(address, verified, share)`
/// tuples — 32 + 1 + 1 = 34 bytes per creator. Matches the Bubblegum
/// reference impl. An empty creator list yields `keccak256(&[])`.
#[must_use]
pub fn hash_creators(creators: &[Creator]) -> [u8; 32] {
    let mut buf = Vec::with_capacity(creators.len() * 34);
    for c in creators {
        buf.extend_from_slice(&c.address);
        buf.push(u8::from(c.verified));
        buf.push(c.share);
    }
    keccak256(&buf)
}
