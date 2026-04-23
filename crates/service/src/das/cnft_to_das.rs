//! Pure mapper: `LeafRecord` (our stored cNFT shape) → `DasAsset` (the
//! external wire format Helius clients expect). No async, no I/O —
//! trivially testable and trivially Rust-portable.

use crate::cnft::LeafRecord;

use super::types::{
    DasAsset, DasAuthority, DasCompression, DasContent, DasCreator, DasFile, DasGrouping,
    DasLinks, DasMetadata, DasOwnership,
};

/// Convert a cNFT `LeafRecord` into the DAS `getAsset` response shape.
#[must_use]
pub fn leaf_record_to_das_asset(record: &LeafRecord) -> DasAsset {
    let m = &record.mint_metadata;

    let creators = m
        .creators
        .iter()
        .map(|c| DasCreator {
            address: bs58::encode(c.address).into_string(),
            share: c.share,
            verified: c.verified,
        })
        .collect();

    let grouping = m
        .collection
        .as_ref()
        .map(|(key, _)| {
            vec![DasGrouping {
                group_key: "collection".into(),
                group_value: bs58::encode(key).into_string(),
            }]
        })
        .unwrap_or_default();

    let owner = bs58::encode(record.owner).into_string();
    let delegate = bs58::encode(record.delegate).into_string();
    let delegated = owner != delegate;

    DasAsset {
        id: bs58::encode(record.asset_id).into_string(),
        interface: "V1_NFT".into(),
        content: DasContent {
            schema: "https://schema.metaplex.com/nft1.0.json".into(),
            json_uri: m.uri.clone(),
            metadata: DasMetadata {
                name: m.name.clone(),
                symbol: m.symbol.clone(),
                description: String::new(),
                ..Default::default()
            },
            links: DasLinks::default(),
            files: Vec::<DasFile>::new(),
            category: None,
        },
        authorities: Vec::<DasAuthority>::new(),
        creators,
        ownership: DasOwnership {
            delegated,
            ownership_model: "single".into(),
            owner,
            ..Default::default()
        },
        grouping,
        mutable: m.is_mutable,
        burnt: record.burned,
        compression: Some(DasCompression {
            eligible: true,
            compressed: true,
            data_hash: bs58::encode(record.data_hash).into_string(),
            creator_hash: bs58::encode(record.creator_hash).into_string(),
            asset_hash: bs58::encode(record.leaf_hash).into_string(),
            tree: bs58::encode(record.tree).into_string(),
            seq: 0,
            leaf_id: record.leaf_index,
        }),
        ..Default::default()
    }
}
