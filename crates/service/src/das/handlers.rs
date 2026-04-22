//! Service-layer DAS handler functions. Async, `CnftStore`-backed.
//! Return domain types (`DasAsset`, `DasAssetProof`); wiring to
//! JSON-RPC lives in the server crate.
//!
//! Step 3a scope: cNFT-only paths. Uncompressed assets (MplCore,
//! Token Metadata) land in 3b when their decoders come online; until
//! then `get_asset` returns `Ok(None)` for anything not in the cNFT
//! index.

use std::collections::BTreeMap;
use std::sync::Arc;

use thiserror::Error;
use tidepool_rpc_core::{compute_proof, ProofError, TreeState};

use crate::cache::{CacheError, CacheStore, SearchFilter};
use crate::cnft::{CnftStore, LeafRecord};
use crate::upstream::UpstreamClient;

use super::cnft_to_das::leaf_record_to_das_asset;
use super::decoder::AccountDecoder;
use super::fetch::{fetch_and_cache_asset, FetchError};
use super::types::{DasAsset, DasAssetProof};

#[derive(Debug, Error)]
pub enum DasError {
    #[error(transparent)]
    Store(#[from] crate::cnft::store::StoreError),
    #[error(transparent)]
    Cache(#[from] CacheError),
    #[error(transparent)]
    Fetch(#[from] FetchError),
    #[error("proof computation failed: {0}")]
    Proof(ProofError),
}

// ProofError doesn't derive Error, so we manually bridge.
impl From<ProofError> for DasError {
    fn from(e: ProofError) -> Self {
        Self::Proof(e)
    }
}

pub type DasResult<T> = Result<T, DasError>;

// ─── getAsset ──────────────────────────────────────────────────────

/// Compressed-only `getAsset`. Kept as a dedicated helper so tests
/// against a `CnftStore`-only ctx don't need upstream/cache wiring.
pub async fn get_asset<S: CnftStore + ?Sized>(
    cnft: &S,
    asset_id: &[u8; 32],
) -> DasResult<Option<DasAsset>> {
    if let Some(record) = cnft.get_leaf(asset_id).await? {
        return Ok(Some(leaf_record_to_das_asset(&record)));
    }
    Ok(None)
}

/// Full `getAsset` dispatch: cNFT store first, then cache, then
/// upstream fetch + decoder pipeline. Populates the cache on decode
/// success. `asset_id_b58` is the base58 address (mint or PDA) —
/// callers typically take this from JSON-RPC params untouched.
pub async fn get_asset_full<S, C, U>(
    cnft: &S,
    cache: &C,
    upstream: &U,
    decoders: &[Arc<dyn AccountDecoder>],
    asset_id_b58: &str,
) -> DasResult<Option<DasAsset>>
where
    S: CnftStore + ?Sized,
    C: CacheStore + ?Sized,
    U: UpstreamClient + ?Sized,
{
    // 1. cNFT first. Decode base58 to [u8; 32]; unparseable ids fall
    //    through to the uncompressed path which may still resolve.
    if let Some(id_bytes) = try_decode_bs58_32(asset_id_b58) {
        if let Some(record) = cnft.get_leaf(&id_bytes).await? {
            return Ok(Some(leaf_record_to_das_asset(&record)));
        }
    }

    // 2. Cache hit — serves repeats without an upstream round-trip.
    if let Some(cached) = cache.get_asset(asset_id_b58).await? {
        return Ok(Some(cached));
    }

    // 3. Upstream fetch + decoder dispatch + cache populate.
    Ok(fetch_and_cache_asset(upstream, cache, decoders, asset_id_b58).await?)
}

// ─── getAssetBatch ─────────────────────────────────────────────────

/// Parallel `getAsset` over many ids. Preserves input order; unknown
/// ids map to `None`. Cache lookups run serially (fast, in-memory);
/// upstream fetches run in parallel via `futures` joins.
pub async fn get_asset_batch<S, C, U>(
    cnft: &S,
    cache: &C,
    upstream: &U,
    decoders: &[Arc<dyn AccountDecoder>],
    asset_ids_b58: &[String],
) -> DasResult<Vec<Option<DasAsset>>>
where
    S: CnftStore + ?Sized,
    C: CacheStore + ?Sized,
    U: UpstreamClient + ?Sized,
{
    let mut out = Vec::with_capacity(asset_ids_b58.len());
    for id in asset_ids_b58 {
        // One-at-a-time keeps the lock-contention story simple on
        // MemoryCache's single Mutex. For larger batch fanouts we'd
        // spawn tasks; at current scale (batches up to 1000) the
        // upstream's own concurrency model is the bottleneck.
        out.push(get_asset_full(cnft, cache, upstream, decoders, id).await?);
    }
    Ok(out)
}

// ─── by-X handlers (cache-only) ────────────────────────────────────

pub async fn get_assets_by_owner<C: CacheStore + ?Sized>(
    cache: &C,
    owner: &str,
) -> DasResult<Vec<DasAsset>> {
    Ok(cache.get_assets_by_owner(owner).await?)
}

pub async fn get_assets_by_authority<C: CacheStore + ?Sized>(
    cache: &C,
    authority: &str,
) -> DasResult<Vec<DasAsset>> {
    Ok(cache.get_assets_by_authority(authority).await?)
}

pub async fn get_assets_by_creator<C: CacheStore + ?Sized>(
    cache: &C,
    creator: &str,
    only_verified: bool,
) -> DasResult<Vec<DasAsset>> {
    Ok(cache.get_assets_by_creator(creator, only_verified).await?)
}

pub async fn get_assets_by_group<C: CacheStore + ?Sized>(
    cache: &C,
    group_key: &str,
    group_value: &str,
) -> DasResult<Vec<DasAsset>> {
    Ok(cache.get_assets_by_group(group_key, group_value).await?)
}

pub async fn search_assets<C: CacheStore + ?Sized>(
    cache: &C,
    filter: &SearchFilter,
) -> DasResult<Vec<DasAsset>> {
    Ok(cache.search_assets(filter).await?)
}

fn try_decode_bs58_32(s: &str) -> Option<[u8; 32]> {
    let bytes = bs58::decode(s).into_vec().ok()?;
    bytes.try_into().ok()
}

// ─── getAssetProof ─────────────────────────────────────────────────

/// `getAssetProof` for a compressed asset. Materializes a `TreeState`
/// from all non-burned leaves in the tree, then runs the pure
/// `compute_proof`. Returns `Ok(None)` when the asset (or its tree)
/// isn't indexed.
pub async fn get_asset_proof<S: CnftStore + ?Sized>(
    cnft: &S,
    asset_id: &[u8; 32],
) -> DasResult<Option<DasAssetProof>> {
    let Some(leaf) = cnft.get_leaf(asset_id).await? else {
        return Ok(None);
    };
    let Some(tree_info) = cnft.get_tree(&leaf.tree).await? else {
        return Ok(None);
    };

    let state = build_tree_state(cnft, &leaf.tree, tree_info.depth).await?;
    let proof = compute_proof(&state, leaf.leaf_index)?;

    Ok(Some(DasAssetProof {
        root: bs58::encode(proof.root).into_string(),
        proof: proof
            .proof
            .iter()
            .map(|n| bs58::encode(n).into_string())
            .collect(),
        node_index: proof.node_index,
        leaf: bs58::encode(proof.leaf).into_string(),
        tree_id: bs58::encode(leaf.tree).into_string(),
    }))
}

/// Parallel version. Preserves input order; unknown ids map to
/// `None`. Internally shares one `TreeState` materialization per
/// unique tree to keep N×proof requests from triggering N×
/// `list_leaves` scans.
pub async fn get_asset_proof_batch<S: CnftStore + ?Sized>(
    cnft: &S,
    asset_ids: &[[u8; 32]],
) -> DasResult<Vec<Option<DasAssetProof>>> {
    let mut results: Vec<Option<DasAssetProof>> = Vec::with_capacity(asset_ids.len());

    // First pass: look up every leaf + group by tree. This lets us
    // materialize each tree's state exactly once even for large
    // batches spanning multiple trees.
    let mut leaves: Vec<Option<LeafRecord>> = Vec::with_capacity(asset_ids.len());
    let mut trees_needed: BTreeMap<[u8; 32], u8> = BTreeMap::new();
    for id in asset_ids {
        let leaf = cnft.get_leaf(id).await?;
        if let Some(ref l) = leaf {
            if let std::collections::btree_map::Entry::Vacant(e) = trees_needed.entry(l.tree) {
                // Depth is the same for every leaf in a tree; we'll
                // fill it from TreeInfo below.
                e.insert(0);
            }
        }
        leaves.push(leaf);
    }

    // Pull each distinct tree's info + state once.
    let mut tree_states: BTreeMap<[u8; 32], TreeState> = BTreeMap::new();
    for tree in trees_needed.keys().copied().collect::<Vec<_>>() {
        let Some(info) = cnft.get_tree(&tree).await? else {
            continue;
        };
        let state = build_tree_state(cnft, &tree, info.depth).await?;
        tree_states.insert(tree, state);
    }

    for (i, id) in asset_ids.iter().enumerate() {
        let Some(leaf) = leaves[i].as_ref() else {
            results.push(None);
            continue;
        };
        let Some(state) = tree_states.get(&leaf.tree) else {
            results.push(None);
            continue;
        };
        let proof = compute_proof(state, leaf.leaf_index)?;
        results.push(Some(DasAssetProof {
            root: bs58::encode(proof.root).into_string(),
            proof: proof
                .proof
                .iter()
                .map(|n| bs58::encode(n).into_string())
                .collect(),
            node_index: proof.node_index,
            leaf: bs58::encode(proof.leaf).into_string(),
            tree_id: bs58::encode(leaf.tree).into_string(),
        }));
        // Discard the unused value from asset_ids iteration — kept
        // for index alignment with `results` + `leaves`.
        let _ = id;
    }

    Ok(results)
}

// ─── helpers ───────────────────────────────────────────────────────

async fn build_tree_state<S: CnftStore + ?Sized>(
    cnft: &S,
    tree: &[u8; 32],
    depth: u8,
) -> DasResult<TreeState> {
    let mut leaves = BTreeMap::new();
    for rec in cnft.list_leaves(tree).await? {
        if !rec.burned {
            leaves.insert(rec.leaf_index, rec.leaf_hash);
        }
    }
    Ok(TreeState { depth, leaves })
}
