//! Bubblegum instruction parser. Pure function: given raw ix data +
//! the ix's account list + optionally a LeafSchemaEvent pulled from
//! the tx's inner ixs, emit a `CnftEvent` (or surface a structured
//! error).
//!
//! Dispatch is a `match` on the 8-byte discriminator prefix; each arm
//! Borsh-decodes the matching `<Ix>InstructionArgs` from
//! `mpl-bubblegum` and pulls the tree / owner / delegate pubkeys out
//! of `accounts[]` by fixed positions. The position tables are
//! verified by integration tests that round-trip encoded args through
//! the parser.
//!
//! Noop-required ixs (verify*, updateMetadata) return
//! `ParseError::Unsupported` when no LeafSchemaEvent accompanies
//! them — we can't reconstruct their new hashes from ix args alone,
//! so the indexer skips cleanly rather than writing wrong state.

use borsh::BorshDeserialize;
// Args types for the six ixs whose body we Borsh-decode. verify* /
// unverify* / setAndVerifyCollection don't need body decoding in our
// flow — all the new state comes from the paired LeafSchemaEvent, and
// positional accounts give us the creator / collection pubkey.
use mpl_bubblegum::instructions::{
    BurnInstructionArgs, CreateTreeConfigInstructionArgs, DelegateInstructionArgs,
    MintToCollectionV1InstructionArgs, MintV1InstructionArgs, TransferInstructionArgs,
    UpdateMetadataInstructionArgs,
};
use mpl_bubblegum::types::{MetadataArgs, UpdateArgs};
use thiserror::Error;
use tidepool_rpc_core::Creator;

use super::leaf_event::LeafSchemaEventDecoded;
use super::types::{CnftEvent, MintMetadata, NoopOverride};

pub const BUBBLEGUM_PROGRAM_ID: &str = "BGUMAp9Gq7iTEuizy4pqaxsTyUCBK68MDfK752saRPUY";

// ─── anchor-derived instruction discriminators ─────────────────────
// These are sha256("global:<ix_name>")[..8] from the on-chain Bubblegum
// program. We hardcode them rather than serializing an
// `<Ix>InstructionData::new()` at runtime because they never change —
// anchor's namespacing is deterministic. A drift test lives at
// tests/discriminators.rs to catch mpl-bubblegum ever rebasing them.

pub const CREATE_TREE_CONFIG_DISC: [u8; 8] = [165, 83, 136, 142, 89, 202, 47, 220];
pub const MINT_V1_DISC: [u8; 8] = [145, 98, 192, 118, 184, 147, 118, 104];
pub const MINT_TO_COLLECTION_V1_DISC: [u8; 8] = [153, 18, 178, 47, 197, 158, 86, 15];
pub const TRANSFER_DISC: [u8; 8] = [163, 52, 200, 231, 140, 3, 69, 186];
pub const BURN_DISC: [u8; 8] = [116, 110, 29, 56, 107, 219, 42, 93];
pub const DELEGATE_DISC: [u8; 8] = [90, 147, 75, 178, 85, 88, 4, 137];
pub const VERIFY_CREATOR_DISC: [u8; 8] = [52, 17, 96, 132, 71, 4, 85, 194];
pub const UNVERIFY_CREATOR_DISC: [u8; 8] = [107, 178, 57, 39, 105, 115, 112, 152];
pub const VERIFY_COLLECTION_DISC: [u8; 8] = [56, 113, 101, 253, 79, 55, 122, 169];
pub const UNVERIFY_COLLECTION_DISC: [u8; 8] = [250, 251, 42, 106, 41, 137, 186, 168];
pub const SET_AND_VERIFY_COLLECTION_DISC: [u8; 8] = [235, 242, 121, 216, 158, 234, 180, 234];
pub const UPDATE_METADATA_DISC: [u8; 8] = [170, 182, 43, 239, 97, 78, 225, 186];

// ─── account position tables ──────────────────────────────────────
// Positions are from the Anchor-generated builder in mpl-bubblegum 3:
// each ix declares an `accounts: [...]` array in a known order. We
// only name the positions we actually read; the rest are unnamed but
// counted by the min-account-count checks.

mod pos {
    // create_tree_config: [treeConfig, merkleTree, payer, treeCreator,
    //                      logWrapper, compressionProgram, systemProgram]
    pub mod create_tree {
        pub const MERKLE_TREE: usize = 1;
        pub const MIN: usize = 7;
    }
    // mint_v1: [treeConfig, leafOwner, leafDelegate, merkleTree, payer,
    //           treeDelegate, logWrapper, compressionProgram, systemProgram]
    pub mod mint_v1 {
        pub const LEAF_OWNER: usize = 1;
        pub const LEAF_DELEGATE: usize = 2;
        pub const MERKLE_TREE: usize = 3;
        pub const MIN: usize = 9;
    }
    // mint_to_collection_v1: adds collectionAuthority /
    //   collectionAuthorityRecordPda / collectionMint / collectionMetadata
    //   / editionAccount / bubblegumSigner / tokenMetadataProgram in
    //   positions 6..=14. collectionMint lands at index 8.
    pub mod mint_to_collection {
        pub const LEAF_OWNER: usize = 1;
        pub const LEAF_DELEGATE: usize = 2;
        pub const MERKLE_TREE: usize = 3;
        pub const COLLECTION_MINT: usize = 8;
        pub const MIN: usize = 16;
    }
    // transfer: [treeConfig, leafOwner, leafDelegate, newLeafOwner,
    //            merkleTree, logWrapper, compressionProgram, systemProgram]
    pub mod transfer {
        pub const NEW_LEAF_OWNER: usize = 3;
        pub const MERKLE_TREE: usize = 4;
        pub const MIN: usize = 8;
    }
    // burn: [treeConfig, leafOwner, leafDelegate, merkleTree,
    //        logWrapper, compressionProgram, systemProgram]
    pub mod burn {
        pub const MERKLE_TREE: usize = 3;
        pub const MIN: usize = 7;
    }
    // delegate: [treeConfig, leafOwner, previousLeafDelegate,
    //            newLeafDelegate, merkleTree, logWrapper,
    //            compressionProgram, systemProgram]
    pub mod delegate {
        pub const NEW_LEAF_DELEGATE: usize = 3;
        pub const MERKLE_TREE: usize = 4;
        pub const MIN: usize = 8;
    }
    // verify_creator / unverify_creator: [treeConfig, leafOwner,
    //   leafDelegate, merkleTree, payer, creator, logWrapper,
    //   compressionProgram, systemProgram]
    pub mod verify_creator {
        pub const MERKLE_TREE: usize = 3;
        pub const CREATOR: usize = 5;
        pub const MIN: usize = 9;
    }
    // verify_collection / unverify_collection / set_and_verify_collection:
    //   same layout as mint_to_collection_v1 for the accounts we read.
    pub mod verify_collection {
        pub const MERKLE_TREE: usize = 3;
        pub const COLLECTION_MINT: usize = 8;
        pub const MIN: usize = 16;
    }
    // update_metadata: [treeConfig, authority, collectionMint,
    //   collectionMetadata, collectionAuthorityRecordPda, leafOwner,
    //   leafDelegate, payer, merkleTree, logWrapper,
    //   compressionProgram, tokenMetadataProgram, systemProgram]
    pub mod update_metadata {
        pub const MERKLE_TREE: usize = 8;
        pub const MIN: usize = 13;
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ParseError {
    #[error("ix data truncated: expected at least {expected}, got {actual}")]
    TruncatedData { expected: usize, actual: usize },
    #[error("unknown discriminator: {discriminator:?}")]
    UnknownDiscriminator { discriminator: [u8; 8] },
    #[error("ix needs at least {expected} accounts, got {actual}")]
    InsufficientAccounts { expected: usize, actual: usize },
    #[error("ix args failed to decode: {0}")]
    DecoderError(String),
    #[error("unsupported: {0}")]
    Unsupported(String),
}

/// Dispatch entry point. `accounts` is the account list in IDL order
/// (same order the builder used). `noop_event` is the
/// LeafSchemaEvent from a paired noop CPI inside the same tx, if any.
/// Returns `Ok(Some(event))` for a tracked state transition,
/// `Ok(None)` for a Bubblegum ix we don't track (Redeem, Compress, V2
/// family, …), `Err` for malformed or unsupported cases.
pub fn parse_bubblegum_instruction(
    data: &[u8],
    accounts: &[[u8; 32]],
    noop_event: Option<&LeafSchemaEventDecoded>,
) -> Result<Option<CnftEvent>, ParseError> {
    if data.len() < 8 {
        return Err(ParseError::TruncatedData {
            expected: 8,
            actual: data.len(),
        });
    }
    let (disc, body) = data.split_at(8);
    let disc: [u8; 8] = disc.try_into().expect("split_at(8) yields 8 bytes");

    match disc {
        CREATE_TREE_CONFIG_DISC => parse_create_tree(body, accounts).map(Some),
        MINT_V1_DISC => parse_mint_v1(body, accounts, noop_event).map(Some),
        MINT_TO_COLLECTION_V1_DISC => {
            parse_mint_to_collection_v1(body, accounts, noop_event).map(Some)
        }
        TRANSFER_DISC => parse_transfer(body, accounts, noop_event).map(Some),
        BURN_DISC => parse_burn(body, accounts, noop_event).map(Some),
        DELEGATE_DISC => parse_delegate(body, accounts, noop_event).map(Some),
        VERIFY_CREATOR_DISC => parse_verify_creator(accounts, noop_event).map(Some),
        UNVERIFY_CREATOR_DISC => parse_unverify_creator(accounts, noop_event).map(Some),
        VERIFY_COLLECTION_DISC => parse_verify_collection(accounts, noop_event).map(Some),
        UNVERIFY_COLLECTION_DISC => parse_unverify_collection(accounts, noop_event).map(Some),
        SET_AND_VERIFY_COLLECTION_DISC => {
            parse_set_and_verify_collection(accounts, noop_event).map(Some)
        }
        UPDATE_METADATA_DISC => parse_update_metadata(body, accounts, noop_event).map(Some),
        _ => Err(ParseError::UnknownDiscriminator { discriminator: disc }),
    }
}

// ─── per-ix parsers ─────────────────────────────────────────────────

fn parse_create_tree(body: &[u8], accounts: &[[u8; 32]]) -> Result<CnftEvent, ParseError> {
    require_accounts(accounts.len(), pos::create_tree::MIN)?;
    let args = CreateTreeConfigInstructionArgs::try_from_slice(body)
        .map_err(|e| ParseError::DecoderError(e.to_string()))?;
    Ok(CnftEvent::CreateTree {
        tree: accounts[pos::create_tree::MERKLE_TREE],
        depth: u8::try_from(args.max_depth).unwrap_or(u8::MAX),
        max_buffer_size: args.max_buffer_size,
    })
}

fn parse_mint_v1(
    body: &[u8],
    accounts: &[[u8; 32]],
    noop_event: Option<&LeafSchemaEventDecoded>,
) -> Result<CnftEvent, ParseError> {
    require_accounts(accounts.len(), pos::mint_v1::MIN)?;
    let args = MintV1InstructionArgs::try_from_slice(body)
        .map_err(|e| ParseError::DecoderError(e.to_string()))?;
    let metadata = to_mint_metadata(&args.metadata, body);
    Ok(CnftEvent::Mint {
        tree: accounts[pos::mint_v1::MERKLE_TREE],
        owner: accounts[pos::mint_v1::LEAF_OWNER],
        delegate: accounts[pos::mint_v1::LEAF_DELEGATE],
        metadata,
        verify_collection: None,
        noop: noop_to_override(noop_event),
    })
}

fn parse_mint_to_collection_v1(
    body: &[u8],
    accounts: &[[u8; 32]],
    noop_event: Option<&LeafSchemaEventDecoded>,
) -> Result<CnftEvent, ParseError> {
    require_accounts(accounts.len(), pos::mint_to_collection::MIN)?;
    let args = MintToCollectionV1InstructionArgs::try_from_slice(body)
        .map_err(|e| ParseError::DecoderError(e.to_string()))?;

    // mint_to_collection_v1 verifies the collection as part of the ix —
    // the stored metadata reflects collection.verified = true regardless
    // of what the raw args carried. We synthesize that state here so
    // the apply step doesn't need to know the rule.
    let collection_mint = accounts[pos::mint_to_collection::COLLECTION_MINT];
    let mut metadata = to_mint_metadata(&args.metadata, body);
    metadata.collection = Some((collection_mint, true));

    Ok(CnftEvent::Mint {
        tree: accounts[pos::mint_to_collection::MERKLE_TREE],
        owner: accounts[pos::mint_to_collection::LEAF_OWNER],
        delegate: accounts[pos::mint_to_collection::LEAF_DELEGATE],
        metadata,
        verify_collection: Some(collection_mint),
        noop: noop_to_override(noop_event),
    })
}

fn parse_transfer(
    body: &[u8],
    accounts: &[[u8; 32]],
    noop_event: Option<&LeafSchemaEventDecoded>,
) -> Result<CnftEvent, ParseError> {
    require_accounts(accounts.len(), pos::transfer::MIN)?;
    let args = TransferInstructionArgs::try_from_slice(body)
        .map_err(|e| ParseError::DecoderError(e.to_string()))?;
    let new_owner = accounts[pos::transfer::NEW_LEAF_OWNER];
    Ok(CnftEvent::Transfer {
        tree: accounts[pos::transfer::MERKLE_TREE],
        leaf_index: u64::from(args.index),
        nonce: args.nonce,
        new_owner,
        // Bubblegum resets delegate to new owner on transfer.
        new_delegate: new_owner,
        data_hash: args.data_hash,
        creator_hash: args.creator_hash,
        noop: noop_to_override(noop_event),
    })
}

fn parse_burn(
    body: &[u8],
    accounts: &[[u8; 32]],
    noop_event: Option<&LeafSchemaEventDecoded>,
) -> Result<CnftEvent, ParseError> {
    require_accounts(accounts.len(), pos::burn::MIN)?;
    let args = BurnInstructionArgs::try_from_slice(body)
        .map_err(|e| ParseError::DecoderError(e.to_string()))?;
    Ok(CnftEvent::Burn {
        tree: accounts[pos::burn::MERKLE_TREE],
        leaf_index: u64::from(args.index),
        nonce: args.nonce,
        noop: noop_to_override(noop_event),
    })
}

fn parse_delegate(
    body: &[u8],
    accounts: &[[u8; 32]],
    noop_event: Option<&LeafSchemaEventDecoded>,
) -> Result<CnftEvent, ParseError> {
    require_accounts(accounts.len(), pos::delegate::MIN)?;
    let args = DelegateInstructionArgs::try_from_slice(body)
        .map_err(|e| ParseError::DecoderError(e.to_string()))?;
    Ok(CnftEvent::Delegate {
        tree: accounts[pos::delegate::MERKLE_TREE],
        leaf_index: u64::from(args.index),
        nonce: args.nonce,
        new_delegate: accounts[pos::delegate::NEW_LEAF_DELEGATE],
        data_hash: args.data_hash,
        creator_hash: args.creator_hash,
        noop: noop_to_override(noop_event),
    })
}

fn parse_verify_creator(
    accounts: &[[u8; 32]],
    noop_event: Option<&LeafSchemaEventDecoded>,
) -> Result<CnftEvent, ParseError> {
    require_accounts(accounts.len(), pos::verify_creator::MIN)?;
    // Args are present but we don't need them — authoritative state
    // comes from the noop event.
    let noop = require_noop(noop_event, "verifyCreator")?;
    Ok(CnftEvent::VerifyCreator {
        tree: accounts[pos::verify_creator::MERKLE_TREE],
        creator: accounts[pos::verify_creator::CREATOR],
        noop,
    })
}

fn parse_unverify_creator(
    accounts: &[[u8; 32]],
    noop_event: Option<&LeafSchemaEventDecoded>,
) -> Result<CnftEvent, ParseError> {
    require_accounts(accounts.len(), pos::verify_creator::MIN)?;
    let noop = require_noop(noop_event, "unverifyCreator")?;
    Ok(CnftEvent::UnverifyCreator {
        tree: accounts[pos::verify_creator::MERKLE_TREE],
        creator: accounts[pos::verify_creator::CREATOR],
        noop,
    })
}

fn parse_verify_collection(
    accounts: &[[u8; 32]],
    noop_event: Option<&LeafSchemaEventDecoded>,
) -> Result<CnftEvent, ParseError> {
    require_accounts(accounts.len(), pos::verify_collection::MIN)?;
    let noop = require_noop(noop_event, "verifyCollection")?;
    Ok(CnftEvent::VerifyCollection {
        tree: accounts[pos::verify_collection::MERKLE_TREE],
        collection: accounts[pos::verify_collection::COLLECTION_MINT],
        noop,
    })
}

fn parse_unverify_collection(
    accounts: &[[u8; 32]],
    noop_event: Option<&LeafSchemaEventDecoded>,
) -> Result<CnftEvent, ParseError> {
    require_accounts(accounts.len(), pos::verify_collection::MIN)?;
    let noop = require_noop(noop_event, "unverifyCollection")?;
    Ok(CnftEvent::UnverifyCollection {
        tree: accounts[pos::verify_collection::MERKLE_TREE],
        collection: accounts[pos::verify_collection::COLLECTION_MINT],
        noop,
    })
}

fn parse_set_and_verify_collection(
    accounts: &[[u8; 32]],
    noop_event: Option<&LeafSchemaEventDecoded>,
) -> Result<CnftEvent, ParseError> {
    require_accounts(accounts.len(), pos::verify_collection::MIN)?;
    let noop = require_noop(noop_event, "setAndVerifyCollection")?;
    Ok(CnftEvent::SetAndVerifyCollection {
        tree: accounts[pos::verify_collection::MERKLE_TREE],
        collection: accounts[pos::verify_collection::COLLECTION_MINT],
        noop,
    })
}

fn parse_update_metadata(
    body: &[u8],
    accounts: &[[u8; 32]],
    noop_event: Option<&LeafSchemaEventDecoded>,
) -> Result<CnftEvent, ParseError> {
    require_accounts(accounts.len(), pos::update_metadata::MIN)?;
    let args = UpdateMetadataInstructionArgs::try_from_slice(body)
        .map_err(|e| ParseError::DecoderError(e.to_string()))?;
    let noop = require_noop(noop_event, "updateMetadata")?;
    let new_metadata = update_args_to_mint_metadata(&args.update_args, body);
    Ok(CnftEvent::UpdateMetadata {
        tree: accounts[pos::update_metadata::MERKLE_TREE],
        new_metadata,
        noop,
    })
}

// ─── helpers ────────────────────────────────────────────────────────

fn require_accounts(actual: usize, expected: usize) -> Result<(), ParseError> {
    if actual < expected {
        return Err(ParseError::InsufficientAccounts { expected, actual });
    }
    Ok(())
}

fn require_noop(
    noop_event: Option<&LeafSchemaEventDecoded>,
    ix_name: &'static str,
) -> Result<NoopOverride, ParseError> {
    let event = noop_event.ok_or_else(|| {
        ParseError::Unsupported(format!(
            "{ix_name} requires a paired noop LeafSchemaEvent to resolve new state; none found"
        ))
    })?;
    event.as_v1_override().ok_or_else(|| {
        ParseError::Unsupported(format!(
            "{ix_name}: LeafSchema V2 events aren't supported in this release"
        ))
    })
}

fn noop_to_override(noop_event: Option<&LeafSchemaEventDecoded>) -> Option<NoopOverride> {
    noop_event.and_then(LeafSchemaEventDecoded::as_v1_override)
}

/// Convert mpl-bubblegum's `MetadataArgs` into our `MintMetadata`,
/// keeping the Borsh preimage bytes (everything after the 8-byte ix
/// discriminator) so we can re-hash without re-serializing on
/// subsequent mutations.
fn to_mint_metadata(m: &MetadataArgs, ix_body_after_disc: &[u8]) -> MintMetadata {
    let creators = m
        .creators
        .iter()
        .map(|c| Creator {
            address: c.address.to_bytes(),
            verified: c.verified,
            share: c.share,
        })
        .collect();
    let collection = m
        .collection
        .as_ref()
        .map(|c| (c.key.to_bytes(), c.verified));

    MintMetadata {
        name: m.name.clone(),
        symbol: m.symbol.clone(),
        uri: m.uri.clone(),
        seller_fee_basis_points: m.seller_fee_basis_points,
        primary_sale_happened: m.primary_sale_happened,
        is_mutable: m.is_mutable,
        creators,
        collection,
        data_hash_input: ix_body_after_disc.to_vec(),
    }
}

/// Partial-update path: `UpdateArgs` is all-Option. We render whatever
/// was provided into a MintMetadata-shaped record; the apply step
/// merges this over the existing record, preserving prior fields for
/// anything left `None`.
fn update_args_to_mint_metadata(u: &UpdateArgs, ix_body_after_disc: &[u8]) -> MintMetadata {
    let creators: Vec<Creator> = u
        .creators
        .as_ref()
        .map(|list| {
            list.iter()
                .map(|c| Creator {
                    address: c.address.to_bytes(),
                    verified: c.verified,
                    share: c.share,
                })
                .collect()
        })
        .unwrap_or_default();

    MintMetadata {
        name: u.name.clone().unwrap_or_default(),
        symbol: u.symbol.clone().unwrap_or_default(),
        uri: u.uri.clone().unwrap_or_default(),
        seller_fee_basis_points: u.seller_fee_basis_points.unwrap_or(0),
        primary_sale_happened: u.primary_sale_happened.unwrap_or(false),
        is_mutable: u.is_mutable.unwrap_or(true),
        creators,
        // updateMetadata doesn't change collection membership.
        collection: None,
        data_hash_input: ix_body_after_disc.to_vec(),
    }
}

