//! Indexer integration test — drives the orchestrator against a fully
//! synthetic upstream (no network). Builds fake Bubblegum txs with
//! proper ix encoding + inner noop LeafSchemaEvents, feeds them
//! through index_tree, asserts resulting store state matches what the
//! event stream implies.

use std::sync::{Arc, Mutex};

use borsh::BorshSerialize;
use mpl_bubblegum::instructions::{
    BurnInstructionArgs, CreateTreeConfigInstructionArgs, MintV1InstructionArgs,
};
use mpl_bubblegum::types::{LeafSchema, MetadataArgs, TokenProgramVersion, Version};
use mpl_bubblegum::LeafSchemaEvent;
use serde_json::json;
use solana_program::pubkey::Pubkey;

use tidepool_rpc::cnft::{
    index_tree,
    parser::{BURN_DISC, CREATE_TREE_CONFIG_DISC, MINT_V1_DISC},
    CnftStore, IndexTreeOptions, MemoryCnftStore, BUBBLEGUM_PROGRAM_ID,
};
use tidepool_rpc::upstream::{FixtureUpstream, UpstreamError};

const SPL_NOOP: &str = "noopb9bkMVfRPU8AsbpTUg8AQkHtKwMYZiFUjNRtMmV";
const ADDR_SYS: &str = "11111111111111111111111111111111";
const ADDR_RENT: &str = "SysvarRent111111111111111111111111111111111";
const TREE_BYTES: [u8; 32] = [0x11; 32];

fn tree_b58() -> String {
    bs58::encode(TREE_BYTES).into_string()
}

fn enc<T: BorshSerialize>(v: &T) -> Vec<u8> {
    let mut out = Vec::new();
    v.serialize(&mut out).unwrap();
    out
}

fn bs58_str(bytes: &[u8]) -> String {
    bs58::encode(bytes).into_string()
}

fn stub_metadata() -> MetadataArgs {
    MetadataArgs {
        name: "X".into(),
        symbol: "X".into(),
        uri: "https://x".into(),
        seller_fee_basis_points: 0,
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

// Build a tx JSON for a single outer Bubblegum ix with the given
// discriminator + Borsh-serialized body + the given account list.
fn outer_bubblegum_tx(disc: [u8; 8], body: Vec<u8>, accounts: &[&str]) -> serde_json::Value {
    let mut data = disc.to_vec();
    data.extend(body);
    let account_keys: Vec<String> = accounts
        .iter()
        .chain(std::iter::once(&BUBBLEGUM_PROGRAM_ID))
        .map(|s| (*s).to_string())
        .collect();
    let bubblegum_index = account_keys.len() - 1;
    let ix_account_indices: Vec<usize> = (0..accounts.len()).collect();
    json!({
        "meta": { "err": null, "innerInstructions": [] },
        "transaction": {
            "message": {
                "accountKeys": account_keys,
                "instructions": [
                    {
                        "programIdIndex": bubblegum_index,
                        "accounts": ix_account_indices,
                        "data": bs58_str(&data),
                    }
                ]
            }
        }
    })
}

/// Tiny in-memory "chain" used to back FixtureUpstream's `getSignaturesForAddress`
/// and `getTransaction` methods.
#[derive(Default)]
struct FixtureChain {
    sigs: Vec<(String, serde_json::Value)>, // (signature, err)
    txs: std::collections::HashMap<String, serde_json::Value>,
}

impl FixtureChain {
    fn append(&mut self, signature: &str, err: serde_json::Value, tx: serde_json::Value) {
        self.sigs.push((signature.to_string(), err));
        self.txs.insert(signature.to_string(), tx);
    }
}

fn upstream_for_chain(chain: &Arc<Mutex<FixtureChain>>) -> FixtureUpstream {
    let sigs_chain = Arc::clone(chain);
    let txs_chain = Arc::clone(chain);
    FixtureUpstream::new()
        .with_method("getSignaturesForAddress", move |params| {
            let opts = params.get(1).cloned().unwrap_or_else(|| json!({}));
            let limit = opts
                .get("limit")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(1000) as usize;
            let before = opts
                .get("before")
                .and_then(|v| v.as_str())
                .map(String::from);
            let until = opts.get("until").and_then(|v| v.as_str()).map(String::from);

            let guard = sigs_chain.lock().unwrap();
            // RPC returns newest-first.
            let all: Vec<_> = guard.sigs.iter().rev().cloned().collect();
            let mut filtered = all;
            if let Some(b) = &before {
                if let Some(i) = filtered.iter().position(|(s, _)| s == b) {
                    filtered = filtered[i + 1..].to_vec();
                }
            }
            if let Some(u) = &until {
                if let Some(i) = filtered.iter().position(|(s, _)| s == u) {
                    filtered.truncate(i);
                }
            }
            filtered.truncate(limit);
            Ok(json!(filtered
                .into_iter()
                .map(|(sig, err)| json!({ "signature": sig, "slot": 1, "err": err }))
                .collect::<Vec<_>>()))
        })
        .with_method("getTransaction", move |params| {
            let sig = params
                .get(0)
                .and_then(|v| v.as_str())
                .ok_or_else(|| UpstreamError::Transport("missing sig".into()))?;
            let guard = txs_chain.lock().unwrap();
            Ok(guard
                .txs
                .get(sig)
                .cloned()
                .unwrap_or(serde_json::Value::Null))
        })
}

// ─── scenarios ──────────────────────────────────────────────────────

#[tokio::test]
async fn single_create_tree_tx_populates_store() {
    let chain = Arc::new(Mutex::new(FixtureChain::default()));
    let tx = outer_bubblegum_tx(
        CREATE_TREE_CONFIG_DISC,
        enc(&CreateTreeConfigInstructionArgs {
            max_depth: 10,
            max_buffer_size: 16,
            public: Some(false),
        }),
        // create_tree_config accounts: [treeConfig, merkleTree, payer,
        //   treeCreator, logWrapper, compressionProgram, systemProgram]
        &[
            ADDR_SYS,
            &tree_b58(),
            ADDR_SYS,
            ADDR_SYS,
            ADDR_SYS,
            ADDR_SYS,
            ADDR_SYS,
        ],
    );
    chain.lock().unwrap().append("sig-create", json!(null), tx);

    let upstream = upstream_for_chain(&chain);
    let store = MemoryCnftStore::new();

    let result = index_tree(&upstream, &store, TREE_BYTES, &IndexTreeOptions::default())
        .await
        .unwrap();

    assert_eq!(result.processed, 1);
    assert_eq!(result.applied, 1);
    let info = store.get_tree(&TREE_BYTES).await.unwrap().unwrap();
    assert_eq!(info.depth, 10);
}

#[tokio::test]
async fn create_plus_two_mints_apply_in_chronological_order() {
    let chain = Arc::new(Mutex::new(FixtureChain::default()));

    // create_tree
    chain.lock().unwrap().append(
        "sig-create",
        json!(null),
        outer_bubblegum_tx(
            CREATE_TREE_CONFIG_DISC,
            enc(&CreateTreeConfigInstructionArgs {
                max_depth: 10,
                max_buffer_size: 16,
                public: Some(false),
            }),
            &[
                ADDR_SYS,
                &tree_b58(),
                ADDR_SYS,
                ADDR_SYS,
                ADDR_SYS,
                ADDR_SYS,
                ADDR_SYS,
            ],
        ),
    );

    // mint_v1 × 2
    let mint_body = enc(&MintV1InstructionArgs {
        metadata: stub_metadata(),
    });
    for sig in ["sig-mint1", "sig-mint2"] {
        chain.lock().unwrap().append(
            sig,
            json!(null),
            outer_bubblegum_tx(
                MINT_V1_DISC,
                mint_body.clone(),
                // mint_v1 accounts: [treeConfig, leafOwner, leafDelegate,
                //   merkleTree, payer, treeDelegate, logWrapper,
                //   compressionProgram, systemProgram]
                &[
                    ADDR_SYS,
                    ADDR_SYS,
                    ADDR_SYS,
                    &tree_b58(),
                    ADDR_SYS,
                    ADDR_SYS,
                    ADDR_SYS,
                    ADDR_SYS,
                    ADDR_SYS,
                ],
            ),
        );
    }

    let upstream = upstream_for_chain(&chain);
    let store = MemoryCnftStore::new();
    let result = index_tree(&upstream, &store, TREE_BYTES, &IndexTreeOptions::default())
        .await
        .unwrap();

    assert_eq!(result.processed, 3);
    assert_eq!(result.applied, 3);
    let info = store.get_tree(&TREE_BYTES).await.unwrap().unwrap();
    assert_eq!(info.num_minted, 2);
    let leaves = store.list_leaves(&TREE_BYTES).await.unwrap();
    assert_eq!(leaves.len(), 2);
    assert_eq!(leaves[0].leaf_index, 0);
    assert_eq!(leaves[1].leaf_index, 1);
}

#[tokio::test]
async fn incremental_call_resumes_from_cursor() {
    let chain = Arc::new(Mutex::new(FixtureChain::default()));
    chain.lock().unwrap().append(
        "sig-create",
        json!(null),
        outer_bubblegum_tx(
            CREATE_TREE_CONFIG_DISC,
            enc(&CreateTreeConfigInstructionArgs {
                max_depth: 10,
                max_buffer_size: 16,
                public: Some(false),
            }),
            &[
                ADDR_SYS,
                &tree_b58(),
                ADDR_SYS,
                ADDR_SYS,
                ADDR_SYS,
                ADDR_SYS,
                ADDR_SYS,
            ],
        ),
    );
    chain.lock().unwrap().append(
        "sig-mint1",
        json!(null),
        outer_bubblegum_tx(
            MINT_V1_DISC,
            enc(&MintV1InstructionArgs {
                metadata: stub_metadata(),
            }),
            &[
                ADDR_SYS,
                ADDR_SYS,
                ADDR_SYS,
                &tree_b58(),
                ADDR_SYS,
                ADDR_SYS,
                ADDR_SYS,
                ADDR_SYS,
                ADDR_SYS,
            ],
        ),
    );

    let upstream = upstream_for_chain(&chain);
    let store = MemoryCnftStore::new();
    let first = index_tree(&upstream, &store, TREE_BYTES, &IndexTreeOptions::default())
        .await
        .unwrap();
    assert_eq!(first.processed, 2);
    assert_eq!(
        store
            .get_last_signature(&TREE_BYTES)
            .await
            .unwrap()
            .as_deref(),
        Some("sig-mint1")
    );

    // New mint arrives — next call should only process that one.
    chain.lock().unwrap().append(
        "sig-mint2",
        json!(null),
        outer_bubblegum_tx(
            MINT_V1_DISC,
            enc(&MintV1InstructionArgs {
                metadata: stub_metadata(),
            }),
            &[
                ADDR_SYS,
                ADDR_SYS,
                ADDR_SYS,
                &tree_b58(),
                ADDR_SYS,
                ADDR_SYS,
                ADDR_SYS,
                ADDR_SYS,
                ADDR_SYS,
            ],
        ),
    );
    let second = index_tree(&upstream, &store, TREE_BYTES, &IndexTreeOptions::default())
        .await
        .unwrap();
    assert_eq!(second.processed, 1);
    let info = store.get_tree(&TREE_BYTES).await.unwrap().unwrap();
    assert_eq!(info.num_minted, 2);
}

#[tokio::test]
async fn failed_tx_advances_cursor_without_applying_state() {
    let chain = Arc::new(Mutex::new(FixtureChain::default()));
    chain.lock().unwrap().append(
        "sig-create",
        json!(null),
        outer_bubblegum_tx(
            CREATE_TREE_CONFIG_DISC,
            enc(&CreateTreeConfigInstructionArgs {
                max_depth: 10,
                max_buffer_size: 16,
                public: Some(false),
            }),
            &[
                ADDR_SYS,
                &tree_b58(),
                ADDR_SYS,
                ADDR_SYS,
                ADDR_SYS,
                ADDR_SYS,
                ADDR_SYS,
            ],
        ),
    );
    // Failed tx with Bubblegum ix in it — should NOT apply.
    chain.lock().unwrap().append(
        "sig-failed",
        json!({ "InstructionError": [0, "Custom"] }),
        outer_bubblegum_tx(
            MINT_V1_DISC,
            enc(&MintV1InstructionArgs {
                metadata: stub_metadata(),
            }),
            &[
                ADDR_SYS,
                ADDR_SYS,
                ADDR_SYS,
                &tree_b58(),
                ADDR_SYS,
                ADDR_SYS,
                ADDR_SYS,
                ADDR_SYS,
                ADDR_SYS,
            ],
        ),
    );

    let upstream = upstream_for_chain(&chain);
    let store = MemoryCnftStore::new();
    let result = index_tree(&upstream, &store, TREE_BYTES, &IndexTreeOptions::default())
        .await
        .unwrap();

    assert_eq!(result.processed, 2);
    assert_eq!(
        result.applied, 1,
        "only createTree applies; failed mint doesn't"
    );
    let info = store.get_tree(&TREE_BYTES).await.unwrap().unwrap();
    assert_eq!(info.num_minted, 0, "no mint should have run");
    assert_eq!(
        store
            .get_last_signature(&TREE_BYTES)
            .await
            .unwrap()
            .as_deref(),
        Some("sig-failed")
    );
}

#[tokio::test]
async fn burn_of_indexed_leaf_marks_it_burned_end_to_end() {
    let chain = Arc::new(Mutex::new(FixtureChain::default()));
    let mint_v1_accounts = [
        ADDR_SYS,
        ADDR_SYS,
        ADDR_SYS,
        &tree_b58(),
        ADDR_SYS,
        ADDR_SYS,
        ADDR_SYS,
        ADDR_SYS,
        ADDR_SYS,
    ];
    let burn_accounts = [
        ADDR_SYS,
        ADDR_SYS,
        ADDR_SYS,
        &tree_b58(),
        ADDR_SYS,
        ADDR_SYS,
        ADDR_SYS,
    ];
    chain.lock().unwrap().append(
        "sig-create",
        json!(null),
        outer_bubblegum_tx(
            CREATE_TREE_CONFIG_DISC,
            enc(&CreateTreeConfigInstructionArgs {
                max_depth: 10,
                max_buffer_size: 16,
                public: Some(false),
            }),
            &[
                ADDR_SYS,
                &tree_b58(),
                ADDR_SYS,
                ADDR_SYS,
                ADDR_SYS,
                ADDR_SYS,
                ADDR_SYS,
            ],
        ),
    );
    chain.lock().unwrap().append(
        "sig-mint",
        json!(null),
        outer_bubblegum_tx(
            MINT_V1_DISC,
            enc(&MintV1InstructionArgs {
                metadata: stub_metadata(),
            }),
            &mint_v1_accounts,
        ),
    );
    chain.lock().unwrap().append(
        "sig-burn",
        json!(null),
        outer_bubblegum_tx(
            BURN_DISC,
            enc(&BurnInstructionArgs {
                root: [0; 32],
                data_hash: [0; 32],
                creator_hash: [0; 32],
                nonce: 0,
                index: 0,
            }),
            &burn_accounts,
        ),
    );

    let upstream = upstream_for_chain(&chain);
    let store = MemoryCnftStore::new();
    let result = index_tree(&upstream, &store, TREE_BYTES, &IndexTreeOptions::default())
        .await
        .unwrap();
    assert_eq!(result.applied, 3);

    let leaf = store
        .get_leaf_by_index(&TREE_BYTES, 0)
        .await
        .unwrap()
        .unwrap();
    assert!(leaf.burned);
}

#[tokio::test]
async fn empty_chain_is_a_noop() {
    let chain = Arc::new(Mutex::new(FixtureChain::default()));
    let upstream = upstream_for_chain(&chain);
    let store = MemoryCnftStore::new();
    let result = index_tree(&upstream, &store, TREE_BYTES, &IndexTreeOptions::default())
        .await
        .unwrap();
    assert_eq!(result.processed, 0);
    assert_eq!(result.applied, 0);
}

#[tokio::test]
#[allow(clippy::too_many_lines)] // end-to-end scenario; splitting obscures the flow
async fn noop_event_pairing_covers_verify_creator_flow() {
    // Build a tx that contains outer create_tree, then a second tx
    // that contains: outer mint_v1 — which our parser routes to Mint.
    // Then a THIRD tx containing verifyCreator with an inner noop
    // LeafSchemaEvent. Verify the noop-authoritative hashes flow
    // through apply into the store.
    let chain = Arc::new(Mutex::new(FixtureChain::default()));
    chain.lock().unwrap().append(
        "sig-create",
        json!(null),
        outer_bubblegum_tx(
            CREATE_TREE_CONFIG_DISC,
            enc(&CreateTreeConfigInstructionArgs {
                max_depth: 10,
                max_buffer_size: 16,
                public: Some(false),
            }),
            &[
                ADDR_SYS,
                &tree_b58(),
                ADDR_SYS,
                ADDR_SYS,
                ADDR_SYS,
                ADDR_SYS,
                ADDR_SYS,
            ],
        ),
    );

    // Mint: metadata with one creator so flip-target exists.
    let mut md = stub_metadata();
    md.creators = vec![mpl_bubblegum::types::Creator {
        address: Pubkey::new_from_array([0x44; 32]),
        verified: false,
        share: 100,
    }];
    chain.lock().unwrap().append(
        "sig-mint",
        json!(null),
        outer_bubblegum_tx(
            MINT_V1_DISC,
            enc(&MintV1InstructionArgs { metadata: md }),
            &[
                ADDR_SYS,
                ADDR_SYS,
                ADDR_SYS,
                &tree_b58(),
                ADDR_SYS,
                ADDR_SYS,
                ADDR_SYS,
                ADDR_SYS,
                ADDR_SYS,
            ],
        ),
    );

    // verifyCreator tx with inner noop LeafSchemaEvent. We embed the
    // event in the `innerInstructions` group for the outer ix.
    let event = LeafSchemaEvent::new(
        Version::V1,
        LeafSchema::V1 {
            id: Pubkey::new_from_array([0x99; 32]),
            owner: Pubkey::new_from_array([1; 32]),
            delegate: Pubkey::new_from_array([2; 32]),
            nonce: 0,
            data_hash: [0xee; 32],
            creator_hash: [0xef; 32],
        },
        [0xcc; 32],
    );
    let mut event_bytes = Vec::new();
    event.serialize(&mut event_bytes).unwrap();

    // verify_creator accounts (9): [treeConfig, leafOwner, leafDelegate,
    //   merkleTree, payer, creator, logWrapper, compressionProgram, systemProgram]
    let disc = tidepool_rpc::cnft::parser::VERIFY_CREATOR_DISC.to_vec();
    let verify_creator_tx = json!({
        "meta": {
            "err": null,
            "innerInstructions": [
                {
                    "index": 0,
                    "instructions": [
                        { "programIdIndex": 10, "accounts": [], "data": bs58_str(&event_bytes) }
                    ]
                }
            ]
        },
        "transaction": {
            "message": {
                "accountKeys": [
                    ADDR_SYS, ADDR_SYS, ADDR_SYS, tree_b58(), ADDR_SYS,
                    bs58_str(&[0x44u8; 32]), // creator — matches metadata creator
                    ADDR_SYS, ADDR_SYS, ADDR_RENT,
                    BUBBLEGUM_PROGRAM_ID,
                    SPL_NOOP,
                ],
                "instructions": [
                    {
                        "programIdIndex": 9,
                        "accounts": [0, 1, 2, 3, 4, 5, 6, 7, 8],
                        "data": bs58_str(&disc),
                    }
                ]
            }
        }
    });
    chain
        .lock()
        .unwrap()
        .append("sig-verify", json!(null), verify_creator_tx);

    let upstream = upstream_for_chain(&chain);
    let store = MemoryCnftStore::new();
    let result = index_tree(&upstream, &store, TREE_BYTES, &IndexTreeOptions::default())
        .await
        .unwrap();

    assert_eq!(result.applied, 3, "createTree + mint + verifyCreator");
    let leaves = store.list_leaves(&TREE_BYTES).await.unwrap();
    assert_eq!(leaves.len(), 1);
    assert_eq!(
        leaves[0].data_hash, [0xee; 32],
        "noop-authoritative dataHash"
    );
    assert_eq!(leaves[0].creator_hash, [0xef; 32]);
    assert!(
        leaves[0].mint_metadata.creators[0].verified,
        "creator flipped by verifyCreator"
    );
}
