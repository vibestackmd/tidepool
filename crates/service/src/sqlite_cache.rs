//! SQLite-backed `CacheStore`. Shares a `Connection` with the other
//! persistent stores via `SqliteBackend`. Tables are prefix-
//! namespaced `cache_*`; schema lives in the single
//! `sqlite_schema.sql` migration.
//!
//! `DasAsset` is stored as serde-JSON in a BLOB column — keeps the
//! schema stable across DAS type shape changes.

use std::collections::HashSet;
use std::sync::Arc;

use async_trait::async_trait;
use rusqlite::{params, Connection, OptionalExtension};
use tokio::sync::Mutex;

use crate::cache::{CacheError, CacheResult, CacheStore, SearchFilter};
use crate::das::{DasAsset, MasterEditionRecord, PrintEditionRecord};
use crate::sqlite_backend::SqliteBackend;

pub struct SqliteCache {
    conn: Arc<Mutex<Connection>>,
}

impl SqliteCache {
    #[must_use]
    pub fn new(backend: &SqliteBackend) -> Self {
        Self {
            conn: Arc::clone(&backend.conn),
        }
    }
}

#[allow(clippy::needless_pass_by_value)]
fn map_err(e: rusqlite::Error) -> CacheError {
    CacheError::Backend(e.to_string())
}

fn wipe_indexes_for(conn: &Connection, id: &str) -> rusqlite::Result<()> {
    conn.execute(
        "DELETE FROM cache_by_owner WHERE asset_id = ?1",
        params![id],
    )?;
    conn.execute(
        "DELETE FROM cache_by_authority WHERE asset_id = ?1",
        params![id],
    )?;
    conn.execute(
        "DELETE FROM cache_by_creator WHERE asset_id = ?1",
        params![id],
    )?;
    conn.execute(
        "DELETE FROM cache_by_group WHERE asset_id = ?1",
        params![id],
    )?;
    Ok(())
}

fn populate_indexes_for(conn: &Connection, asset: &DasAsset) -> rusqlite::Result<()> {
    if !asset.ownership.owner.is_empty() {
        conn.execute(
            "INSERT OR IGNORE INTO cache_by_owner(owner, asset_id) VALUES (?1, ?2)",
            params![asset.ownership.owner, asset.id],
        )?;
    }
    for auth in &asset.authorities {
        conn.execute(
            "INSERT OR IGNORE INTO cache_by_authority(authority, asset_id) VALUES (?1, ?2)",
            params![auth.address, asset.id],
        )?;
    }
    for creator in &asset.creators {
        conn.execute(
            "INSERT OR IGNORE INTO cache_by_creator(creator, asset_id) VALUES (?1, ?2)",
            params![creator.address, asset.id],
        )?;
    }
    for group in &asset.grouping {
        conn.execute(
            "INSERT OR IGNORE INTO cache_by_group(group_key, group_value, asset_id) VALUES (?1, ?2, ?3)",
            params![group.group_key, group.group_value, asset.id],
        )?;
    }
    Ok(())
}

fn assets_by_ids(conn: &Connection, ids: &[String]) -> rusqlite::Result<Vec<DasAsset>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let placeholders = ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
    let sql = format!("SELECT json FROM cache_assets WHERE id IN ({placeholders})");
    let params: Vec<&dyn rusqlite::ToSql> = ids.iter().map(|s| s as &dyn rusqlite::ToSql).collect();
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(&params[..], |row| row.get::<_, Vec<u8>>(0))?;
    let mut out = Vec::new();
    for r in rows {
        let json = r?;
        if let Ok(a) = serde_json::from_slice::<DasAsset>(&json) {
            out.push(a);
        }
    }
    Ok(out)
}

fn select_ids(conn: &Connection, sql: &str, bind: &str) -> rusqlite::Result<Vec<String>> {
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map(params![bind], |row| row.get::<_, String>(0))?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

#[async_trait]
impl CacheStore for SqliteCache {
    async fn put_asset(&self, asset: DasAsset) -> CacheResult<()> {
        let c = self.conn.lock().await;
        let json = serde_json::to_vec(&asset).map_err(|e| CacheError::Backend(e.to_string()))?;
        wipe_indexes_for(&c, &asset.id).map_err(map_err)?;
        c.execute(
            "INSERT INTO cache_assets(id, json, interface, burnt)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(id) DO UPDATE SET
                json = excluded.json,
                interface = excluded.interface,
                burnt = excluded.burnt",
            params![asset.id, json, asset.interface, i64::from(asset.burnt)],
        )
        .map_err(map_err)?;
        populate_indexes_for(&c, &asset).map_err(map_err)?;
        Ok(())
    }

    async fn get_asset(&self, id: &str) -> CacheResult<Option<DasAsset>> {
        let c = self.conn.lock().await;
        let json: Option<Vec<u8>> = c
            .query_row(
                "SELECT json FROM cache_assets WHERE id = ?1",
                params![id],
                |r| r.get(0),
            )
            .optional()
            .map_err(map_err)?;
        Ok(json.and_then(|b| serde_json::from_slice(&b).ok()))
    }

    async fn get_asset_batch(&self, ids: &[String]) -> CacheResult<Vec<Option<DasAsset>>> {
        let c = self.conn.lock().await;
        let mut out = Vec::with_capacity(ids.len());
        for id in ids {
            let json: Option<Vec<u8>> = c
                .query_row(
                    "SELECT json FROM cache_assets WHERE id = ?1",
                    params![id],
                    |r| r.get(0),
                )
                .optional()
                .map_err(map_err)?;
            out.push(json.and_then(|b| serde_json::from_slice(&b).ok()));
        }
        Ok(out)
    }

    async fn get_assets_by_owner(&self, owner: &str) -> CacheResult<Vec<DasAsset>> {
        let c = self.conn.lock().await;
        let ids = select_ids(
            &c,
            "SELECT asset_id FROM cache_by_owner WHERE owner = ?1",
            owner,
        )
        .map_err(map_err)?;
        assets_by_ids(&c, &ids).map_err(map_err)
    }

    async fn get_assets_by_authority(&self, authority: &str) -> CacheResult<Vec<DasAsset>> {
        let c = self.conn.lock().await;
        let ids = select_ids(
            &c,
            "SELECT asset_id FROM cache_by_authority WHERE authority = ?1",
            authority,
        )
        .map_err(map_err)?;
        assets_by_ids(&c, &ids).map_err(map_err)
    }

    async fn get_assets_by_creator(
        &self,
        creator: &str,
        only_verified: bool,
    ) -> CacheResult<Vec<DasAsset>> {
        let c = self.conn.lock().await;
        let ids = select_ids(
            &c,
            "SELECT asset_id FROM cache_by_creator WHERE creator = ?1",
            creator,
        )
        .map_err(map_err)?;
        let mut out = assets_by_ids(&c, &ids).map_err(map_err)?;
        if only_verified {
            out.retain(|a| {
                a.creators
                    .iter()
                    .any(|cc| cc.address == creator && cc.verified)
            });
        }
        Ok(out)
    }

    async fn get_assets_by_group(
        &self,
        group_key: &str,
        group_value: &str,
    ) -> CacheResult<Vec<DasAsset>> {
        let c = self.conn.lock().await;
        let mut stmt = c
            .prepare(
                "SELECT asset_id FROM cache_by_group WHERE group_key = ?1 AND group_value = ?2",
            )
            .map_err(map_err)?;
        let rows = stmt
            .query_map(params![group_key, group_value], |row| {
                row.get::<_, String>(0)
            })
            .map_err(map_err)?;
        let mut ids = Vec::new();
        for r in rows {
            ids.push(r.map_err(map_err)?);
        }
        assets_by_ids(&c, &ids).map_err(map_err)
    }

    async fn search_assets(&self, filter: &SearchFilter) -> CacheResult<Vec<DasAsset>> {
        fn narrow(acc: &mut Option<HashSet<String>>, next: Vec<String>) {
            let next: HashSet<String> = next.into_iter().collect();
            match acc {
                Some(existing) => existing.retain(|id| next.contains(id)),
                None => *acc = Some(next),
            }
        }

        let c = self.conn.lock().await;
        let mut candidates: Option<HashSet<String>> = None;

        if let Some(owner) = &filter.owner_address {
            let ids = select_ids(
                &c,
                "SELECT asset_id FROM cache_by_owner WHERE owner = ?1",
                owner,
            )
            .map_err(map_err)?;
            narrow(&mut candidates, ids);
        }
        if let Some(auth) = &filter.authority_address {
            let ids = select_ids(
                &c,
                "SELECT asset_id FROM cache_by_authority WHERE authority = ?1",
                auth,
            )
            .map_err(map_err)?;
            narrow(&mut candidates, ids);
        }
        if let Some(creator) = &filter.creator_address {
            let ids = select_ids(
                &c,
                "SELECT asset_id FROM cache_by_creator WHERE creator = ?1",
                creator,
            )
            .map_err(map_err)?;
            narrow(&mut candidates, ids);
        }
        if let Some((gk, gv)) = &filter.grouping {
            let mut stmt = c
                .prepare(
                    "SELECT asset_id FROM cache_by_group WHERE group_key = ?1 AND group_value = ?2",
                )
                .map_err(map_err)?;
            let rows = stmt
                .query_map(params![gk, gv], |row| row.get::<_, String>(0))
                .map_err(map_err)?;
            let mut ids = Vec::new();
            for r in rows {
                ids.push(r.map_err(map_err)?);
            }
            narrow(&mut candidates, ids);
        }

        let ids: Vec<String> = if let Some(s) = candidates {
            s.into_iter().collect()
        } else {
            let mut stmt = c.prepare("SELECT id FROM cache_assets").map_err(map_err)?;
            let rows = stmt
                .query_map([], |r| r.get::<_, String>(0))
                .map_err(map_err)?;
            let mut v = Vec::new();
            for r in rows {
                v.push(r.map_err(map_err)?);
            }
            v
        };
        let mut out = assets_by_ids(&c, &ids).map_err(map_err)?;

        if let Some(iface) = &filter.interface {
            out.retain(|a| &a.interface == iface);
        }
        if let Some(want_burnt) = filter.burnt {
            out.retain(|a| a.burnt == want_burnt);
        }
        if let (Some(creator), Some(true)) = (&filter.creator_address, filter.creator_verified) {
            out.retain(|a| {
                a.creators
                    .iter()
                    .any(|c| &c.address == creator && c.verified)
            });
        }
        Ok(out)
    }

    async fn put_master_edition(&self, record: MasterEditionRecord) -> CacheResult<()> {
        let c = self.conn.lock().await;
        c.execute(
            "INSERT INTO cache_master_editions(master_mint, master_edition_pda, supply, max_supply)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(master_mint) DO UPDATE SET
                master_edition_pda = excluded.master_edition_pda,
                supply = excluded.supply,
                max_supply = excluded.max_supply",
            params![
                record.master_mint,
                record.master_edition_pda,
                i64::try_from(record.supply).unwrap_or(i64::MAX),
                record
                    .max_supply
                    .map(|v| i64::try_from(v).unwrap_or(i64::MAX)),
            ],
        )
        .map_err(map_err)?;
        Ok(())
    }

    async fn put_print_edition(&self, record: PrintEditionRecord) -> CacheResult<()> {
        let c = self.conn.lock().await;
        c.execute(
            "INSERT INTO cache_print_editions(print_mint, print_edition_pda, parent_master_edition_pda, edition_num)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(print_mint) DO UPDATE SET
                print_edition_pda = excluded.print_edition_pda,
                parent_master_edition_pda = excluded.parent_master_edition_pda,
                edition_num = excluded.edition_num",
            params![
                record.print_mint,
                record.print_edition_pda,
                record.parent_master_edition_pda,
                i64::try_from(record.edition_num).unwrap_or(i64::MAX),
            ],
        )
        .map_err(map_err)?;
        Ok(())
    }

    async fn get_master_edition(&self, mint: &str) -> CacheResult<Option<MasterEditionRecord>> {
        let c = self.conn.lock().await;
        c.query_row(
            "SELECT master_mint, master_edition_pda, supply, max_supply FROM cache_master_editions WHERE master_mint = ?1",
            params![mint],
            |r| {
                let supply: i64 = r.get(2)?;
                let max_supply: Option<i64> = r.get(3)?;
                Ok(MasterEditionRecord {
                    master_mint: r.get(0)?,
                    master_edition_pda: r.get(1)?,
                    supply: u64::try_from(supply).unwrap_or(0),
                    max_supply: max_supply.map(|v| u64::try_from(v).unwrap_or(0)),
                })
            },
        )
        .optional()
        .map_err(map_err)
    }

    async fn list_print_editions(
        &self,
        master_edition_pda: &str,
    ) -> CacheResult<Vec<PrintEditionRecord>> {
        let c = self.conn.lock().await;
        let mut stmt = c
            .prepare(
                "SELECT print_mint, print_edition_pda, parent_master_edition_pda, edition_num
                 FROM cache_print_editions
                 WHERE parent_master_edition_pda = ?1
                 ORDER BY edition_num ASC",
            )
            .map_err(map_err)?;
        let rows = stmt
            .query_map(params![master_edition_pda], |r| {
                let edition_num: i64 = r.get(3)?;
                Ok(PrintEditionRecord {
                    print_mint: r.get(0)?,
                    print_edition_pda: r.get(1)?,
                    parent_master_edition_pda: r.get(2)?,
                    edition_num: u64::try_from(edition_num).unwrap_or(0),
                })
            })
            .map_err(map_err)?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r.map_err(map_err)?);
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::das::{
        DasAuthority, DasContent, DasCreator, DasGrouping, DasLinks, DasMetadata, DasOwnership,
    };

    fn backend() -> SqliteBackend {
        SqliteBackend::open_in_memory().unwrap()
    }

    fn stub_asset(id: &str, owner: &str) -> DasAsset {
        DasAsset {
            id: id.into(),
            interface: "V1_NFT".into(),
            content: DasContent {
                metadata: DasMetadata::default(),
                links: DasLinks::default(),
                ..Default::default()
            },
            authorities: vec![DasAuthority {
                address: "AUTH_A".into(),
                scopes: vec!["full".into()],
            }],
            creators: vec![DasCreator {
                address: "CREATOR_A".into(),
                share: 100,
                verified: true,
            }],
            ownership: DasOwnership {
                ownership_model: "single".into(),
                owner: owner.into(),
                ..Default::default()
            },
            grouping: vec![DasGrouping {
                group_key: "collection".into(),
                group_value: "COLL_A".into(),
            }],
            mutable: true,
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn asset_round_trips_through_sqlite() {
        let c = SqliteCache::new(&backend());
        c.put_asset(stub_asset("A1", "OWNER_A")).await.unwrap();
        let got = c.get_asset("A1").await.unwrap().unwrap();
        assert_eq!(got.id, "A1");
        assert_eq!(got.ownership.owner, "OWNER_A");
    }

    #[tokio::test]
    async fn by_owner_index_populates_and_reindexes_on_reput() {
        let c = SqliteCache::new(&backend());
        c.put_asset(stub_asset("A1", "OWNER_A")).await.unwrap();
        assert_eq!(c.get_assets_by_owner("OWNER_A").await.unwrap().len(), 1);
        c.put_asset(stub_asset("A1", "OWNER_B")).await.unwrap();
        assert!(c.get_assets_by_owner("OWNER_A").await.unwrap().is_empty());
        assert_eq!(c.get_assets_by_owner("OWNER_B").await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn search_narrows_by_multiple_indexes() {
        let c = SqliteCache::new(&backend());
        let mut a1 = stub_asset("A1", "OWNER_A");
        a1.grouping[0].group_value = "COLL_X".into();
        let mut a2 = stub_asset("A2", "OWNER_A");
        a2.grouping[0].group_value = "COLL_Y".into();
        c.put_asset(a1).await.unwrap();
        c.put_asset(a2).await.unwrap();

        let filter = SearchFilter {
            owner_address: Some("OWNER_A".into()),
            grouping: Some(("collection".into(), "COLL_X".into())),
            ..Default::default()
        };
        let got = c.search_assets(&filter).await.unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].id, "A1");
    }

    #[tokio::test]
    async fn edition_round_trip() {
        let c = SqliteCache::new(&backend());
        c.put_master_edition(MasterEditionRecord {
            master_mint: "MASTER".into(),
            master_edition_pda: "MASTER_PDA".into(),
            supply: 3,
            max_supply: Some(100),
        })
        .await
        .unwrap();
        c.put_print_edition(PrintEditionRecord {
            print_mint: "PRINT2".into(),
            print_edition_pda: "PRINT2_PDA".into(),
            parent_master_edition_pda: "MASTER_PDA".into(),
            edition_num: 2,
        })
        .await
        .unwrap();
        c.put_print_edition(PrintEditionRecord {
            print_mint: "PRINT1".into(),
            print_edition_pda: "PRINT1_PDA".into(),
            parent_master_edition_pda: "MASTER_PDA".into(),
            edition_num: 1,
        })
        .await
        .unwrap();
        let master = c.get_master_edition("MASTER").await.unwrap().unwrap();
        assert_eq!(master.supply, 3);
        let prints = c.list_print_editions("MASTER_PDA").await.unwrap();
        assert_eq!(prints.len(), 2);
        assert_eq!(prints[0].edition_num, 1);
        assert_eq!(prints[1].edition_num, 2);
    }
}
