//! Opportunistic enrichment of `EnhancedTransaction` fields from the
//! DAS cache. Keeps the parse step pure: fan out upstream → parse →
//! enrich. Looking up per-mint metadata in the cache is cheap (single
//! cache read per distinct mint) and never does network; anything not
//! already cached stays unenriched.
//!
//! Today the only enrichment is `tokenTransfers[].tokenStandard`,
//! which clients rely on to distinguish NFT mints, pNFTs, and
//! fungible transfers from the raw token-balance diff we produce.

use std::collections::HashMap;

use crate::cache::CacheStore;

use super::types::EnhancedTransaction;

/// Fill in `tokenTransfers[].tokenStandard` for every transaction
/// whose mint is already in the DAS cache. Mints that aren't cached
/// are left with `token_standard = None`; the round-trip contract
/// test is tolerant of that via `skip_serializing_if`.
pub async fn enrich_token_standards<C: CacheStore + ?Sized>(
    cache: &C,
    txs: &mut [EnhancedTransaction],
) {
    // Collect the distinct mint set across all txs so we do one cache
    // hit per mint, not one per (tx, transfer) pair.
    let mut distinct_mints: Vec<String> = Vec::new();
    for tx in txs.iter() {
        for t in &tx.token_transfers {
            if !distinct_mints.contains(&t.mint) {
                distinct_mints.push(t.mint.clone());
            }
        }
    }
    if distinct_mints.is_empty() {
        return;
    }

    // Resolve each mint to an optional tokenStandard string. Batch
    // where the cache supports it; fall back to per-id lookups.
    let batch = cache.get_asset_batch(&distinct_mints).await.unwrap_or_else(|_| {
        distinct_mints.iter().map(|_| None).collect()
    });
    let mut lookup: HashMap<String, String> = HashMap::new();
    for (mint, entry) in distinct_mints.iter().zip(batch.into_iter()) {
        if let Some(asset) = entry {
            if let Some(ts) = asset.content.metadata.token_standard.clone() {
                lookup.insert(mint.clone(), ts);
            }
        }
    }
    if lookup.is_empty() {
        return;
    }

    for tx in txs.iter_mut() {
        for t in &mut tx.token_transfers {
            if t.token_standard.is_none() {
                if let Some(ts) = lookup.get(&t.mint) {
                    t.token_standard = Some(ts.clone());
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache::MemoryCache;
    use crate::das::types::{DasAsset, DasContent, DasMetadata};
    use crate::enhanced::types::{EnhancedEvents, EnhancedTokenTransfer, EnhancedTransaction};

    fn tx_with_transfer(mint: &str) -> EnhancedTransaction {
        EnhancedTransaction {
            signature: "S".into(),
            slot: 1,
            timestamp: None,
            tx_type: "TRANSFER".into(),
            source: "SYSTEM_PROGRAM".into(),
            fee: 0,
            fee_payer: "F".into(),
            description: String::new(),
            native_transfers: vec![],
            token_transfers: vec![EnhancedTokenTransfer {
                from_user_account: None,
                to_user_account: None,
                from_token_account: None,
                to_token_account: None,
                mint: mint.into(),
                token_amount: 1,
                token_standard: None,
            }],
            instructions: vec![],
            account_data: vec![],
            events: EnhancedEvents::default(),
            lighthouse_data: None,
            transaction_error: None,
        }
    }

    fn asset_with_token_standard(id: &str, ts: &str) -> DasAsset {
        DasAsset {
            id: id.into(),
            interface: "FungibleToken".into(),
            content: DasContent {
                metadata: DasMetadata {
                    token_standard: Some(ts.into()),
                    ..Default::default()
                },
                ..Default::default()
            },
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn enriches_from_cache() {
        let cache = MemoryCache::new();
        cache
            .put_asset(asset_with_token_standard("MINT_A", "Fungible"))
            .await
            .unwrap();
        let mut txs = vec![tx_with_transfer("MINT_A")];
        enrich_token_standards(&cache, &mut txs).await;
        assert_eq!(
            txs[0].token_transfers[0].token_standard.as_deref(),
            Some("Fungible")
        );
    }

    #[tokio::test]
    async fn uncached_mints_stay_none() {
        let cache = MemoryCache::new();
        let mut txs = vec![tx_with_transfer("UNKNOWN_MINT")];
        enrich_token_standards(&cache, &mut txs).await;
        assert!(txs[0].token_transfers[0].token_standard.is_none());
    }

    #[tokio::test]
    async fn does_not_overwrite_pre_populated_standard() {
        let cache = MemoryCache::new();
        cache
            .put_asset(asset_with_token_standard("MINT_A", "Fungible"))
            .await
            .unwrap();
        let mut tx = tx_with_transfer("MINT_A");
        tx.token_transfers[0].token_standard = Some("ProgrammableNonFungible".into());
        let mut txs = vec![tx];
        enrich_token_standards(&cache, &mut txs).await;
        // Pre-populated value wins — the enricher is opportunistic,
        // not authoritative.
        assert_eq!(
            txs[0].token_transfers[0].token_standard.as_deref(),
            Some("ProgrammableNonFungible")
        );
    }
}
