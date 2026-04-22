//! MplCore decoder tests. Build real `BaseAssetV1` + plugin-laden
//! account data using mpl-core's own types, serialize them via
//! Borsh, feed through the decoder, assert DAS shape.

use borsh::BorshSerialize;
use mpl_core::accounts::{BaseAssetV1, BaseCollectionV1};
use mpl_core::types::{Key, UpdateAuthority};
use solana_program::pubkey::Pubkey;

use tidepool_rpc::das::{AccountDecoder, MplCoreDecoder};

fn pk(b: u8) -> Pubkey {
    Pubkey::new_from_array([b; 32])
}

fn encode_asset(asset: &BaseAssetV1) -> Vec<u8> {
    // Anchor-style: discriminator byte (Key) + Borsh-serialized body.
    // mpl-core's BaseAssetV1 already carries `key` as its first field,
    // so a straight Borsh serialization produces the on-chain layout.
    let mut out = Vec::new();
    asset.serialize(&mut out).expect("serialize");
    out
}

fn encode_collection(c: &BaseCollectionV1) -> Vec<u8> {
    let mut out = Vec::new();
    c.serialize(&mut out).expect("serialize");
    out
}

#[test]
fn decoder_program_id_and_name() {
    let d = MplCoreDecoder;
    assert_eq!(d.program_id(), "CoREENxT6tW1HoK8ypY1SxRMZTcVPm7R94rH4PZNhX7d");
    assert_eq!(d.name(), "MplCoreAsset");
}

#[test]
fn empty_data_returns_none() {
    let decoded = MplCoreDecoder.decode("anypk", &[]).unwrap();
    assert!(decoded.is_none());
}

#[test]
fn unknown_key_byte_returns_none() {
    // Key::Uninitialized = 0 is neither AssetV1 (1) nor CollectionV1 (5).
    let decoded = MplCoreDecoder.decode("anypk", &[0, 1, 2, 3]).unwrap();
    assert!(decoded.is_none());
}

#[test]
fn asset_v1_no_plugins_yields_basic_das_shape() {
    let asset = BaseAssetV1 {
        key: Key::AssetV1,
        owner: pk(0x22),
        update_authority: UpdateAuthority::Address(pk(0x33)),
        name: "Test Asset".into(),
        uri: "https://example.com/t.json".into(),
        seq: None,
    };
    let data = encode_asset(&asset);

    let pubkey_str = "TestAssetPubkey".to_string();
    let decoded = MplCoreDecoder.decode(&pubkey_str, &data).unwrap().expect("Some");

    assert_eq!(decoded.id, pubkey_str);
    assert_eq!(decoded.interface, "MplCoreAsset");
    assert_eq!(decoded.content.metadata.name, "Test Asset");
    assert_eq!(decoded.content.json_uri, "https://example.com/t.json");
    assert_eq!(decoded.ownership.owner, pk(0x22).to_string());
    assert!(!decoded.ownership.frozen);
    assert!(!decoded.ownership.delegated);
    assert_eq!(decoded.authorities.len(), 1);
    assert_eq!(decoded.authorities[0].address, pk(0x33).to_string());
    assert!(decoded.grouping.is_empty(), "no collection → no grouping");
    assert!(decoded.compression.is_none(), "uncompressed asset");
}

#[test]
fn asset_v1_with_collection_update_authority_yields_grouping() {
    let asset = BaseAssetV1 {
        key: Key::AssetV1,
        owner: pk(0x22),
        update_authority: UpdateAuthority::Collection(pk(0x77)),
        name: "Collection Member".into(),
        uri: "https://example.com/m.json".into(),
        seq: None,
    };
    let data = encode_asset(&asset);
    let decoded = MplCoreDecoder.decode("pk", &data).unwrap().unwrap();

    assert_eq!(decoded.grouping.len(), 1);
    assert_eq!(decoded.grouping[0].group_key, "collection");
    assert_eq!(decoded.grouping[0].group_value, pk(0x77).to_string());
}

#[test]
fn collection_v1_yields_collection_interface() {
    let collection = BaseCollectionV1 {
        key: Key::CollectionV1,
        update_authority: pk(0x33),
        name: "Test Collection".into(),
        uri: "https://example.com/c.json".into(),
        num_minted: 5,
        current_size: 5,
    };
    let data = encode_collection(&collection);
    let decoded = MplCoreDecoder.decode("pk", &data).unwrap().unwrap();

    assert_eq!(decoded.interface, "MplCoreCollection");
    assert_eq!(decoded.content.metadata.name, "Test Collection");
    // Collections don't belong to a grouping — they ARE the group.
    assert!(decoded.grouping.is_empty());
}
