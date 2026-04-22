//! Tidepool core primitives. Pure functions over plain data — no I/O,
//! no async, no Solana client dependencies. Everything here is safe to
//! run in WASM and trivially portable.
//!
//! Three modules:
//!
//! - [`types`]: shared data structures (`Creator`, `TreeState`,
//!   `LeafSchemaV1`, `MerkleProof`).
//! - [`hash`]: keccak256 primitives, merkle-pair hashing, empty-node
//!   cascade, LeafSchema V1 + creator + metadata hashing.
//! - [`proof`]: compute + verify merkle proofs over a sparse
//!   [`TreeState`].
//!
//! All bytes are `[u8; 32]` where the domain demands it — no
//! runtime-length-checking, no accidentally-truncated buffers.

#![forbid(unsafe_code)]

pub mod hash;
pub mod proof;
pub mod types;

pub use hash::{
    empty_node, hash_creators, hash_leaf_v1, hash_metadata_args_bytes, hash_pair, keccak256,
};
pub use proof::{compute_proof, verify_proof, ProofError};
pub use types::{Creator, LeafSchemaV1, MerkleProof, TreeState};
