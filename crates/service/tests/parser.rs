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
    BurnInstructionArgs, BurnInstructionData, BurnV2InstructionArgs, BurnV2InstructionData,
    CreateTreeConfigInstructionArgs, CreateTreeConfigInstructionData,
    CreateTreeConfigV2InstructionArgs, CreateTreeConfigV2InstructionData, DelegateInstructionArgs,
    DelegateInstructionData, DelegateV2InstructionArgs, DelegateV2InstructionData,
    MintToCollectionV1InstructionArgs, MintToCollectionV1InstructionData, MintV1InstructionArgs,
    MintV1InstructionData, MintV2InstructionArgs, MintV2InstructionData,
    SetAndVerifyCollectionInstructionData, SetCollectionV2InstructionData, TransferInstructionArgs,
    TransferInstructionData, TransferV2InstructionArgs, TransferV2InstructionData,
    UnverifyCollectionInstructionData, UnverifyCreatorInstructionData,
    UnverifyCreatorV2InstructionData, UpdateMetadataInstructionArgs, UpdateMetadataInstructionData,
    UpdateMetadataV2InstructionData, VerifyCollectionInstructionData,
    VerifyCreatorInstructionData, VerifyCreatorV2InstructionData,
};
use mpl_bubblegum::types::{
    LeafSchema, MetadataArgs, MetadataArgsV2, TokenProgramVersion, UpdateArgs, Version,
};
use mpl_bubblegum::LeafSchemaEvent;
use solana_program::pubkey::Pubkey;

use tidepool_rpc::cnft::{
    decode_leaf_schema_event,
    parser::{
        BURN_DISC, BURN_V2_DISC, CREATE_TREE_CONFIG_DISC, CREATE_TREE_CONFIG_V2_DISC, DELEGATE_DISC,
        DELEGATE_V2_DISC, MINT_TO_COLLECTION_V1_DISC, MINT_V1_DISC, MINT_V2_DISC,
        SET_AND_VERIFY_COLLECTION_DISC, SET_COLLECTION_V2_DISC, TRANSFER_DISC, TRANSFER_V2_DISC,
        UNVERIFY_COLLECTION_DISC, UNVERIFY_CREATOR_DISC, UNVERIFY_CREATOR_V2_DISC,
        UPDATE_METADATA_DISC, UPDATE_METADATA_V2_DISC, VERIFY_COLLECTION_DISC,
        VERIFY_CREATOR_DISC, VERIFY_CREATOR_V2_DISC,
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
#[allow(clippy::too_many_lines)]
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

    // V2 family.
    check(
        "create_tree_config_v2",
        CREATE_TREE_CONFIG_V2_DISC,
        encode(&CreateTreeConfigV2InstructionData::new()),
    );
    check(
        "mint_v2",
        MINT_V2_DISC,
        encode(&MintV2InstructionData::new()),
    );
    check(
        "transfer_v2",
        TRANSFER_V2_DISC,
        encode(&TransferV2InstructionData::new()),
    );
    check(
        "burn_v2",
        BURN_V2_DISC,
        encode(&BurnV2InstructionData::new()),
    );
    check(
        "delegate_v2",
        DELEGATE_V2_DISC,
        encode(&DelegateV2InstructionData::new()),
    );
    check(
        "verify_creator_v2",
        VERIFY_CREATOR_V2_DISC,
        encode(&VerifyCreatorV2InstructionData::new()),
    );
    check(
        "unverify_creator_v2",
        UNVERIFY_CREATOR_V2_DISC,
        encode(&UnverifyCreatorV2InstructionData::new()),
    );
    check(
        "update_metadata_v2",
        UPDATE_METADATA_V2_DISC,
        encode(&UpdateMetadataV2InstructionData::new()),
    );
    check(
        "set_collection_v2",
        SET_COLLECTION_V2_DISC,
        encode(&SetCollectionV2InstructionData::new()),
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

// ─── V2 ixs ─────────────────────────────────────────────────────────
// V2 parsing always requires a paired noop LeafSchemaEvent — the V2
// leaf hash folds in collection_hash/asset_data_hash/flags we don't
// reconstruct, so the noop's emitted leaf_hash is the only source of
// truth. These tests drive the V2 path end-to-end.

const BUBBLEGUM_PROGRAM_ID_BYTES: [u8; 32] = mpl_bubblegum::ID.to_bytes();

fn stub_metadata_args_v2() -> MetadataArgsV2 {
    MetadataArgsV2 {
        name: "V2 Test".into(),
        symbol: "V2T".into(),
        uri: "https://example.com/v2.json".into(),
        seller_fee_basis_points: 250,
        primary_sale_happened: false,
        is_mutable: true,
        token_standard: None,
        collection: None,
        creators: vec![],
    }
}

fn noop_event_v2(nonce: u64) -> Vec<u8> {
    let event = LeafSchemaEvent::new(
        Version::V2,
        LeafSchema::V2 {
            id: Pubkey::new_from_array([0x9a; 32]),
            owner: Pubkey::new_from_array(OWNER),
            delegate: Pubkey::new_from_array(DELEGATE),
            nonce,
            data_hash: [0xda; 32],
            creator_hash: [0xcc; 32],
            collection_hash: [0xc1; 32],
            asset_data_hash: [0xad; 32],
            flags: 0,
        },
        [0xab; 32],
    );
    encode(&event)
}

#[test]
fn create_tree_config_v2_yields_tree_info() {
    let mut data = CREATE_TREE_CONFIG_V2_DISC.to_vec();
    data.extend(encode(&CreateTreeConfigV2InstructionArgs {
        max_depth: 14,
        max_buffer_size: 64,
        public: Some(false),
    }));
    let accounts = [FILLER, TREE, FILLER, FILLER, FILLER, FILLER, FILLER];
    let res = parse_bubblegum_instruction(&data, &accounts, None).unwrap().unwrap();
    match res {
        CnftEvent::CreateTree { tree, depth, max_buffer_size } => {
            assert_eq!(tree, TREE);
            assert_eq!(depth, 14);
            assert_eq!(max_buffer_size, 64);
        }
        other => panic!("expected CreateTree, got {other:?}"),
    }
}

#[test]
fn mint_v2_with_noop_carries_authoritative_state() {
    let mut data = MINT_V2_DISC.to_vec();
    data.extend(encode(&MintV2InstructionArgs {
        metadata: stub_metadata_args_v2(),
        asset_data: None,
        asset_data_schema: None,
    }));
    // 13-slot layout; optional slots filled with Bubblegum placeholder.
    let mut accounts = vec![BUBBLEGUM_PROGRAM_ID_BYTES; 13];
    accounts[4] = OWNER; // leaf_owner
    accounts[6] = TREE;  // merkle_tree
    // leaf_delegate at 5 = placeholder → resolves to owner.
    // core_collection at 7 = placeholder → verify_collection = None.

    let noop_bytes = noop_event_v2(11);
    let noop = decode_leaf_schema_event(&noop_bytes).unwrap();

    let res = parse_bubblegum_instruction(&data, &accounts, Some(&noop))
        .unwrap()
        .unwrap();
    match res {
        CnftEvent::Mint {
            tree,
            owner,
            delegate,
            verify_collection,
            noop: ov,
            metadata,
            ..
        } => {
            assert_eq!(tree, TREE);
            assert_eq!(owner, OWNER);
            assert_eq!(delegate, OWNER, "absent leaf_delegate defaults to owner");
            assert_eq!(verify_collection, None);
            assert_eq!(metadata.name, "V2 Test");
            let ov = ov.expect("noop carried");
            assert_eq!(ov.nonce, 11);
            assert_eq!(ov.leaf_hash, [0xab; 32]);
        }
        other => panic!("expected Mint, got {other:?}"),
    }
}

#[test]
fn mint_v2_without_noop_is_unsupported() {
    let mut data = MINT_V2_DISC.to_vec();
    data.extend(encode(&MintV2InstructionArgs {
        metadata: stub_metadata_args_v2(),
        asset_data: None,
        asset_data_schema: None,
    }));
    let mut accounts = vec![BUBBLEGUM_PROGRAM_ID_BYTES; 13];
    accounts[4] = OWNER;
    accounts[6] = TREE;

    let err = parse_bubblegum_instruction(&data, &accounts, None).unwrap_err();
    assert!(matches!(err, ParseError::Unsupported(_)), "got {err:?}");
}

#[test]
fn transfer_v2_uses_noop_leaf_hash() {
    let mut data = TRANSFER_V2_DISC.to_vec();
    data.extend(encode(&TransferV2InstructionArgs {
        root: [0; 32],
        data_hash: [0; 32],
        creator_hash: [0; 32],
        asset_data_hash: None,
        flags: None,
        nonce: 7,
        index: 7,
    }));
    let mut accounts = vec![BUBBLEGUM_PROGRAM_ID_BYTES; 11];
    accounts[5] = NEW_OWNER;
    accounts[6] = TREE;

    let noop_bytes = noop_event_v2(7);
    let noop = decode_leaf_schema_event(&noop_bytes).unwrap();

    let res = parse_bubblegum_instruction(&data, &accounts, Some(&noop))
        .unwrap()
        .unwrap();
    match res {
        CnftEvent::Transfer {
            tree,
            leaf_index,
            new_owner,
            new_delegate,
            noop: ov,
            ..
        } => {
            assert_eq!(tree, TREE);
            assert_eq!(leaf_index, 7);
            assert_eq!(new_owner, NEW_OWNER);
            assert_eq!(new_delegate, NEW_OWNER);
            assert_eq!(ov.unwrap().leaf_hash, [0xab; 32]);
        }
        other => panic!("expected Transfer, got {other:?}"),
    }
}

#[test]
fn burn_v2_yields_burn_with_noop() {
    let mut data = BURN_V2_DISC.to_vec();
    data.extend(encode(&BurnV2InstructionArgs {
        root: [0; 32],
        data_hash: [0; 32],
        creator_hash: [0; 32],
        asset_data_hash: None,
        flags: None,
        nonce: 3,
        index: 3,
    }));
    let mut accounts = vec![BUBBLEGUM_PROGRAM_ID_BYTES; 12];
    accounts[5] = TREE;

    let noop_bytes = noop_event_v2(3);
    let noop = decode_leaf_schema_event(&noop_bytes).unwrap();

    let res = parse_bubblegum_instruction(&data, &accounts, Some(&noop))
        .unwrap()
        .unwrap();
    match res {
        CnftEvent::Burn { tree, leaf_index, nonce, .. } => {
            assert_eq!(tree, TREE);
            assert_eq!(leaf_index, 3);
            assert_eq!(nonce, 3);
        }
        other => panic!("expected Burn, got {other:?}"),
    }
}

#[test]
fn delegate_v2_reads_new_delegate_from_slot() {
    let mut data = DELEGATE_V2_DISC.to_vec();
    data.extend(encode(&DelegateV2InstructionArgs {
        root: [0; 32],
        data_hash: [0; 32],
        creator_hash: [0; 32],
        collection_hash: None,
        asset_data_hash: None,
        flags: None,
        nonce: 9,
        index: 9,
    }));
    let mut accounts = vec![BUBBLEGUM_PROGRAM_ID_BYTES; 9];
    accounts[4] = NEW_DELEGATE;
    accounts[5] = TREE;

    let noop_bytes = noop_event_v2(9);
    let noop = decode_leaf_schema_event(&noop_bytes).unwrap();

    let res = parse_bubblegum_instruction(&data, &accounts, Some(&noop))
        .unwrap()
        .unwrap();
    match res {
        CnftEvent::Delegate { new_delegate, .. } => {
            assert_eq!(new_delegate, NEW_DELEGATE);
        }
        other => panic!("expected Delegate, got {other:?}"),
    }
}

#[test]
fn verify_creator_v2_yields_event_with_creator_from_slot() {
    let data = VERIFY_CREATOR_V2_DISC.to_vec();
    let mut accounts = vec![BUBBLEGUM_PROGRAM_ID_BYTES; 9];
    accounts[2] = CREATOR;
    accounts[5] = TREE;

    let noop_bytes = noop_event_v2(0);
    let noop = decode_leaf_schema_event(&noop_bytes).unwrap();

    let res = parse_bubblegum_instruction(&data, &accounts, Some(&noop))
        .unwrap()
        .unwrap();
    match res {
        CnftEvent::VerifyCreator { tree, creator, .. } => {
            assert_eq!(tree, TREE);
            assert_eq!(creator, CREATOR);
        }
        other => panic!("expected VerifyCreator, got {other:?}"),
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
