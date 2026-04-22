//! Tidepool service layer — upstream-agnostic, transport-agnostic.
//!
//! This crate owns:
//!
//! - **cNFT state**: replayed Bubblegum tree state, event-sourced from
//!   `getSignaturesForAddress` + `getTransaction` responses.
//! - **DAS services** (step 3 — not yet landed): `get_asset`,
//!   `get_asset_proof`, and the search family.
//! - **Pluggable adapter traits**: [`UpstreamClient`], [`CnftStore`],
//!   [`CacheStore`]. Tests inject in-memory or fixture impls; the HTTP
//!   server wires in network-backed impls.
//!
//! Pure algorithms (keccak, merkle proof) live in `tidepool-rpc-core`.
//! Anything with I/O, async, or Solana-protocol awareness lives here.

#![forbid(unsafe_code)]

pub mod cache;
pub mod cnft;
pub mod das;
pub mod upstream;

// Re-export core primitives so downstream consumers don't need to
// depend on both crates by hand.
pub use tidepool_rpc_core::{
    compute_proof, empty_node, hash_creators, hash_leaf_v1, hash_metadata_args_bytes, hash_pair,
    keccak256, verify_proof, Creator, LeafSchemaV1, MerkleProof, ProofError, TreeState,
};
