//! Token Metadata decoder (legacy NFTs, including Token-2022 mints
//! whose metadata lives at the same Metaplex Metadata PDA derivation).
//!
//! Step 3b1 scope: decode a Metadata account → DAS shape. The
//! `ownership.owner` field is left empty here — the Metadata account
//! itself doesn't carry the holding wallet, only the mint. Owner
//! resolution (getTokenLargestAccounts on the mint → read the token
//! account's owner slot) lives in `fetch_and_cache_asset` so the
//! decoder stays sync + pure.
//!
//! Edition-PDA side-effect indexing (tracking print editions for
//! `getNftEditions`) similarly lives at the handler boundary.

use mpl_token_metadata::accounts::Metadata;
use mpl_token_metadata::types::{Key, TokenStandard};

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

/// Map the Metaplex `TokenStandard` enum to the `interface` string
/// Helius's DAS API publishes. The distinction matters: wallets
/// render pNFTs differently from plain NFTs (royalty enforcement,
/// burn-on-transfer constraints, etc.). Returning `V1_NFT`
/// universally — our previous behavior — mis-classified every
/// programmable NFT on mainnet.
///
/// Mapping matches real Helius output observed via the contract-test
/// fixtures (see `contracts/fixtures/getAsset/`). A `None` standard
/// (old Metadata accounts from pre-`TokenStandard` days) defaults to
/// `V1_NFT`, which is what Helius also does.
fn interface_for_standard(std: Option<&TokenStandard>) -> &'static str {
    match std {
        Some(
            TokenStandard::ProgrammableNonFungible
            | TokenStandard::ProgrammableNonFungibleEdition,
        ) => "ProgrammableNFT",
        Some(TokenStandard::Fungible) => "FungibleToken",
        Some(TokenStandard::FungibleAsset) => "FungibleAsset",
        Some(TokenStandard::NonFungible | TokenStandard::NonFungibleEdition) | None => "V1_NFT",
    }
}

/// Canonical `token_standard` string Helius populates under
/// `content.metadata.token_standard`. Distinct from the top-level
/// `interface` — Helius carries both.
fn token_standard_name(std: Option<&TokenStandard>) -> Option<String> {
    let name = match std? {
        TokenStandard::NonFungible => "NonFungible",
        TokenStandard::FungibleAsset => "FungibleAsset",
        TokenStandard::Fungible => "Fungible",
        TokenStandard::NonFungibleEdition => "NonFungibleEdition",
        TokenStandard::ProgrammableNonFungible => "ProgrammableNonFungible",
        TokenStandard::ProgrammableNonFungibleEdition => "ProgrammableNonFungibleEdition",
    };
    Some(name.into())
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
        interface: interface_for_standard(m.token_standard.as_ref()).into(),
        content: DasContent {
            schema: "https://schema.metaplex.com/nft1.0.json".into(),
            json_uri: m.uri.trim_end_matches('\0').to_string(),
            metadata: DasMetadata {
                name: m.name.trim_end_matches('\0').to_string(),
                symbol: m.symbol.trim_end_matches('\0').to_string(),
                description: String::new(),
                token_standard: token_standard_name(m.token_standard.as_ref()),
                ..Default::default()
            },
            links: DasLinks::default(),
            files: Vec::<DasFile>::new(),
            category: None,
        },
        authorities: vec![DasAuthority {
            address: m.update_authority.to_string(),
            scopes: vec!["full".into()],
        }],
        creators,
        ownership: DasOwnership {
            ownership_model: "single".into(),
            // Left empty — resolved by the handler layer via
            // getProgramAccounts memcmp against the mint's token
            // program. We know the mint here (m.mint), we don't know
            // which account holds the token.
            owner: String::new(),
            ..Default::default()
        },
        grouping,
        mutable: m.is_mutable,
        burnt: false,
        compression: None,
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interface_maps_programmable_nfts() {
        assert_eq!(
            interface_for_standard(Some(&TokenStandard::ProgrammableNonFungible)),
            "ProgrammableNFT"
        );
        assert_eq!(
            interface_for_standard(Some(&TokenStandard::ProgrammableNonFungibleEdition)),
            "ProgrammableNFT"
        );
    }

    #[test]
    fn interface_maps_plain_nfts_to_v1_nft() {
        assert_eq!(
            interface_for_standard(Some(&TokenStandard::NonFungible)),
            "V1_NFT"
        );
        assert_eq!(
            interface_for_standard(Some(&TokenStandard::NonFungibleEdition)),
            "V1_NFT"
        );
    }

    #[test]
    fn interface_maps_fungibles() {
        assert_eq!(
            interface_for_standard(Some(&TokenStandard::Fungible)),
            "FungibleToken"
        );
        assert_eq!(
            interface_for_standard(Some(&TokenStandard::FungibleAsset)),
            "FungibleAsset"
        );
    }

    #[test]
    fn interface_missing_standard_defaults_to_v1_nft() {
        // Legacy Metadata accounts from pre-`TokenStandard` days
        // leave this field None. Helius reports them as V1_NFT.
        assert_eq!(interface_for_standard(None), "V1_NFT");
    }

    #[test]
    fn token_standard_name_round_trips_all_variants() {
        let cases = [
            (TokenStandard::NonFungible, "NonFungible"),
            (TokenStandard::FungibleAsset, "FungibleAsset"),
            (TokenStandard::Fungible, "Fungible"),
            (TokenStandard::NonFungibleEdition, "NonFungibleEdition"),
            (
                TokenStandard::ProgrammableNonFungible,
                "ProgrammableNonFungible",
            ),
            (
                TokenStandard::ProgrammableNonFungibleEdition,
                "ProgrammableNonFungibleEdition",
            ),
        ];
        for (std, expected) in cases {
            assert_eq!(
                token_standard_name(Some(&std)).as_deref(),
                Some(expected)
            );
        }
        assert!(token_standard_name(None).is_none());
    }
}
