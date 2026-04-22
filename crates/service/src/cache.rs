//! Cache layer for decoded DAS assets. Populated as a side effect of
//! `fetch_and_cache_asset`; queried by every by-X handler
//! (`get_assets_by_owner` / `_group` / `_authority` / `_creator`) and
//! by `search_assets`.
//!
//! Secondary indexes are built on `put_asset`:
//!   - owner → [id]
//!   - each authority.address → [id]
//!   - each grouping `(group_key, group_value)` → [id]
//!   - each creator.address → [id]
//!
//! The in-memory impl wraps a single tokio `Mutex`. Fine at our scale
//! (tens of thousands of assets, reads + writes in microseconds).
//! Swap to a sharded map or SQLite later if contention shows up.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use async_trait::async_trait;
use thiserror::Error;
use tokio::sync::Mutex;

use crate::das::DasAsset;

#[derive(Debug, Error)]
pub enum CacheError {
    // Placeholder for real backend errors (e.g. SQLite). Memory impl
    // currently never errors, but callers code against this enum so
    // future backends slot in transparently.
    #[error("cache backend error: {0}")]
    Backend(String),
}

pub type CacheResult<T> = Result<T, CacheError>;

/// Filter knobs for `search_assets`. Mirrors the TS shape + Helius
/// DAS docs. All fields are optional and ANDed together.
#[derive(Debug, Clone, Default)]
pub struct SearchFilter {
    pub owner_address: Option<String>,
    pub authority_address: Option<String>,
    pub creator_address: Option<String>,
    /// Only return creators whose `verified` flag is true. Has no
    /// effect without `creator_address`.
    pub creator_verified: Option<bool>,
    /// `(group_key, group_value)` — typically `("collection", <pk>)`.
    pub grouping: Option<(String, String)>,
    pub interface: Option<String>,
    pub burnt: Option<bool>,
}

#[async_trait]
pub trait CacheStore: Send + Sync {
    async fn put_asset(&self, asset: DasAsset) -> CacheResult<()>;
    async fn get_asset(&self, id: &str) -> CacheResult<Option<DasAsset>>;
    async fn get_asset_batch(&self, ids: &[String]) -> CacheResult<Vec<Option<DasAsset>>>;

    async fn get_assets_by_owner(&self, owner: &str) -> CacheResult<Vec<DasAsset>>;
    async fn get_assets_by_authority(&self, authority: &str) -> CacheResult<Vec<DasAsset>>;
    async fn get_assets_by_creator(
        &self,
        creator: &str,
        only_verified: bool,
    ) -> CacheResult<Vec<DasAsset>>;
    async fn get_assets_by_group(
        &self,
        group_key: &str,
        group_value: &str,
    ) -> CacheResult<Vec<DasAsset>>;

    async fn search_assets(&self, filter: &SearchFilter) -> CacheResult<Vec<DasAsset>>;
}

#[derive(Default)]
pub struct MemoryCache {
    inner: Arc<Mutex<MemoryCacheInner>>,
}

#[derive(Default)]
#[allow(clippy::struct_field_names)] // all fields share "by_" prefix — intentional: secondary-index naming convention.
struct MemoryCacheInner {
    by_id: HashMap<String, DasAsset>,
    by_owner: HashMap<String, HashSet<String>>,
    by_authority: HashMap<String, HashSet<String>>,
    by_creator: HashMap<String, HashSet<String>>,
    by_group: HashMap<(String, String), HashSet<String>>,
}

impl MemoryCache {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl CacheStore for MemoryCache {
    async fn put_asset(&self, asset: DasAsset) -> CacheResult<()> {
        let mut g = self.inner.lock().await;
        let id = asset.id.clone();

        // Wipe prior index entries for this id so re-puts stay consistent.
        if let Some(prior) = g.by_id.get(&id).cloned() {
            g.by_owner
                .entry(prior.ownership.owner.clone())
                .or_default()
                .remove(&id);
            for auth in &prior.authorities {
                g.by_authority.entry(auth.address.clone()).or_default().remove(&id);
            }
            for creator in &prior.creators {
                g.by_creator.entry(creator.address.clone()).or_default().remove(&id);
            }
            for group in &prior.grouping {
                g.by_group
                    .entry((group.group_key.clone(), group.group_value.clone()))
                    .or_default()
                    .remove(&id);
            }
        }

        // Populate indexes for the new state. We skip owner indexing
        // when owner is empty (Token Metadata decoder leaves it blank
        // until owner resolution happens — 3c+).
        if !asset.ownership.owner.is_empty() {
            g.by_owner
                .entry(asset.ownership.owner.clone())
                .or_default()
                .insert(id.clone());
        }
        for auth in &asset.authorities {
            g.by_authority
                .entry(auth.address.clone())
                .or_default()
                .insert(id.clone());
        }
        for creator in &asset.creators {
            g.by_creator
                .entry(creator.address.clone())
                .or_default()
                .insert(id.clone());
        }
        for group in &asset.grouping {
            g.by_group
                .entry((group.group_key.clone(), group.group_value.clone()))
                .or_default()
                .insert(id.clone());
        }

        g.by_id.insert(id, asset);
        Ok(())
    }

    async fn get_asset(&self, id: &str) -> CacheResult<Option<DasAsset>> {
        let g = self.inner.lock().await;
        Ok(g.by_id.get(id).cloned())
    }

    async fn get_asset_batch(&self, ids: &[String]) -> CacheResult<Vec<Option<DasAsset>>> {
        let g = self.inner.lock().await;
        Ok(ids.iter().map(|id| g.by_id.get(id).cloned()).collect())
    }

    async fn get_assets_by_owner(&self, owner: &str) -> CacheResult<Vec<DasAsset>> {
        let g = self.inner.lock().await;
        Ok(resolve_ids(&g, g.by_owner.get(owner)))
    }

    async fn get_assets_by_authority(&self, authority: &str) -> CacheResult<Vec<DasAsset>> {
        let g = self.inner.lock().await;
        Ok(resolve_ids(&g, g.by_authority.get(authority)))
    }

    async fn get_assets_by_creator(
        &self,
        creator: &str,
        only_verified: bool,
    ) -> CacheResult<Vec<DasAsset>> {
        let g = self.inner.lock().await;
        let mut out = resolve_ids(&g, g.by_creator.get(creator));
        if only_verified {
            out.retain(|a| {
                a.creators
                    .iter()
                    .any(|c| c.address == creator && c.verified)
            });
        }
        Ok(out)
    }

    async fn get_assets_by_group(
        &self,
        group_key: &str,
        group_value: &str,
    ) -> CacheResult<Vec<DasAsset>> {
        let g = self.inner.lock().await;
        Ok(resolve_ids(
            &g,
            g.by_group
                .get(&(group_key.to_string(), group_value.to_string())),
        ))
    }

    async fn search_assets(&self, filter: &SearchFilter) -> CacheResult<Vec<DasAsset>> {
        let g = self.inner.lock().await;
        // Seed the candidate set with the smallest available index;
        // fall back to the full by_id table only when no indexed
        // field is provided.
        let mut candidates: Option<HashSet<String>> = None;

        if let Some(owner) = &filter.owner_address {
            narrow(&mut candidates, g.by_owner.get(owner));
        }
        if let Some(auth) = &filter.authority_address {
            narrow(&mut candidates, g.by_authority.get(auth));
        }
        if let Some(creator) = &filter.creator_address {
            narrow(&mut candidates, g.by_creator.get(creator));
        }
        if let Some((gk, gv)) = &filter.grouping {
            narrow(&mut candidates, g.by_group.get(&(gk.clone(), gv.clone())));
        }

        let ids: Vec<String> = match candidates {
            Some(s) => s.into_iter().collect(),
            None => g.by_id.keys().cloned().collect(),
        };

        let mut out: Vec<DasAsset> = ids
            .into_iter()
            .filter_map(|id| g.by_id.get(&id).cloned())
            .collect();

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
}

fn resolve_ids(g: &MemoryCacheInner, ids: Option<&HashSet<String>>) -> Vec<DasAsset> {
    ids.map(|s| {
        s.iter()
            .filter_map(|id| g.by_id.get(id).cloned())
            .collect()
    })
    .unwrap_or_default()
}

fn narrow(acc: &mut Option<HashSet<String>>, next: Option<&HashSet<String>>) {
    let next = next.cloned().unwrap_or_default();
    match acc {
        Some(existing) => existing.retain(|id| next.contains(id)),
        None => *acc = Some(next),
    }
}
