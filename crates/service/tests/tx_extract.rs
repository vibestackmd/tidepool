//! tx_extract integration tests. Build `RpcTransactionResponse`
//! values by hand (via serde_json → deserialize) with carefully-
//! crafted inner instruction groups, then assert the extraction
//! finds the right Bubblegum ixs and pairs them with noop events.

use borsh::BorshSerialize;
use mpl_bubblegum::types::{LeafSchema, Version};
use mpl_bubblegum::LeafSchemaEvent;
use serde_json::json;
use solana_program::pubkey::Pubkey;

use tidepool_rpc::cnft::{extract_bubblegum_ixs, RpcTransactionResponse, BUBBLEGUM_PROGRAM_ID};

const SPL_NOOP: &str = "noopb9bkMVfRPU8AsbpTUg8AQkHtKwMYZiFUjNRtMmV";
const OTHER_PROGRAM: &str = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA";

// Known-good 32-byte-decode base58 strings. `11111…` is the system
// program (32 zero bytes). The others are well-known mainnet pubkeys
// that just happen to be conveniently memorable.
const ADDR_SYS: &str = "11111111111111111111111111111111";
const ADDR_RENT: &str = "SysvarRent111111111111111111111111111111111";

fn enc_bs58(bytes: &[u8]) -> String {
    bs58::encode(bytes).into_string()
}

fn borsh_vec<T: BorshSerialize>(v: &T) -> Vec<u8> {
    let mut out = Vec::new();
    v.serialize(&mut out).expect("borsh serialize");
    out
}

fn leaf_event_bytes() -> Vec<u8> {
    borsh_vec(&LeafSchemaEvent::new(
        Version::V1,
        LeafSchema::V1 {
            id: Pubkey::new_from_array([0x99; 32]),
            owner: Pubkey::new_from_array([0x22; 32]),
            delegate: Pubkey::new_from_array([0x33; 32]),
            nonce: 5,
            data_hash: [0xaa; 32],
            creator_hash: [0xbb; 32],
        },
        [0xcc; 32],
    ))
}

fn from_json(v: serde_json::Value) -> RpcTransactionResponse {
    serde_json::from_value(v).expect("tx deserialize")
}

#[test]
fn extracts_a_single_outer_bubblegum_ix() {
    let ix_data = vec![1u8, 2, 3, 4];
    let tx = from_json(json!({
        "meta": { "err": null, "innerInstructions": [] },
        "transaction": {
            "message": {
                "accountKeys": [ADDR_SYS, ADDR_RENT, BUBBLEGUM_PROGRAM_ID],
                "instructions": [
                    { "programIdIndex": 2, "accounts": [0, 1], "data": enc_bs58(&ix_data) }
                ]
            }
        }
    }));
    let ixs = extract_bubblegum_ixs(&tx);
    assert_eq!(ixs.len(), 1);
    assert_eq!(ixs[0].data, vec![1u8, 2, 3, 4]);
    assert_eq!(ixs[0].accounts.len(), 2);
    assert_eq!(ixs[0].accounts[0], [0u8; 32]); // system program as 32 zeros
    assert!(ixs[0].noop_event.is_none());
}

#[test]
fn ignores_non_bubblegum_ixs() {
    let tx = from_json(json!({
        "meta": { "err": null, "innerInstructions": [] },
        "transaction": {
            "message": {
                "accountKeys": [ADDR_SYS, ADDR_RENT, OTHER_PROGRAM],
                "instructions": [
                    { "programIdIndex": 2, "accounts": [0, 1], "data": enc_bs58(&[1]) }
                ]
            }
        }
    }));
    assert_eq!(extract_bubblegum_ixs(&tx).len(), 0);
}

#[test]
fn skips_failed_txs() {
    let tx = from_json(json!({
        "meta": {
            "err": { "InstructionError": [0, "Custom"] },
            "innerInstructions": []
        },
        "transaction": {
            "message": {
                "accountKeys": [ADDR_SYS, BUBBLEGUM_PROGRAM_ID],
                "instructions": [
                    { "programIdIndex": 1, "accounts": [0], "data": enc_bs58(&[1]) }
                ]
            }
        }
    }));
    assert_eq!(extract_bubblegum_ixs(&tx).len(), 0);
}

#[test]
fn pairs_outer_bubblegum_ix_with_inner_noop_leaf_event() {
    let tx = from_json(json!({
        "meta": {
            "err": null,
            "innerInstructions": [
                {
                    "index": 0,
                    "instructions": [
                        // Bubblegum's own noop CPI carrying LeafSchemaEvent.
                        { "programIdIndex": 3, "accounts": [], "data": enc_bs58(&leaf_event_bytes()) }
                    ]
                }
            ]
        },
        "transaction": {
            "message": {
                "accountKeys": [ADDR_SYS, ADDR_RENT, BUBBLEGUM_PROGRAM_ID, SPL_NOOP],
                "instructions": [
                    { "programIdIndex": 2, "accounts": [0, 1], "data": enc_bs58(&[1, 2, 3, 4, 5, 6, 7, 8]) }
                ]
            }
        }
    }));
    let ixs = extract_bubblegum_ixs(&tx);
    assert_eq!(ixs.len(), 1);
    let event = ixs[0]
        .noop_event
        .as_ref()
        .expect("outer ix should be paired with noop event");
    match &event.schema {
        tidepool_rpc::cnft::DecodedLeafSchema::V1 { nonce, owner, .. } => {
            assert_eq!(*nonce, 5);
            assert_eq!(owner, &[0x22; 32]);
        }
        tidepool_rpc::cnft::DecodedLeafSchema::V2 { .. } => panic!("expected V1"),
    }
}

#[test]
fn walks_inner_bubblegum_calls_from_wrapper_programs() {
    // Outer ix is a wrapper program (OTHER_PROGRAM) that CPIs into
    // Bubblegum, which then CPIs into noop with a LeafSchemaEvent.
    let wrapper_data = vec![0xaau8];
    let bg_data = vec![0xbbu8, 0xcc];
    let tx = from_json(json!({
        "meta": {
            "err": null,
            "innerInstructions": [
                {
                    "index": 0,
                    "instructions": [
                        { "programIdIndex": 3, "accounts": [0, 1], "data": enc_bs58(&bg_data) },
                        { "programIdIndex": 4, "accounts": [], "data": enc_bs58(&leaf_event_bytes()) }
                    ]
                }
            ]
        },
        "transaction": {
            "message": {
                "accountKeys": [ADDR_SYS, ADDR_RENT, OTHER_PROGRAM, BUBBLEGUM_PROGRAM_ID, SPL_NOOP],
                "instructions": [
                    { "programIdIndex": 2, "accounts": [0, 1], "data": enc_bs58(&wrapper_data) }
                ]
            }
        }
    }));

    let ixs = extract_bubblegum_ixs(&tx);
    assert_eq!(ixs.len(), 1, "should catch the inner Bubblegum CPI");
    assert_eq!(ixs[0].data, vec![0xbbu8, 0xcc]);
    assert!(ixs[0].noop_event.is_some());
}

#[test]
fn preserves_outer_then_inner_order_within_a_tx() {
    let outer = vec![0x01u8];
    let inner = vec![0x02u8];
    let tx = from_json(json!({
        "meta": {
            "err": null,
            "innerInstructions": [
                {
                    "index": 0,
                    "instructions": [
                        { "programIdIndex": 2, "accounts": [0, 1], "data": enc_bs58(&inner) }
                    ]
                }
            ]
        },
        "transaction": {
            "message": {
                "accountKeys": [ADDR_SYS, ADDR_RENT, BUBBLEGUM_PROGRAM_ID],
                "instructions": [
                    { "programIdIndex": 2, "accounts": [0, 1], "data": enc_bs58(&outer) }
                ]
            }
        }
    }));
    let ixs = extract_bubblegum_ixs(&tx);
    assert_eq!(ixs.len(), 2);
    assert_eq!(ixs[0].data, vec![0x01]);
    assert_eq!(ixs[1].data, vec![0x02]);
}

#[test]
fn resolves_loaded_addresses_for_versioned_txs() {
    let tx = from_json(json!({
        "meta": {
            "err": null,
            "innerInstructions": [],
            "loadedAddresses": {
                "writable": [ADDR_RENT],
                "readonly": [BUBBLEGUM_PROGRAM_ID]
            }
        },
        "transaction": {
            "message": {
                // Static keys (indices 0..=1)
                "accountKeys": [ADDR_SYS, OTHER_PROGRAM],
                "instructions": [
                    // programIdIndex 3 → readonly[0] = Bubblegum
                    // accounts [0, 2] → [ADDR_SYS, ADDR_RENT]
                    { "programIdIndex": 3, "accounts": [0, 2], "data": enc_bs58(&[1u8]) }
                ]
            }
        }
    }));
    let ixs = extract_bubblegum_ixs(&tx);
    assert_eq!(ixs.len(), 1);
    assert_eq!(ixs[0].accounts.len(), 2);
    assert_eq!(ixs[0].accounts[0], [0u8; 32]); // ADDR_SYS (zeros)
}

#[test]
fn empty_or_malformed_tx_returns_empty() {
    let empty: RpcTransactionResponse = serde_json::from_value(json!({})).unwrap();
    assert_eq!(extract_bubblegum_ixs(&empty).len(), 0);

    let no_tx: RpcTransactionResponse =
        serde_json::from_value(json!({ "meta": { "err": null } })).unwrap();
    assert_eq!(extract_bubblegum_ixs(&no_tx).len(), 0);
}
