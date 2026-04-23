//! SQLite-backed `CnftStore`. Shares a `Connection` with the other
//! persistent stores (DAS cache, webhook registry) via
//! `SqliteBackend`. Surfpool-style: one `--db path.sqlite` file holds
//! everything.
//!
//! Tables prefixed `cnft_*` so co-tenanting is obvious in `.tables`.
//! Concurrency: async methods hold the shared `Mutex<Connection>`
//! for the duration of the DB call — fine at local-dev QPS; worth
//! `spawn_blocking` if it ever shows up in profiling.

use std::sync::Arc;

use async_trait::async_trait;
use rusqlite::{params, Connection, OptionalExtension};
use tokio::sync::Mutex;

use super::store::{CnftStore, StoreError, StoreResult};
use super::types::{LeafRecord, MintMetadata, TreeInfo};
use crate::sqlite_backend::SqliteBackend;

pub struct SqliteCnftStore {
    conn: Arc<Mutex<Connection>>,
}

impl SqliteCnftStore {
    /// Wrap a shared backend. All stores created from the same
    /// backend share one `Connection`.
    #[must_use]
    pub fn new(backend: &SqliteBackend) -> Self {
        Self {
            conn: Arc::clone(&backend.conn),
        }
    }
}

#[allow(clippy::needless_pass_by_value)]
fn map_sqlite_err(e: rusqlite::Error) -> StoreError {
    StoreError::UnknownTree {
        tree: format!("sqlite: {e}"),
    }
}

fn row_to_tree(row: &rusqlite::Row<'_>) -> rusqlite::Result<TreeInfo> {
    let tree: Vec<u8> = row.get(0)?;
    let depth: i64 = row.get(1)?;
    let max_buffer_size: i64 = row.get(2)?;
    let num_minted: i64 = row.get(3)?;
    let mut tree_arr = [0u8; 32];
    tree_arr.copy_from_slice(&tree);
    Ok(TreeInfo {
        tree: tree_arr,
        depth: u8::try_from(depth).unwrap_or(0),
        max_buffer_size: u32::try_from(max_buffer_size).unwrap_or(0),
        num_minted: u64::try_from(num_minted).unwrap_or(0),
    })
}

fn row_to_leaf(row: &rusqlite::Row<'_>) -> rusqlite::Result<LeafRecord> {
    let asset_id: Vec<u8> = row.get("asset_id")?;
    let tree: Vec<u8> = row.get("tree")?;
    let nonce: i64 = row.get("nonce")?;
    let leaf_index: i64 = row.get("leaf_index")?;
    let owner: Vec<u8> = row.get("owner")?;
    let delegate: Vec<u8> = row.get("delegate")?;
    let data_hash: Vec<u8> = row.get("data_hash")?;
    let creator_hash: Vec<u8> = row.get("creator_hash")?;
    let leaf_hash: Vec<u8> = row.get("leaf_hash")?;
    let burned: i64 = row.get("burned")?;
    let mint_metadata_json: Vec<u8> = row.get("mint_metadata_json")?;
    let mint_metadata: MintMetadata = serde_json::from_slice(&mint_metadata_json).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Blob, Box::new(e))
    })?;
    Ok(LeafRecord {
        asset_id: to_32(&asset_id),
        tree: to_32(&tree),
        nonce: u64::try_from(nonce).unwrap_or(0),
        leaf_index: u64::try_from(leaf_index).unwrap_or(0),
        mint_metadata,
        owner: to_32(&owner),
        delegate: to_32(&delegate),
        data_hash: to_32(&data_hash),
        creator_hash: to_32(&creator_hash),
        leaf_hash: to_32(&leaf_hash),
        burned: burned != 0,
    })
}

fn to_32(bytes: &[u8]) -> [u8; 32] {
    let mut out = [0u8; 32];
    let n = bytes.len().min(32);
    out[..n].copy_from_slice(&bytes[..n]);
    out
}

#[async_trait]
impl CnftStore for SqliteCnftStore {
    async fn put_tree(&self, info: TreeInfo) -> StoreResult<()> {
        let c = self.conn.lock().await;
        c.execute(
            "INSERT INTO cnft_trees(tree_pubkey, depth, max_buffer_size, num_minted)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(tree_pubkey) DO UPDATE SET
                depth = excluded.depth,
                max_buffer_size = excluded.max_buffer_size,
                num_minted = excluded.num_minted",
            params![
                info.tree.as_slice(),
                i64::from(info.depth),
                i64::from(info.max_buffer_size),
                i64::try_from(info.num_minted).unwrap_or(i64::MAX),
            ],
        )
        .map_err(map_sqlite_err)?;
        Ok(())
    }

    async fn get_tree(&self, tree: &[u8; 32]) -> StoreResult<Option<TreeInfo>> {
        let c = self.conn.lock().await;
        c.query_row(
            "SELECT tree_pubkey, depth, max_buffer_size, num_minted FROM cnft_trees WHERE tree_pubkey = ?1",
            params![tree.as_slice()],
            row_to_tree,
        )
        .optional()
        .map_err(map_sqlite_err)
    }

    async fn alloc_leaf_index(&self, tree: &[u8; 32]) -> StoreResult<u64> {
        let c = self.conn.lock().await;
        let new: Option<i64> = c
            .query_row(
                "UPDATE cnft_trees SET num_minted = num_minted + 1
                 WHERE tree_pubkey = ?1
                 RETURNING num_minted - 1",
                params![tree.as_slice()],
                |r| r.get(0),
            )
            .optional()
            .map_err(map_sqlite_err)?;
        new.map(|n| u64::try_from(n).unwrap_or(0))
            .ok_or_else(|| StoreError::UnknownTree { tree: hex_of(tree) })
    }

    async fn ensure_num_minted_at_least(&self, tree: &[u8; 32], floor: u64) -> StoreResult<()> {
        let c = self.conn.lock().await;
        let changed = c
            .execute(
                "UPDATE cnft_trees SET num_minted = ?1 WHERE tree_pubkey = ?2 AND num_minted < ?1",
                params![i64::try_from(floor).unwrap_or(i64::MAX), tree.as_slice()],
            )
            .map_err(map_sqlite_err)?;
        if changed == 0 {
            let exists: bool = c
                .query_row(
                    "SELECT 1 FROM cnft_trees WHERE tree_pubkey = ?1",
                    params![tree.as_slice()],
                    |_| Ok(true),
                )
                .optional()
                .map_err(map_sqlite_err)?
                .unwrap_or(false);
            if !exists {
                return Err(StoreError::UnknownTree { tree: hex_of(tree) });
            }
        }
        Ok(())
    }

    async fn put_leaf(&self, record: LeafRecord) -> StoreResult<()> {
        let c = self.conn.lock().await;
        let metadata_json =
            serde_json::to_vec(&record.mint_metadata).map_err(|e| StoreError::UnknownTree {
                tree: format!("serialize metadata: {e}"),
            })?;
        c.execute(
            "INSERT INTO cnft_leaves(asset_id, tree, nonce, leaf_index, owner, delegate,
                                data_hash, creator_hash, leaf_hash, burned,
                                mint_metadata_json)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
             ON CONFLICT(asset_id) DO UPDATE SET
                tree = excluded.tree,
                nonce = excluded.nonce,
                leaf_index = excluded.leaf_index,
                owner = excluded.owner,
                delegate = excluded.delegate,
                data_hash = excluded.data_hash,
                creator_hash = excluded.creator_hash,
                leaf_hash = excluded.leaf_hash,
                burned = excluded.burned,
                mint_metadata_json = excluded.mint_metadata_json",
            params![
                record.asset_id.as_slice(),
                record.tree.as_slice(),
                i64::try_from(record.nonce).unwrap_or(i64::MAX),
                i64::try_from(record.leaf_index).unwrap_or(i64::MAX),
                record.owner.as_slice(),
                record.delegate.as_slice(),
                record.data_hash.as_slice(),
                record.creator_hash.as_slice(),
                record.leaf_hash.as_slice(),
                i64::from(record.burned),
                metadata_json,
            ],
        )
        .map_err(map_sqlite_err)?;
        Ok(())
    }

    async fn get_leaf(&self, asset_id: &[u8; 32]) -> StoreResult<Option<LeafRecord>> {
        let c = self.conn.lock().await;
        c.query_row(
            "SELECT asset_id, tree, nonce, leaf_index, owner, delegate,
                    data_hash, creator_hash, leaf_hash, burned, mint_metadata_json
             FROM cnft_leaves WHERE asset_id = ?1",
            params![asset_id.as_slice()],
            row_to_leaf,
        )
        .optional()
        .map_err(map_sqlite_err)
    }

    async fn get_leaf_by_index(
        &self,
        tree: &[u8; 32],
        leaf_index: u64,
    ) -> StoreResult<Option<LeafRecord>> {
        let c = self.conn.lock().await;
        c.query_row(
            "SELECT asset_id, tree, nonce, leaf_index, owner, delegate,
                    data_hash, creator_hash, leaf_hash, burned, mint_metadata_json
             FROM cnft_leaves WHERE tree = ?1 AND leaf_index = ?2",
            params![
                tree.as_slice(),
                i64::try_from(leaf_index).unwrap_or(i64::MAX)
            ],
            row_to_leaf,
        )
        .optional()
        .map_err(map_sqlite_err)
    }

    async fn list_leaves(&self, tree: &[u8; 32]) -> StoreResult<Vec<LeafRecord>> {
        let c = self.conn.lock().await;
        let mut stmt = c
            .prepare(
                "SELECT asset_id, tree, nonce, leaf_index, owner, delegate,
                        data_hash, creator_hash, leaf_hash, burned, mint_metadata_json
                 FROM cnft_leaves WHERE tree = ?1 ORDER BY rowid_alias ASC",
            )
            .map_err(map_sqlite_err)?;
        let rows = stmt
            .query_map(params![tree.as_slice()], row_to_leaf)
            .map_err(map_sqlite_err)?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r.map_err(map_sqlite_err)?);
        }
        Ok(out)
    }

    async fn get_last_signature(&self, tree: &[u8; 32]) -> StoreResult<Option<String>> {
        let c = self.conn.lock().await;
        c.query_row(
            "SELECT signature FROM cnft_last_sig WHERE tree = ?1",
            params![tree.as_slice()],
            |r| r.get::<_, String>(0),
        )
        .optional()
        .map_err(map_sqlite_err)
    }

    async fn set_last_signature(&self, tree: &[u8; 32], signature: String) -> StoreResult<()> {
        let c = self.conn.lock().await;
        c.execute(
            "INSERT INTO cnft_last_sig(tree, signature) VALUES (?1, ?2)
             ON CONFLICT(tree) DO UPDATE SET signature = excluded.signature",
            params![tree.as_slice(), signature],
        )
        .map_err(map_sqlite_err)?;
        Ok(())
    }
}

fn hex_of(bytes: &[u8; 32]) -> String {
    let mut s = String::with_capacity(64);
    for b in bytes {
        use std::fmt::Write;
        let _ = write!(&mut s, "{b:02x}");
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use tidepool_rpc_core::Creator;

    fn backend() -> SqliteBackend {
        SqliteBackend::open_in_memory().unwrap()
    }

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
            data_hash_input: vec![0xaa; 32],
        }
    }

    #[tokio::test]
    async fn put_and_get_tree() {
        let s = SqliteCnftStore::new(&backend());
        s.put_tree(TreeInfo {
            tree: [0x11; 32],
            depth: 20,
            max_buffer_size: 64,
            num_minted: 3,
        })
        .await
        .unwrap();
        let got = s.get_tree(&[0x11; 32]).await.unwrap().expect("present");
        assert_eq!(got.depth, 20);
        assert_eq!(got.num_minted, 3);
    }

    #[tokio::test]
    async fn alloc_leaf_index_is_monotonic() {
        let s = SqliteCnftStore::new(&backend());
        s.put_tree(TreeInfo {
            tree: [1; 32],
            depth: 8,
            max_buffer_size: 16,
            num_minted: 0,
        })
        .await
        .unwrap();
        assert_eq!(s.alloc_leaf_index(&[1; 32]).await.unwrap(), 0);
        assert_eq!(s.alloc_leaf_index(&[1; 32]).await.unwrap(), 1);
        assert_eq!(s.alloc_leaf_index(&[1; 32]).await.unwrap(), 2);
    }

    #[tokio::test]
    async fn alloc_on_unknown_tree_errors() {
        let s = SqliteCnftStore::new(&backend());
        assert!(matches!(
            s.alloc_leaf_index(&[9; 32]).await,
            Err(StoreError::UnknownTree { .. })
        ));
    }

    #[tokio::test]
    async fn put_and_list_leaves_in_insertion_order() {
        let s = SqliteCnftStore::new(&backend());
        s.put_tree(TreeInfo {
            tree: [1; 32],
            depth: 8,
            max_buffer_size: 16,
            num_minted: 0,
        })
        .await
        .unwrap();
        for i in 0u8..3 {
            let rec = LeafRecord {
                asset_id: [i + 1; 32],
                tree: [1; 32],
                nonce: u64::from(i),
                leaf_index: u64::from(i),
                mint_metadata: stub_metadata(),
                owner: [0; 32],
                delegate: [0; 32],
                data_hash: [0; 32],
                creator_hash: [0; 32],
                leaf_hash: [i + 1; 32],
                burned: false,
            };
            s.put_leaf(rec).await.unwrap();
        }
        let listed = s.list_leaves(&[1; 32]).await.unwrap();
        assert_eq!(listed.len(), 3);
        assert_eq!(listed[0].asset_id[0], 1);
        assert_eq!(listed[2].asset_id[0], 3);
    }

    #[tokio::test]
    async fn get_leaf_by_index_finds_across_updates() {
        let s = SqliteCnftStore::new(&backend());
        s.put_tree(TreeInfo {
            tree: [1; 32],
            depth: 8,
            max_buffer_size: 16,
            num_minted: 0,
        })
        .await
        .unwrap();
        let mut rec = LeafRecord {
            asset_id: [2; 32],
            tree: [1; 32],
            nonce: 0,
            leaf_index: 0,
            mint_metadata: stub_metadata(),
            owner: [0xaa; 32],
            delegate: [0; 32],
            data_hash: [0; 32],
            creator_hash: [0; 32],
            leaf_hash: [2; 32],
            burned: false,
        };
        s.put_leaf(rec.clone()).await.unwrap();
        rec.owner = [0xbb; 32];
        s.put_leaf(rec).await.unwrap();
        let got = s.get_leaf_by_index(&[1; 32], 0).await.unwrap().unwrap();
        assert_eq!(got.owner, [0xbb; 32], "upsert path applied");
    }

    #[tokio::test]
    async fn last_signature_round_trip() {
        let s = SqliteCnftStore::new(&backend());
        assert!(s.get_last_signature(&[1; 32]).await.unwrap().is_none());
        s.set_last_signature(&[1; 32], "SIG1".into()).await.unwrap();
        assert_eq!(
            s.get_last_signature(&[1; 32]).await.unwrap().as_deref(),
            Some("SIG1")
        );
        s.set_last_signature(&[1; 32], "SIG2".into()).await.unwrap();
        assert_eq!(
            s.get_last_signature(&[1; 32]).await.unwrap().as_deref(),
            Some("SIG2")
        );
    }

    #[tokio::test]
    async fn persistence_survives_across_opens() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let path = tmp.path().to_path_buf();
        drop(tmp);

        {
            let s = SqliteCnftStore::new(&SqliteBackend::open(&path).unwrap());
            s.put_tree(TreeInfo {
                tree: [0x22; 32],
                depth: 10,
                max_buffer_size: 32,
                num_minted: 5,
            })
            .await
            .unwrap();
        }

        let s = SqliteCnftStore::new(&SqliteBackend::open(&path).unwrap());
        let got = s.get_tree(&[0x22; 32]).await.unwrap().expect("persisted");
        assert_eq!(got.depth, 10);
        assert_eq!(got.num_minted, 5);

        let _ = std::fs::remove_file(&path);
    }
}
