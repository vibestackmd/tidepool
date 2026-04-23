//! Persistence contract for replayed cNFT state + an in-memory impl.
//!
//! The trait is deliberately narrow: tree lifecycle, per-leaf CRUD,
//! tree-scoped listing for proof generation, and a last-indexed-
//! signature cursor for incremental scans. A SQLite-backed impl could
//! land behind the same contract without touching callers.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use thiserror::Error;
use tokio::sync::Mutex;

use super::types::{LeafRecord, TreeInfo};

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("unknown tree: {tree}")]
    UnknownTree { tree: String },
}

pub type StoreResult<T> = Result<T, StoreError>;

/// Contract every persistence backend implements. All operations are
/// async so a future SQLite impl slots in without changing the
/// handlers; the in-memory impl wraps tokio's async Mutex to match.
#[async_trait]
pub trait CnftStore: Send + Sync {
    // ─── tree lifecycle ───────────────────────────────────────────

    async fn put_tree(&self, info: TreeInfo) -> StoreResult<()>;
    async fn get_tree(&self, tree: &[u8; 32]) -> StoreResult<Option<TreeInfo>>;

    /// Allocate and return the next leaf index for a new mint on this
    /// tree. Bumps `num_minted` as a side effect.
    async fn alloc_leaf_index(&self, tree: &[u8; 32]) -> StoreResult<u64>;

    /// Force the tree's `num_minted` counter up to at least `floor`.
    /// The indexer uses this when a noop-override's nonce lands ahead
    /// of where we thought we were, so the counter stays consistent.
    async fn ensure_num_minted_at_least(&self, tree: &[u8; 32], floor: u64) -> StoreResult<()>;

    // ─── per-leaf state ───────────────────────────────────────────

    async fn put_leaf(&self, record: LeafRecord) -> StoreResult<()>;
    async fn get_leaf(&self, asset_id: &[u8; 32]) -> StoreResult<Option<LeafRecord>>;
    async fn get_leaf_by_index(
        &self,
        tree: &[u8; 32],
        leaf_index: u64,
    ) -> StoreResult<Option<LeafRecord>>;

    /// Return every leaf for `tree` in insertion order. Used to
    /// materialize a `TreeState` before calling `compute_proof`.
    async fn list_leaves(&self, tree: &[u8; 32]) -> StoreResult<Vec<LeafRecord>>;

    // ─── indexer bookkeeping ──────────────────────────────────────

    async fn get_last_signature(&self, tree: &[u8; 32]) -> StoreResult<Option<String>>;
    async fn set_last_signature(&self, tree: &[u8; 32], signature: String) -> StoreResult<()>;
}

/// In-memory `CnftStore`. Ships as the default. Thread-safe via a
/// single tokio `Mutex` guarding the entire state map — fine at our
/// scale (one Bubblegum tree = tens of thousands of leaves, reads and
/// writes are in the microsecond range).
#[derive(Default)]
pub struct MemoryCnftStore {
    inner: Arc<Mutex<MemoryStoreInner>>,
}

#[derive(Default)]
struct MemoryStoreInner {
    trees: HashMap<[u8; 32], TreeInfo>,
    leaves_by_asset: HashMap<[u8; 32], LeafRecord>,
    // (tree, leaf_index) → asset_id
    leaf_index: HashMap<([u8; 32], u64), [u8; 32]>,
    // insertion-ordered per tree
    tree_leaf_order: HashMap<[u8; 32], Vec<[u8; 32]>>,
    last_sig: HashMap<[u8; 32], String>,
}

impl MemoryCnftStore {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl CnftStore for MemoryCnftStore {
    async fn put_tree(&self, info: TreeInfo) -> StoreResult<()> {
        let mut g = self.inner.lock().await;
        g.trees.insert(info.tree, info);
        Ok(())
    }

    async fn get_tree(&self, tree: &[u8; 32]) -> StoreResult<Option<TreeInfo>> {
        let g = self.inner.lock().await;
        Ok(g.trees.get(tree).cloned())
    }

    async fn alloc_leaf_index(&self, tree: &[u8; 32]) -> StoreResult<u64> {
        let mut g = self.inner.lock().await;
        let info = g
            .trees
            .get_mut(tree)
            .ok_or_else(|| StoreError::UnknownTree {
                tree: bs58_like(tree),
            })?;
        let idx = info.num_minted;
        info.num_minted = idx + 1;
        Ok(idx)
    }

    async fn ensure_num_minted_at_least(&self, tree: &[u8; 32], floor: u64) -> StoreResult<()> {
        let mut g = self.inner.lock().await;
        let info = g
            .trees
            .get_mut(tree)
            .ok_or_else(|| StoreError::UnknownTree {
                tree: bs58_like(tree),
            })?;
        if info.num_minted < floor {
            info.num_minted = floor;
        }
        Ok(())
    }

    async fn put_leaf(&self, record: LeafRecord) -> StoreResult<()> {
        let mut g = self.inner.lock().await;
        let asset_id = record.asset_id;
        let tree = record.tree;
        let leaf_index = record.leaf_index;
        let is_new = !g.leaves_by_asset.contains_key(&asset_id);
        g.leaves_by_asset.insert(asset_id, record);
        g.leaf_index.insert((tree, leaf_index), asset_id);
        if is_new {
            g.tree_leaf_order.entry(tree).or_default().push(asset_id);
        }
        Ok(())
    }

    async fn get_leaf(&self, asset_id: &[u8; 32]) -> StoreResult<Option<LeafRecord>> {
        let g = self.inner.lock().await;
        Ok(g.leaves_by_asset.get(asset_id).cloned())
    }

    async fn get_leaf_by_index(
        &self,
        tree: &[u8; 32],
        leaf_index: u64,
    ) -> StoreResult<Option<LeafRecord>> {
        let g = self.inner.lock().await;
        if let Some(asset_id) = g.leaf_index.get(&(*tree, leaf_index)) {
            Ok(g.leaves_by_asset.get(asset_id).cloned())
        } else {
            Ok(None)
        }
    }

    async fn list_leaves(&self, tree: &[u8; 32]) -> StoreResult<Vec<LeafRecord>> {
        let g = self.inner.lock().await;
        let order = g.tree_leaf_order.get(tree).cloned().unwrap_or_default();
        Ok(order
            .into_iter()
            .filter_map(|asset_id| g.leaves_by_asset.get(&asset_id).cloned())
            .collect())
    }

    async fn get_last_signature(&self, tree: &[u8; 32]) -> StoreResult<Option<String>> {
        let g = self.inner.lock().await;
        Ok(g.last_sig.get(tree).cloned())
    }

    async fn set_last_signature(&self, tree: &[u8; 32], signature: String) -> StoreResult<()> {
        let mut g = self.inner.lock().await;
        g.last_sig.insert(*tree, signature);
        Ok(())
    }
}

/// Format a 32-byte array as a rough hex representation for error
/// messages — the service layer doesn't pull bs58 yet, and hex is
/// good enough for diagnostic output. Bs58 encoding lives at the
/// network/server boundary.
fn bs58_like(bytes: &[u8; 32]) -> String {
    let mut s = String::with_capacity(64);
    for b in bytes {
        use std::fmt::Write;
        let _ = write!(&mut s, "{b:02x}");
    }
    s
}
