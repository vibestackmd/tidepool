//! Tree snapshot dump + load.
//!
//! A `TreeSnapshot` is a serializable capture of one Bubblegum tree's
//! full indexed state — `TreeInfo`, every `LeafRecord` in insertion
//! order, and the indexer's `last_signature` cursor. Intended flow:
//!
//! 1. Backfill a tree once (via `--index-tree` or `tidepoolIndexTree`).
//! 2. Call `tidepoolDumpTree` to export the `TreeSnapshot` JSON.
//! 3. Commit the JSON to the repo or keep it as a fixture.
//! 4. On fresh boots, `tidepoolLoadTree` applies the snapshot
//!    directly — no re-paging `getSignaturesForAddress` or re-parsing
//!    hundreds of `getTransaction` responses.
//!
//! Works with any `CnftStore` backend (memory, SQLite, future impls).
//! The snapshot format is the source of truth; if the memory + SQLite
//! backends ever drift, this module surfaces the disagreement.

use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use serde::{Deserialize, Serialize};

use super::store::{CnftStore, StoreError};
use super::types::{LeafRecord, TreeInfo};

/// A portable snapshot of one tree's indexed state. JSON-safe — every
/// 32-byte field serializes as base58 at the wire layer when exposed
/// via RPC, but in-memory we keep byte arrays so round-trips don't pay
/// encoding overhead. The RPC handler does the conversion.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TreeSnapshot {
    /// Snapshot format version. Bumping lets future reads reject or
    /// migrate older payloads cleanly.
    pub format_version: u32,
    pub tree_info: TreeInfo,
    pub leaves: Vec<LeafRecord>,
    /// Last `getSignaturesForAddress` cursor when the dump was taken.
    /// Loaders use this to resume incremental indexing instead of
    /// replaying history the snapshot already contains.
    pub last_signature: Option<String>,
}

pub const SNAPSHOT_FORMAT_VERSION: u32 = 1;

/// Dump the full indexed state for `tree` from `store`. Returns
/// `Ok(None)` when the tree isn't registered.
pub async fn dump_tree<S: CnftStore + ?Sized>(
    store: &S,
    tree: &[u8; 32],
) -> Result<Option<TreeSnapshot>, StoreError> {
    let Some(tree_info) = store.get_tree(tree).await? else {
        return Ok(None);
    };
    let leaves = store.list_leaves(tree).await?;
    let last_signature = store.get_last_signature(tree).await?;
    Ok(Some(TreeSnapshot {
        format_version: SNAPSHOT_FORMAT_VERSION,
        tree_info,
        leaves,
        last_signature,
    }))
}

/// Apply a snapshot to `store`. Overwrites any existing state for the
/// tree. Returns an error if the snapshot's `format_version` is newer
/// than we understand (future-compat guard).
pub async fn load_tree<S: CnftStore + ?Sized>(
    store: &S,
    snapshot: TreeSnapshot,
) -> Result<LoadSummary, StoreError> {
    if snapshot.format_version > SNAPSHOT_FORMAT_VERSION {
        return Err(StoreError::UnknownTree {
            tree: format!(
                "snapshot format_version {} is newer than supported ({})",
                snapshot.format_version, SNAPSHOT_FORMAT_VERSION
            ),
        });
    }
    let tree = snapshot.tree_info.tree;
    let leaf_count = snapshot.leaves.len();

    store.put_tree(snapshot.tree_info).await?;
    for leaf in snapshot.leaves {
        store.put_leaf(leaf).await?;
    }
    if let Some(sig) = snapshot.last_signature {
        store.set_last_signature(&tree, sig).await?;
    }

    Ok(LoadSummary {
        tree,
        leaf_count: leaf_count as u64,
    })
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LoadSummary {
    pub tree: [u8; 32],
    pub leaf_count: u64,
}

/// Discriminator for snapshot kind. Today only `Tree` exists; adding
/// new kinds (cache, webhooks) slots in without breaking the
/// envelope or the CLI `--snapshot` loader.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SnapshotKind {
    Tree,
}

/// Wire-friendly envelope for snapshot transfer over JSON-RPC and
/// on-disk `--snapshot` files. Raw `TreeSnapshot` serde'd to JSON uses
/// arrays-of-numbers for `[u8; 32]` fields, which is verbose and
/// opaque to clients. This envelope wraps the snapshot in a single
/// base64 blob so payloads stay compact and the internal shape can
/// evolve without breaking wire compat.
///
/// `kind` is the dispatch discriminator for future snapshot types.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotBlob {
    pub kind: SnapshotKind,
    pub format_version: u32,
    /// Base64-encoded, serde-JSON-serialized kind-specific payload.
    /// For `SnapshotKind::Tree` that's a `TreeSnapshot`.
    pub data: String,
}

impl SnapshotBlob {
    #[must_use]
    pub fn from_tree(snapshot: &TreeSnapshot) -> Self {
        let json = serde_json::to_vec(snapshot).unwrap_or_default();
        Self {
            kind: SnapshotKind::Tree,
            format_version: snapshot.format_version,
            data: B64.encode(json),
        }
    }

    /// Decode into the kind-specific payload. Returns an error if the
    /// envelope's `kind` doesn't match what was expected.
    pub fn into_tree_snapshot(self) -> Result<TreeSnapshot, String> {
        if self.kind != SnapshotKind::Tree {
            return Err(format!("expected SnapshotKind::Tree, got {:?}", self.kind));
        }
        let bytes = B64
            .decode(self.data.as_bytes())
            .map_err(|e| format!("base64 decode: {e}"))?;
        serde_json::from_slice(&bytes).map_err(|e| format!("snapshot deserialize: {e}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cnft::store::MemoryCnftStore;
    use crate::cnft::types::MintMetadata;
    use tidepool_rpc_core::Creator;

    const TREE: [u8; 32] = [0x11; 32];

    fn stub_metadata() -> MintMetadata {
        MintMetadata {
            name: "Asset".into(),
            symbol: "A".into(),
            uri: "https://example.com/a.json".into(),
            seller_fee_basis_points: 500,
            primary_sale_happened: false,
            is_mutable: true,
            creators: vec![Creator {
                address: [0x44; 32],
                verified: true,
                share: 100,
            }],
            collection: None,
            data_hash_input: vec![0xaa; 16],
        }
    }

    fn stub_leaf(index: u64) -> LeafRecord {
        let i = u8::try_from(index).unwrap_or(0);
        LeafRecord {
            asset_id: [i + 1; 32],
            tree: TREE,
            nonce: index,
            leaf_index: index,
            mint_metadata: stub_metadata(),
            owner: [i + 2; 32],
            delegate: [i + 3; 32],
            data_hash: [i + 4; 32],
            creator_hash: [i + 5; 32],
            leaf_hash: [i + 6; 32],
            burned: false,
        }
    }

    async fn seed_store() -> MemoryCnftStore {
        let s = MemoryCnftStore::new();
        s.put_tree(TreeInfo {
            tree: TREE,
            depth: 20,
            max_buffer_size: 64,
            num_minted: 3,
        })
        .await
        .unwrap();
        for i in 0..3 {
            s.put_leaf(stub_leaf(i)).await.unwrap();
        }
        s.set_last_signature(&TREE, "CURSOR_SIG".into())
            .await
            .unwrap();
        s
    }

    #[tokio::test]
    async fn dump_captures_tree_leaves_and_cursor() {
        let s = seed_store().await;
        let snap = dump_tree(&s, &TREE).await.unwrap().expect("Some");
        assert_eq!(snap.format_version, SNAPSHOT_FORMAT_VERSION);
        assert_eq!(snap.tree_info.depth, 20);
        assert_eq!(snap.tree_info.num_minted, 3);
        assert_eq!(snap.leaves.len(), 3);
        assert_eq!(snap.leaves[0].leaf_index, 0);
        assert_eq!(snap.leaves[2].leaf_index, 2);
        assert_eq!(snap.last_signature.as_deref(), Some("CURSOR_SIG"));
    }

    #[tokio::test]
    async fn dump_unknown_tree_returns_none() {
        let s = MemoryCnftStore::new();
        assert!(dump_tree(&s, &[0x99; 32]).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn load_applies_snapshot_to_fresh_store() {
        let src = seed_store().await;
        let snap = dump_tree(&src, &TREE).await.unwrap().unwrap();

        let dst = MemoryCnftStore::new();
        let summary = load_tree(&dst, snap).await.unwrap();
        assert_eq!(summary.leaf_count, 3);
        assert_eq!(summary.tree, TREE);

        // Verify: every leaf + tree info + cursor round-tripped.
        let got_tree = dst.get_tree(&TREE).await.unwrap().expect("present");
        assert_eq!(got_tree.num_minted, 3);
        let got_leaves = dst.list_leaves(&TREE).await.unwrap();
        assert_eq!(got_leaves.len(), 3);
        assert_eq!(
            dst.get_last_signature(&TREE).await.unwrap().as_deref(),
            Some("CURSOR_SIG")
        );
    }

    #[tokio::test]
    async fn load_rejects_future_format_version() {
        let dst = MemoryCnftStore::new();
        let bad = TreeSnapshot {
            format_version: SNAPSHOT_FORMAT_VERSION + 1,
            tree_info: TreeInfo {
                tree: TREE,
                depth: 1,
                max_buffer_size: 1,
                num_minted: 0,
            },
            leaves: vec![],
            last_signature: None,
        };
        assert!(load_tree(&dst, bad).await.is_err());
    }

    #[tokio::test]
    async fn snapshot_blob_round_trip() {
        let src = seed_store().await;
        let snap = dump_tree(&src, &TREE).await.unwrap().unwrap();
        let blob = SnapshotBlob::from_tree(&snap);
        assert_eq!(blob.kind, SnapshotKind::Tree);
        let decoded = blob.into_tree_snapshot().expect("decode");
        assert_eq!(decoded, snap);
    }
}
