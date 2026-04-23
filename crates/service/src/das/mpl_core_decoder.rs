//! MplCore decoder. Uses `mpl_core::IndexableAsset::fetch` which
//! handles the full account layout + plugin registry walk — we don't
//! hand-roll Borsh decoding or plugin traversal, we just shape the
//! result into a DAS asset.
//!
//! Handles `Key::AssetV1` (individual asset) and `Key::CollectionV1`.
//! Hashed assets (`HashedAssetV1`) and groups are recognized but
//! currently mapped to minimal responses — they're rarely directly
//! queried in real flows.

use mpl_core::types::{Key, Plugin, PluginType, UpdateAuthority};
use mpl_core::IndexableAsset;
use solana_program::pubkey::Pubkey;

use super::decoder::{AccountDecoder, DecoderError};
use super::types::{
    DasAsset, DasAuthority, DasContent, DasCreator, DasFile, DasGrouping, DasLinks, DasMetadata,
    DasOwnership,
};

pub struct MplCoreDecoder;

impl MplCoreDecoder {
    pub const PROGRAM_ID: &'static str = "CoREENxT6tW1HoK8ypY1SxRMZTcVPm7R94rH4PZNhX7d";
    pub const NAME: &'static str = "MplCoreAsset";
}

impl AccountDecoder for MplCoreDecoder {
    fn program_id(&self) -> &str {
        Self::PROGRAM_ID
    }

    fn name(&self) -> &str {
        Self::NAME
    }

    fn decode(&self, pubkey: &str, data: &[u8]) -> Result<Option<DasAsset>, DecoderError> {
        if data.is_empty() {
            return Ok(None);
        }
        // First byte is the Key discriminator.
        let key = match data[0] {
            x if x == Key::AssetV1 as u8 => Key::AssetV1,
            x if x == Key::CollectionV1 as u8 => Key::CollectionV1,
            // Other variants exist (HashedAssetV1, GroupV1, plugin
            // account) but we don't serve them through getAsset today.
            _ => return Ok(None),
        };

        let indexable =
            IndexableAsset::fetch(key, data).map_err(|e| DecoderError::DecodeFailed {
                decoder: Self::NAME,
                context: format!("pubkey {pubkey}"),
                source: e,
            })?;

        Ok(Some(to_das_asset(pubkey, &indexable, key)))
    }
}

fn to_das_asset(pubkey: &str, ix: &IndexableAsset, key: Key) -> DasAsset {
    let owner = ix.owner.as_ref().map(Pubkey::to_string).unwrap_or_default();

    // Creators: Royalties plugin carries canonical royalty splits;
    // VerifiedCreators plugin tracks which creator addresses have
    // verified their involvement. Merge so each creator entry has
    // address + share + verified.
    let creators = merge_creators(ix);

    // Grouping: MplCore assets reference their collection via
    // `update_authority = Collection(pubkey)`. Collections don't have
    // grouping themselves (they're the group).
    let grouping = match (&ix.update_authority, key) {
        (UpdateAuthority::Collection(pk), Key::AssetV1) => vec![DasGrouping {
            group_key: "collection".into(),
            group_value: pk.to_string(),
        }],
        _ => vec![],
    };

    // Authorities: for assets, the effective update authority. We
    // report it under a single entry with the standard `full` scope.
    let authorities = update_authority_to_entry(&ix.update_authority);

    let (frozen, delegated_by_plugin) = freeze_and_delegate_flags(ix);

    DasAsset {
        id: pubkey.to_string(),
        interface: interface_for_key(key),
        content: DasContent {
            schema: "https://schema.metaplex.com/nft1.0.json".into(),
            json_uri: ix.uri.clone(),
            metadata: DasMetadata {
                name: ix.name.clone(),
                ..Default::default()
            },
            links: DasLinks::default(),
            files: Vec::<DasFile>::new(),
            category: None,
        },
        authorities,
        creators,
        ownership: DasOwnership {
            frozen,
            delegated: delegated_by_plugin,
            ownership_model: "single".into(),
            owner,
            ..Default::default()
        },
        grouping,
        mutable: true, // MplCore assets are mutable unless ImmutableMetadata plugin is present.
        burnt: false,
        compression: None,
        ..Default::default()
    }
}

fn interface_for_key(key: Key) -> String {
    match key {
        Key::CollectionV1 => "MplCoreCollection".into(),
        // AssetV1 and anything unrecognized fall through to the
        // default single-asset interface tag.
        _ => "MplCoreAsset".into(),
    }
}

fn update_authority_to_entry(ua: &UpdateAuthority) -> Vec<DasAuthority> {
    match ua {
        UpdateAuthority::Address(pk) | UpdateAuthority::Collection(pk) => vec![DasAuthority {
            address: pk.to_string(),
            scopes: vec!["full".into()],
        }],
        UpdateAuthority::None => vec![],
    }
}

fn merge_creators(ix: &IndexableAsset) -> Vec<DasCreator> {
    use std::collections::BTreeMap;

    // Start with royalty creators (canonical shares).
    let mut by_address: BTreeMap<String, DasCreator> = BTreeMap::new();
    if let Some(plugin) = ix.plugins.get(&PluginType::Royalties) {
        if let Plugin::Royalties(r) = &plugin.data {
            for c in &r.creators {
                by_address.insert(
                    c.address.to_string(),
                    DasCreator {
                        address: c.address.to_string(),
                        share: c.percentage,
                        verified: false,
                    },
                );
            }
        }
    }
    // Overlay verified flags from VerifiedCreators plugin.
    if let Some(plugin) = ix.plugins.get(&PluginType::VerifiedCreators) {
        if let Plugin::VerifiedCreators(v) = &plugin.data {
            for sig in &v.signatures {
                let addr = sig.address.to_string();
                if let Some(c) = by_address.get_mut(&addr) {
                    c.verified = sig.verified;
                } else if sig.verified {
                    // Verified creator with no royalty entry — still
                    // report, share=0.
                    by_address.insert(
                        addr.clone(),
                        DasCreator {
                            address: addr,
                            share: 0,
                            verified: true,
                        },
                    );
                }
            }
        }
    }

    by_address.into_values().collect()
}

fn freeze_and_delegate_flags(ix: &IndexableAsset) -> (bool, bool) {
    let mut frozen = false;
    let mut delegated = false;

    if let Some(p) = ix.plugins.get(&PluginType::FreezeDelegate) {
        if let Plugin::FreezeDelegate(fd) = &p.data {
            if fd.frozen {
                frozen = true;
            }
        }
        delegated = true;
    }
    if let Some(p) = ix.plugins.get(&PluginType::PermanentFreezeDelegate) {
        if let Plugin::PermanentFreezeDelegate(fd) = &p.data {
            if fd.frozen {
                frozen = true;
            }
        }
    }
    if ix.plugins.contains_key(&PluginType::TransferDelegate)
        || ix
            .plugins
            .contains_key(&PluginType::PermanentTransferDelegate)
    {
        delegated = true;
    }

    (frozen, delegated)
}
