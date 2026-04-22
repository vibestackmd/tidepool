//! Token Metadata decoder (legacy NFTs, including Token-2022 mints
//! whose metadata lives at the same Metaplex Metadata PDA derivation).
//!
//! Step 3b1 scope: decode a Metadata account → DAS shape. The
//! `ownership.owner` field is left empty here — real Helius resolves
//! the owner by scanning token accounts for the mint's holder, which
//! requires a `getProgramAccounts` memcmp against the upstream. That
//! resolution lives in the DAS handler layer (`fetch_and_cache_asset`
//! — step 3c), not in the decoder, so the decoder stays sync + pure.
//!
//! Edition-PDA side-effect indexing (tracking print editions for
//! `getNftEditions`) similarly lives at the handler boundary.

use mpl_token_metadata::accounts::Metadata;
use mpl_token_metadata::types::Key;

use super::decoder::{AccountDecoder, DecoderError};
use super::types::{
    DasAsset, DasAuthority, DasContent, DasCreator, DasFile, DasGrouping, DasLinks, DasMetadata,
    DasOwnership,
};

pub struct TokenMetadataDecoder;

impl TokenMetadataDecoder {
    pub const PROGRAM_ID: &'static str = "metaqbxxUerdq28cj1RbAWkYQm3ybzjb6a8bt518x1s";
    pub const NAME: &'static str = "V1_NFT";
}

impl AccountDecoder for TokenMetadataDecoder {
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
        // First byte is the Key discriminator. Only MetadataV1 lives
        // in the main DAS pipeline; EditionV1 / MasterEditionV1/V2 are
        // separate flows served by getNftEditions (step 3c).
        if data[0] != Key::MetadataV1 as u8 {
            return Ok(None);
        }

        let metadata = Metadata::from_bytes(data).map_err(|e| DecoderError::DecodeFailed {
            decoder: Self::NAME,
            context: format!("pubkey {pubkey}"),
            source: e,
        })?;

        Ok(Some(to_das_asset(pubkey, &metadata)))
    }
}

fn to_das_asset(pubkey: &str, m: &Metadata) -> DasAsset {
    let creators = m
        .creators
        .as_ref()
        .map(|list| {
            list.iter()
                .map(|c| DasCreator {
                    address: c.address.to_string(),
                    share: c.share,
                    verified: c.verified,
                })
                .collect()
        })
        .unwrap_or_default();

    let grouping = m
        .collection
        .as_ref()
        .filter(|c| c.verified)
        .map(|c| {
            vec![DasGrouping {
                group_key: "collection".into(),
                group_value: c.key.to_string(),
            }]
        })
        .unwrap_or_default();

    DasAsset {
        id: pubkey.to_string(),
        interface: "V1_NFT".into(),
        content: DasContent {
            schema: "https://schema.metaplex.com/nft1.0.json".into(),
            json_uri: m.uri.trim_end_matches('\0').to_string(),
            metadata: DasMetadata {
                name: m.name.trim_end_matches('\0').to_string(),
                symbol: m.symbol.trim_end_matches('\0').to_string(),
                description: String::new(),
            },
            links: DasLinks {
                image: None,
                animation_url: None,
            },
            files: Vec::<DasFile>::new(),
        },
        authorities: vec![DasAuthority {
            address: m.update_authority.to_string(),
            scopes: vec!["full".into()],
        }],
        creators,
        ownership: DasOwnership {
            frozen: false,
            delegated: false,
            ownership_model: "single".into(),
            // Left empty — resolved by the handler layer via
            // getProgramAccounts memcmp against the mint's token
            // program. We know the mint here (m.mint), we don't know
            // which account holds the token.
            owner: String::new(),
        },
        grouping,
        mutable: m.is_mutable,
        burnt: false,
        compression: None,
    }
}
