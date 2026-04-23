//! Roundtrip tests for the LeafSchemaEvent decoder. Build real
//! mpl-bubblegum LeafSchemaEvent values, Borsh-serialize them, decode
//! with our wrapper, assert every field arrived.

use borsh::BorshSerialize;
use mpl_bubblegum::types::{BubblegumEventType, LeafSchema, Version};
use mpl_bubblegum::LeafSchemaEvent;
use solana_program::pubkey::Pubkey;
use tidepool_rpc::cnft::{decode_leaf_schema_event, DecodedLeafSchema};

fn pk(b: u8) -> Pubkey {
    Pubkey::new_from_array([b; 32])
}

#[test]
fn v1_roundtrip_preserves_every_field() {
    let event = LeafSchemaEvent::new(
        Version::V1,
        LeafSchema::V1 {
            id: pk(0x11),
            owner: pk(0x22),
            delegate: pk(0x33),
            nonce: 42,
            data_hash: [0xaa; 32],
            creator_hash: [0xbb; 32],
        },
        [0xcc; 32],
    );
    let mut bytes = Vec::new();
    event.serialize(&mut bytes).expect("serialize event");

    let out = decode_leaf_schema_event(&bytes).expect("decode event");
    match out.schema {
        DecodedLeafSchema::V1 {
            id,
            owner,
            delegate,
            nonce,
            data_hash,
            creator_hash,
        } => {
            assert_eq!(id, [0x11; 32]);
            assert_eq!(owner, [0x22; 32]);
            assert_eq!(delegate, [0x33; 32]);
            assert_eq!(nonce, 42);
            assert_eq!(data_hash, [0xaa; 32]);
            assert_eq!(creator_hash, [0xbb; 32]);
        }
        DecodedLeafSchema::V2 { .. } => panic!("expected V1"),
    }
    assert_eq!(out.leaf_hash, [0xcc; 32]);
}

#[test]
fn rejects_wrong_event_type_byte_cheaply() {
    // Force event_type = Uninitialized (0). Decoder should bail on
    // the first-byte check without invoking borsh.
    let mut bytes = vec![BubblegumEventType::Uninitialized as u8];
    bytes.resize(100, 0);
    assert!(decode_leaf_schema_event(&bytes).is_none());
}

#[test]
fn rejects_truncated_bytes() {
    assert!(decode_leaf_schema_event(&[]).is_none());
    assert!(decode_leaf_schema_event(&[BubblegumEventType::LeafSchemaEvent as u8]).is_none());
    // Valid first byte but not enough to parse — still returns None.
    assert!(decode_leaf_schema_event(&[BubblegumEventType::LeafSchemaEvent as u8, 0, 0, 1]).is_none());
}

#[test]
fn as_override_produces_matching_noop_override_for_v1() {
    let event = LeafSchemaEvent::new(
        Version::V1,
        LeafSchema::V1 {
            id: pk(1),
            owner: pk(2),
            delegate: pk(3),
            nonce: 7,
            data_hash: [4; 32],
            creator_hash: [5; 32],
        },
        [6; 32],
    );
    let mut bytes = Vec::new();
    event.serialize(&mut bytes).unwrap();
    let decoded = decode_leaf_schema_event(&bytes).unwrap();

    let ov = decoded.as_override();
    assert_eq!(ov.nonce, 7);
    assert_eq!(ov.leaf_index, 7);
    assert_eq!(ov.id, [1; 32]);
    assert_eq!(ov.owner, [2; 32]);
    assert_eq!(ov.delegate, [3; 32]);
    assert_eq!(ov.data_hash, [4; 32]);
    assert_eq!(ov.creator_hash, [5; 32]);
    assert_eq!(ov.leaf_hash, [6; 32]);
    assert!(!decoded.is_v2());
}
