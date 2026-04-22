//! Parser integration tests. Synthesize valid ix payloads via
//! mpl-bubblegum's Borsh-derived args types, concatenate the
//! discriminator + body, feed through `parse_bubblegum_instruction`,
//! assert the resulting CnftEvent shape.
//!
//! Also contains a drift test: for every ix we handle, verify that
//! our hardcoded discriminator matches what
//! `mpl_bubblegum::instructions::<Ix>InstructionData::new()` emits
//! when Borsh-serialized. Catches any rebase of the upstream
//! anchor discriminators on version bump.

use borsh::BorshSerialize;
use mpl_bubblegum::instructions::{
    BurnInstructionArgs, BurnInstructionData, CreateTreeConfigInstructionArgs,
    CreateTreeConfigInstructionData, DelegateInstructionArgs, DelegateInstructionData,
    MintToCollectionV1InstructionArgs, MintToCollectionV1InstructionData, MintV1InstructionArgs,
    MintV1InstructionData, SetAndVerifyCollectionInstructionData, TransferInstructionArgs,
    TransferInstructionData, UnverifyCollectionInstructionData, UnverifyCreatorInstructionData,
    UpdateMetadataInstructionArgs, UpdateMetadataInstructionData, VerifyCollectionInstructionData,
    VerifyCreatorInstructionData,
};
use mpl_bubblegum::types::{
    LeafSchema, MetadataArgs, TokenProgramVersion, UpdateArgs, Version,
};
use mpl_bubblegum::LeafSchemaEvent;
use solana_program::pubkey::Pubkey;

use tidepool_rpc::cnft::{
    decode_leaf_schema_event,
    parser::{
        BURN_DISC, CREATE_TREE_CONFIG_DISC, DELEGATE_DISC, MINT_TO_COLLECTION_V1_DISC, MINT_V1_DISC,
        SET_AND_VERIFY_COLLECTION_DISC, TRANSFER_DISC, UNVERIFY_COLLECTION_DISC,
        UNVERIFY_CREATOR_DISC, UPDATE_METADATA_DISC, VERIFY_COLLECTION_DISC, VERIFY_CREATOR_DISC,
    },
    parse_bubblegum_instruction, CnftEvent, ParseError,
};

const TREE: [u8; 32] = [0x11; 32];
const OWNER: [u8; 32] = [0x22; 32];
const DELEGATE: [u8; 32] = [0x33; 32];
const NEW_OWNER: [u8; 32] = [0x44; 32];
const NEW_DELEGATE: [u8; 32] = [0x55; 32];
const COLLECTION_MINT: [u8; 32] = [0x66; 32];
const CREATOR: [u8; 32] = [0x77; 32];
const FILLER: [u8; 32] = [0x99; 32];

fn encode<T: BorshSerialize>(value: &T) -> Vec<u8> {
    let mut out = Vec::new();
    value.serialize(&mut out).expect("borsh serialize");
    out
}

fn stub_metadata_args() -> MetadataArgs {
    MetadataArgs {
        name: "Test".into(),
        symbol: "TST".into(),
        uri: "https://example.com/t.json".into(),
        seller_fee_basis_points: 500,
        primary_sale_happened: false,
        is_mutable: true,
        edition_nonce: None,
        token_standard: None,
        collection: None,
        uses: None,
        token_program_version: TokenProgramVersion::Original,
        creators: vec![],
    }
}

fn noop_event_v1(nonce: u64) -> Vec<u8> {
    let event = LeafSchemaEvent::new(
        Version::V1,
        LeafSchema::V1 {
            id: Pubkey::new_from_array([0x99; 32]),
            owner: Pubkey::new_from_array(OWNER),
            delegate: Pubkey::new_from_array(DELEGATE),
            nonce,
            data_hash: [0xaa; 32],
            creator_hash: [0xbb; 32],
        },
        [0xcc; 32],
    );
    encode(&event)
}

// ─── discriminator drift check ──────────────────────────────────────

#[test]
fn hardcoded_discriminators_match_mpl_bubblegum_runtime() {
    let check = |name: &str, expected: [u8; 8], actual: Vec<u8>| {
        assert_eq!(
            actual[..8],
            expected,
            "discriminator drift for {name} — update hardcoded constant in parser.rs"
        );
    };
    check(
        "create_tree_config",
        CREATE_TREE_CONFIG_DISC,
        encode(&CreateTreeConfigInstructionData::new()),
    );
    check("mint_v1", MINT_V1_DISC, encode(&MintV1InstructionData::new()));
    check(
        "mint_to_collection_v1",
        MINT_TO_COLLECTION_V1_DISC,
        encode(&MintToCollectionV1InstructionData::new()),
    );
    check(
        "transfer",
        TRANSFER_DISC,
        encode(&TransferInstructionData::new()),
    );
    check("burn", BURN_DISC, encode(&BurnInstructionData::new()));
    check(
        "delegate",
        DELEGATE_DISC,
        encode(&DelegateInstructionData::new()),
    );
    check(
        "verify_creator",
        VERIFY_CREATOR_DISC,
        encode(&VerifyCreatorInstructionData::new()),
    );
    check(
        "unverify_creator",
        UNVERIFY_CREATOR_DISC,
        encode(&UnverifyCreatorInstructionData::new()),
    );
    check(
        "verify_collection",
        VERIFY_COLLECTION_DISC,
        encode(&VerifyCollectionInstructionData::new()),
    );
    check(
        "unverify_collection",
        UNVERIFY_COLLECTION_DISC,
        encode(&UnverifyCollectionInstructionData::new()),
    );
    check(
        "set_and_verify_collection",
        SET_AND_VERIFY_COLLECTION_DISC,
        encode(&SetAndVerifyCollectionInstructionData::new()),
    );
    check(
        "update_metadata",
        UPDATE_METADATA_DISC,
        encode(&UpdateMetadataInstructionData::new()),
    );
}

// ─── per-ix parsing ─────────────────────────────────────────────────

#[test]
fn create_tree_config_yields_tree_info() {
    let mut data = CREATE_TREE_CONFIG_DISC.to_vec();
    data.extend(encode(&CreateTreeConfigInstructionArgs {
        max_depth: 20,
        max_buffer_size: 64,
        public: Some(false),
    }));
    let accounts = [FILLER, TREE, FILLER, FILLER, FILLER, FILLER, FILLER];

    let res = parse_bubblegum_instruction(&data, &accounts, None).unwrap().unwrap();
    match res {
        CnftEvent::CreateTree { tree, depth, max_buffer_size } => {
            assert_eq!(tree, TREE);
            assert_eq!(depth, 20);
            assert_eq!(max_buffer_size, 64);
        }
        other => panic!("expected CreateTree, got {other:?}"),
    }
}

#[test]
fn mint_v1_yields_mint_event_with_metadata() {
    let mut data = MINT_V1_DISC.to_vec();
    data.extend(encode(&MintV1InstructionArgs {
        metadata: stub_metadata_args(),
    }));
    let accounts = [FILLER, OWNER, DELEGATE, TREE, FILLER, FILLER, FILLER, FILLER, FILLER];

    let res = parse_bubblegum_instruction(&data, &accounts, None).unwrap().unwrap();
    match res {
        CnftEvent::Mint {
            tree,
            owner,
            delegate,
            metadata,
            verify_collection,
            ..
        } => {
            assert_eq!(tree, TREE);
            assert_eq!(owner, OWNER);
            assert_eq!(delegate, DELEGATE);
            assert_eq!(metadata.name, "Test");
            assert_eq!(verify_collection, None);
        }
        other => panic!("expected Mint, got {other:?}"),
    }
}

#[test]
fn mint_to_collection_v1_marks_verify_collection() {
    let mut data = MINT_TO_COLLECTION_V1_DISC.to_vec();
    data.extend(encode(&MintToCollectionV1InstructionArgs {
        metadata: stub_metadata_args(),
    }));
    let mut accounts = vec![FILLER; 16];
    accounts[1] = OWNER;
    accounts[2] = DELEGATE;
    accounts[3] = TREE;
    accounts[8] = COLLECTION_MINT;

    let res = parse_bubblegum_instruction(&data, &accounts, None).unwrap().unwrap();
    match res {
        CnftEvent::Mint {
            metadata,
            verify_collection,
            ..
        } => {
            assert_eq!(verify_collection, Some(COLLECTION_MINT));
            assert_eq!(metadata.collection, Some((COLLECTION_MINT, true)));
        }
        other => panic!("expected Mint, got {other:?}"),
    }
}

#[test]
fn transfer_yields_new_owner_and_delegate_equals_new_owner() {
    let mut data = TRANSFER_DISC.to_vec();
    data.extend(encode(&TransferInstructionArgs {
        root: [1; 32],
        data_hash: [2; 32],
        creator_hash: [3; 32],
        nonce: 7,
        index: 7,
    }));
    let accounts = [FILLER, OWNER, DELEGATE, NEW_OWNER, TREE, FILLER, FILLER, FILLER];

    let res = parse_bubblegum_instruction(&data, &accounts, None).unwrap().unwrap();
    match res {
        CnftEvent::Transfer {
            tree,
            leaf_index,
            nonce,
            new_owner,
            new_delegate,
            data_hash,
            ..
        } => {
            assert_eq!(tree, TREE);
            assert_eq!(leaf_index, 7);
            assert_eq!(nonce, 7);
            assert_eq!(new_owner, NEW_OWNER);
            assert_eq!(new_delegate, NEW_OWNER, "delegate resets to newOwner");
            assert_eq!(data_hash, [2; 32]);
        }
        other => panic!("expected Transfer, got {other:?}"),
    }
}

#[test]
fn burn_yields_tree_leaf_and_nonce() {
    let mut data = BURN_DISC.to_vec();
    data.extend(encode(&BurnInstructionArgs {
        root: [0; 32],
        data_hash: [0; 32],
        creator_hash: [0; 32],
        nonce: 42,
        index: 42,
    }));
    let accounts = [FILLER, OWNER, DELEGATE, TREE, FILLER, FILLER, FILLER];

    let res = parse_bubblegum_instruction(&data, &accounts, None).unwrap().unwrap();
    match res {
        CnftEvent::Burn { tree, leaf_index, nonce, .. } => {
            assert_eq!(tree, TREE);
            assert_eq!(leaf_index, 42);
            assert_eq!(nonce, 42);
        }
        other => panic!("expected Burn, got {other:?}"),
    }
}

#[test]
fn delegate_yields_new_delegate() {
    let mut data = DELEGATE_DISC.to_vec();
    data.extend(encode(&DelegateInstructionArgs {
        root: [0; 32],
        data_hash: [0xaa; 32],
        creator_hash: [0xbb; 32],
        nonce: 3,
        index: 3,
    }));
    let accounts = [FILLER, OWNER, DELEGATE, NEW_DELEGATE, TREE, FILLER, FILLER, FILLER];

    let res = parse_bubblegum_instruction(&data, &accounts, None).unwrap().unwrap();
    match res {
        CnftEvent::Delegate { new_delegate, data_hash, creator_hash, .. } => {
            assert_eq!(new_delegate, NEW_DELEGATE);
            assert_eq!(data_hash, [0xaa; 32]);
            assert_eq!(creator_hash, [0xbb; 32]);
        }
        other => panic!("expected Delegate, got {other:?}"),
    }
}

#[test]
fn verify_creator_without_noop_event_is_unsupported() {
    let data = VERIFY_CREATOR_DISC.to_vec();
    let mut accounts = vec![FILLER; 9];
    accounts[3] = TREE;
    accounts[5] = CREATOR;
    let err = parse_bubblegum_instruction(&data, &accounts, None).unwrap_err();
    assert!(matches!(err, ParseError::Unsupported(_)), "got {err:?}");
}

#[test]
fn verify_creator_with_noop_event_yields_event_with_authoritative_state() {
    let data = VERIFY_CREATOR_DISC.to_vec();
    let mut accounts = vec![FILLER; 9];
    accounts[3] = TREE;
    accounts[5] = CREATOR;
    let noop_bytes = noop_event_v1(5);
    let noop = decode_leaf_schema_event(&noop_bytes).unwrap();

    let res = parse_bubblegum_instruction(&data, &accounts, Some(&noop))
        .unwrap()
        .unwrap();
    match res {
        CnftEvent::VerifyCreator { tree, creator, noop } => {
            assert_eq!(tree, TREE);
            assert_eq!(creator, CREATOR);
            assert_eq!(noop.nonce, 5);
            assert_eq!(noop.owner, OWNER);
            assert_eq!(noop.data_hash, [0xaa; 32]);
        }
        other => panic!("expected VerifyCreator, got {other:?}"),
    }
}

#[test]
fn update_metadata_with_partial_args_carries_provided_fields() {
    let mut data = UPDATE_METADATA_DISC.to_vec();
    data.extend(encode(&UpdateMetadataInstructionArgs {
        root: [0; 32],
        nonce: 2,
        index: 2,
        current_metadata: stub_metadata_args(),
        update_args: UpdateArgs {
            name: Some("NewName".into()),
            symbol: None,
            uri: None,
            creators: None,
            seller_fee_basis_points: None,
            primary_sale_happened: None,
            is_mutable: None,
        },
    }));
    let mut accounts = vec![FILLER; 13];
    accounts[8] = TREE;
    let noop_bytes = noop_event_v1(2);
    let noop = decode_leaf_schema_event(&noop_bytes).unwrap();

    let res = parse_bubblegum_instruction(&data, &accounts, Some(&noop))
        .unwrap()
        .unwrap();
    match res {
        CnftEvent::UpdateMetadata { new_metadata, noop: override_, .. } => {
            assert_eq!(new_metadata.name, "NewName");
            assert_eq!(override_.nonce, 2);
        }
        other => panic!("expected UpdateMetadata, got {other:?}"),
    }
}

// ─── error paths ────────────────────────────────────────────────────

#[test]
fn unknown_discriminator_returns_error() {
    let data = [99u8, 99, 99, 99, 99, 99, 99, 99];
    let err = parse_bubblegum_instruction(&data, &[], None).unwrap_err();
    assert!(matches!(err, ParseError::UnknownDiscriminator { .. }));
}

#[test]
fn truncated_data_returns_error() {
    let err = parse_bubblegum_instruction(&[1, 2, 3], &[], None).unwrap_err();
    assert!(matches!(err, ParseError::TruncatedData { .. }));
}

#[test]
fn insufficient_accounts_returns_error() {
    let mut data = BURN_DISC.to_vec();
    data.extend(encode(&BurnInstructionArgs {
        root: [0; 32],
        data_hash: [0; 32],
        creator_hash: [0; 32],
        nonce: 0,
        index: 0,
    }));
    let accounts = [FILLER, FILLER]; // too few
    let err = parse_bubblegum_instruction(&data, &accounts, None).unwrap_err();
    assert!(matches!(err, ParseError::InsufficientAccounts { .. }));
}
