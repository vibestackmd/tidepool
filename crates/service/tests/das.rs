//! DAS handler integration tests. Seed a cNFT into a store via
//! apply_event, then hit `get_asset` / `get_asset_proof` /
//! `get_asset_proof_batch` directly (service-layer fns, no HTTP).

use std::sync::Arc;

use tidepool_rpc::cache::{CacheStore, MemoryCache};
use tidepool_rpc::cnft::{
    apply::derive_asset_id, apply_event, CnftEvent, MemoryCnftStore, MintMetadata,
};
use tidepool_rpc::das::{
    get_asset, get_asset_proof, get_asset_proof_batch, get_balances, get_nft_editions,
    get_token_accounts, AccountDecoder, DasError, MasterEditionRecord, PrintEditionRecord,
    TokenAccountsFilter,
};
use tidepool_rpc::upstream::FixtureUpstream;
use tidepool_rpc::verify_proof;
use tidepool_core::Creator;

const TREE: [u8; 32] = [0x11; 32];

fn stub_mint_metadata() -> MintMetadata {
    MintMetadata {
        name: "Compressed".into(),
        symbol: "CMP".into(),
        uri: "https://example.com/cnft.json".into(),
        seller_fee_basis_points: 250,
        primary_sale_happened: false,
        is_mutable: true,
        creators: vec![Creator {
            address: [0x44; 32],
            verified: false,
            share: 100,
        }],
        collection: None,
        data_hash_input: br#"{"name":"Compressed"}"#.to_vec(),
    }
}

async fn seed(store: &MemoryCnftStore) -> [u8; 32] {
    apply_event(
        store,
        CnftEvent::CreateTree {
            tree: TREE,
            depth: 6,
            max_buffer_size: 8,
        },
    )
    .await
    .unwrap();
    apply_event(
        store,
        CnftEvent::Mint {
            tree: TREE,
            owner: [0x22; 32],
            delegate: [0x33; 32],
            metadata: stub_mint_metadata(),
            verify_collection: None,
            noop: None,
        },
    )
    .await
    .unwrap();
    derive_asset_id(&TREE, 0)
}

fn bs58_to_bytes(s: &str) -> Vec<u8> {
    bs58::decode(s).into_vec().unwrap()
}

#[tokio::test]
async fn get_asset_returns_cnft_shape_for_indexed_asset() {
    let store = MemoryCnftStore::new();
    let asset_id = seed(&store).await;

    let asset = get_asset(&store, &asset_id).await.unwrap().expect("Some");
    assert_eq!(asset.id, bs58::encode(asset_id).into_string());
    assert_eq!(asset.interface, "V1_NFT");

    let compression = asset.compression.as_ref().expect("compression present");
    assert!(compression.compressed);
    assert!(compression.eligible);
    assert_eq!(compression.tree, bs58::encode(TREE).into_string());
    assert_eq!(compression.leaf_id, 0);

    // owner (0x22) != delegate (0x33) → delegated=true
    assert!(asset.ownership.delegated);
}

#[tokio::test]
async fn get_asset_returns_none_for_unknown_id() {
    let store = MemoryCnftStore::new();
    let unknown = [0xee; 32];
    assert!(get_asset(&store, &unknown).await.unwrap().is_none());
}

#[tokio::test]
async fn get_asset_proof_round_trips_against_verify_proof() {
    let store = MemoryCnftStore::new();
    let asset_id = seed(&store).await;

    let proof = get_asset_proof(&store, &asset_id)
        .await
        .unwrap()
        .expect("Some");
    assert_eq!(proof.tree_id, bs58::encode(TREE).into_string());
    // depth 6 → node_index = 2^6 + leaf_index(0) = 64
    assert_eq!(proof.node_index, 64);
    assert_eq!(proof.proof.len(), 6);

    // Decode base58 and cross-verify.
    let leaf_bytes: [u8; 32] = bs58_to_bytes(&proof.leaf).try_into().unwrap();
    let root_bytes: [u8; 32] = bs58_to_bytes(&proof.root).try_into().unwrap();
    let proof_nodes: Vec<[u8; 32]> = proof
        .proof
        .iter()
        .map(|s| bs58_to_bytes(s).try_into().unwrap())
        .collect();
    assert!(verify_proof(&leaf_bytes, &proof_nodes, 0, &root_bytes));
}

#[tokio::test]
async fn get_asset_proof_returns_none_for_unknown_asset() {
    let store = MemoryCnftStore::new();
    let unknown = [0xcc; 32];
    assert!(get_asset_proof(&store, &unknown).await.unwrap().is_none());
}

#[tokio::test]
async fn get_asset_proof_batch_returns_ordered_nulls_for_misses() {
    let store = MemoryCnftStore::new();
    let known = seed(&store).await;
    let unknown = [0xaa; 32];

    let results = get_asset_proof_batch(&store, &[known, unknown, known])
        .await
        .unwrap();
    assert_eq!(results.len(), 3);
    assert!(results[0].is_some(), "known id → proof");
    assert!(results[1].is_none(), "unknown id → None");
    assert!(results[2].is_some(), "known id again → proof");
}

#[tokio::test]
async fn get_asset_proof_batch_shares_tree_state_across_asset_ids() {
    // Not asserting the sharing directly — there's no hook — but a
    // batch over multiple leaves in one tree should still produce
    // verifiable proofs for each. If the tree-state materialization
    // regresses we'll see it here.
    let store = MemoryCnftStore::new();
    apply_event(
        &store,
        CnftEvent::CreateTree {
            tree: TREE,
            depth: 5,
            max_buffer_size: 8,
        },
    )
    .await
    .unwrap();
    let mut asset_ids = Vec::new();
    for i in 0u8..4 {
        apply_event(
            &store,
            CnftEvent::Mint {
                tree: TREE,
                owner: [i; 32],
                delegate: [i; 32],
                metadata: stub_mint_metadata(),
                verify_collection: None,
                noop: None,
            },
        )
        .await
        .unwrap();
        asset_ids.push(derive_asset_id(&TREE, u64::from(i)));
    }

    let results = get_asset_proof_batch(&store, &asset_ids).await.unwrap();
    assert_eq!(results.len(), 4);
    for (i, proof) in results.iter().enumerate() {
        let proof = proof.as_ref().expect("Some");
        let leaf: [u8; 32] = bs58_to_bytes(&proof.leaf).try_into().unwrap();
        let root: [u8; 32] = bs58_to_bytes(&proof.root).try_into().unwrap();
        let nodes: Vec<[u8; 32]> = proof
            .proof
            .iter()
            .map(|s| bs58_to_bytes(s).try_into().unwrap())
            .collect();
        assert!(
            verify_proof(&leaf, &nodes, i as u64, &root),
            "proof {i} failed to verify"
        );
    }
}

// ─── getNftEditions ─────────────────────────────────────────────────

#[tokio::test]
async fn get_nft_editions_returns_none_for_unknown_master() {
    let cache = MemoryCache::new();
    // Upstream with no stubbed accounts — cold-path fetch of the
    // master mint yields nothing, so there's nothing to return.
    let upstream = FixtureUpstream::new();
    let decoders: Vec<Arc<dyn AccountDecoder>> = vec![];
    let got = get_nft_editions(&cache, &upstream, &decoders, "UNKNOWN_MINT", 1, 100)
        .await
        .unwrap();
    assert!(got.is_none());
}

#[tokio::test]
async fn get_nft_editions_paginates_indexed_prints() {
    let cache = MemoryCache::new();
    cache
        .put_master_edition(MasterEditionRecord {
            master_mint: "MASTER_MINT".into(),
            master_edition_pda: "MASTER_PDA".into(),
            supply: 5,
            max_supply: Some(100),
        })
        .await
        .unwrap();
    for (mint, num) in [
        ("PRINT1", 1u64),
        ("PRINT2", 2),
        ("PRINT3", 3),
        ("PRINT4", 4),
        ("PRINT5", 5),
    ] {
        cache
            .put_print_edition(PrintEditionRecord {
                print_mint: mint.into(),
                print_edition_pda: format!("{mint}_PDA"),
                parent_master_edition_pda: "MASTER_PDA".into(),
                edition_num: num,
            })
            .await
            .unwrap();
    }

    let upstream = FixtureUpstream::new();
    let decoders: Vec<Arc<dyn AccountDecoder>> = vec![];

    // Page 1, limit 2: the first two.
    let page1 = get_nft_editions(&cache, &upstream, &decoders, "MASTER_MINT", 1, 2)
        .await
        .unwrap()
        .expect("Some");
    assert_eq!(page1.total, 5);
    assert_eq!(page1.page, 1);
    assert_eq!(page1.limit, 2);
    assert_eq!(page1.master_edition_address, "MASTER_PDA");
    assert_eq!(page1.supply, 5);
    assert_eq!(page1.max_supply, Some(100));
    assert_eq!(page1.editions.len(), 2);
    assert_eq!(page1.editions[0].mint, "PRINT1");
    assert_eq!(page1.editions[0].edition, 1);
    assert_eq!(page1.editions[1].mint, "PRINT2");

    // Page 3 at limit 2: last odd entry.
    let page3 = get_nft_editions(&cache, &upstream, &decoders, "MASTER_MINT", 3, 2)
        .await
        .unwrap()
        .expect("Some");
    assert_eq!(page3.editions.len(), 1);
    assert_eq!(page3.editions[0].mint, "PRINT5");

    // Beyond end: empty editions, total unchanged.
    let overshoot = get_nft_editions(&cache, &upstream, &decoders, "MASTER_MINT", 99, 2)
        .await
        .unwrap()
        .expect("Some");
    assert!(overshoot.editions.is_empty());
    assert_eq!(overshoot.total, 5);
}

#[tokio::test]
async fn get_nft_editions_response_shape_round_trips_via_serde() {
    // Spot-check the wire-shape serialization. The pagination test
    // above exercises the logic; this test pins the JSON field names
    // clients see.
    let cache = MemoryCache::new();
    cache
        .put_master_edition(MasterEditionRecord {
            master_mint: "MASTER".into(),
            master_edition_pda: "MASTER_PDA".into(),
            supply: 7,
            max_supply: None,
        })
        .await
        .unwrap();
    cache
        .put_print_edition(PrintEditionRecord {
            print_mint: "P".into(),
            print_edition_pda: "P_PDA".into(),
            parent_master_edition_pda: "MASTER_PDA".into(),
            edition_num: 7,
        })
        .await
        .unwrap();

    let upstream = FixtureUpstream::new();
    let decoders: Vec<Arc<dyn AccountDecoder>> = vec![];
    let got = get_nft_editions(&cache, &upstream, &decoders, "MASTER", 1, 10)
        .await
        .unwrap()
        .expect("Some");
    let json = serde_json::to_value(&got).unwrap();
    assert_eq!(json["master_edition_address"], "MASTER_PDA");
    assert_eq!(json["supply"], 7);
    // Helius omits max_supply when None.
    assert!(json.get("max_supply").is_none());
    assert_eq!(json["total"], 1);
    assert_eq!(json["editions"][0]["mint"], "P");
    assert_eq!(json["editions"][0]["edition_address"], "P_PDA");
    assert_eq!(json["editions"][0]["edition"], 7);
}

// ─── getTokenAccounts ───────────────────────────────────────────────

fn token_account_json(
    address: &str,
    mint: &str,
    owner: &str,
    amount: u64,
    frozen: bool,
) -> serde_json::Value {
    serde_json::json!({
        "pubkey": address,
        "account": {
            "data": {
                "parsed": {
                    "type": "account",
                    "info": {
                        "mint": mint,
                        "owner": owner,
                        "state": if frozen { "frozen" } else { "initialized" },
                        "tokenAmount": {
                            "amount": amount.to_string(),
                            "decimals": 0,
                            "uiAmount": amount as f64,
                            "uiAmountString": amount.to_string(),
                        }
                    }
                },
                "program": "spl-token"
            }
        }
    })
}

#[tokio::test]
async fn get_token_accounts_rejects_no_filter() {
    let upstream = FixtureUpstream::new();
    let filter = TokenAccountsFilter {
        page: 1,
        limit: 100,
        ..Default::default()
    };
    let err = get_token_accounts(&upstream, &filter).await.unwrap_err();
    assert!(matches!(err, DasError::BadRequest(_)), "got {err:?}");
}

#[tokio::test]
async fn get_token_accounts_by_owner_paginates_and_hides_zero_balance() {
    // Upstream returns 3 accounts for the owner query: two with
    // non-zero balance, one zero. Token-2022 arm returns empty.
    let upstream = FixtureUpstream::new().with_method("getTokenAccountsByOwner", |params| {
        // Distinguish SPL vs Token-2022 by the programId filter —
        // return data for SPL only.
        let program = params
            .get(1)
            .and_then(|f| f.get("programId"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");
        let spl = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA";
        if program == spl {
            Ok(serde_json::json!({
                "context": { "slot": 1 },
                "value": [
                    token_account_json("ADDR_A", "MINT1", "OWNER", 5, false),
                    token_account_json("ADDR_B", "MINT2", "OWNER", 0, false),
                    token_account_json("ADDR_C", "MINT3", "OWNER", 3, false),
                ]
            }))
        } else {
            Ok(serde_json::json!({ "context": { "slot": 1 }, "value": [] }))
        }
    });

    let filter = TokenAccountsFilter {
        owner: Some("OWNER".into()),
        page: 1,
        limit: 100,
        show_zero_balance: false,
        ..Default::default()
    };
    let got = get_token_accounts(&upstream, &filter).await.unwrap();
    assert_eq!(got.total, 2, "zero-balance entry hidden");
    // Sorted by address: ADDR_A before ADDR_C.
    assert_eq!(got.token_accounts[0].address, "ADDR_A");
    assert_eq!(got.token_accounts[0].amount, 5);
    assert_eq!(got.token_accounts[1].address, "ADDR_C");

    // show_zero_balance=true includes the zero-balance entry.
    let filter_all = TokenAccountsFilter {
        show_zero_balance: true,
        ..filter.clone()
    };
    let all = get_token_accounts(&upstream, &filter_all).await.unwrap();
    assert_eq!(all.total, 3);
}

#[tokio::test]
async fn get_token_accounts_by_mint_uses_program_accounts() {
    let upstream = FixtureUpstream::new().with_method("getProgramAccounts", |params| {
        // Confirm the memcmp filter on offset 0 is present.
        let filters = params
            .get(1)
            .and_then(|o| o.get("filters"))
            .cloned()
            .unwrap_or(serde_json::Value::Null);
        assert!(filters.to_string().contains("\"offset\":0"));
        // Return under SPL only.
        let program = params
            .get(0)
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");
        if program == "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA" {
            Ok(serde_json::json!([
                token_account_json("ADDR1", "TARGET_MINT", "OWNER1", 1, false),
                token_account_json("ADDR2", "TARGET_MINT", "OWNER2", 2, true),
            ]))
        } else {
            Ok(serde_json::json!([]))
        }
    });
    let filter = TokenAccountsFilter {
        mint: Some("TARGET_MINT".into()),
        page: 1,
        limit: 100,
        ..Default::default()
    };
    let got = get_token_accounts(&upstream, &filter).await.unwrap();
    assert_eq!(got.total, 2);
    let frozen: Vec<_> = got
        .token_accounts
        .iter()
        .filter(|a| a.frozen)
        .map(|a| a.address.clone())
        .collect();
    assert_eq!(frozen, vec!["ADDR2".to_string()]);
}

#[tokio::test]
async fn get_token_accounts_paginates_with_page_limit() {
    let upstream = FixtureUpstream::new().with_method("getTokenAccountsByOwner", |params| {
        let program = params
            .get(1)
            .and_then(|f| f.get("programId"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");
        if program == "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA" {
            Ok(serde_json::json!({
                "context": { "slot": 1 },
                "value": [
                    token_account_json("ADDR_A", "M", "O", 1, false),
                    token_account_json("ADDR_B", "M", "O", 1, false),
                    token_account_json("ADDR_C", "M", "O", 1, false),
                    token_account_json("ADDR_D", "M", "O", 1, false),
                ]
            }))
        } else {
            Ok(serde_json::json!({ "context": { "slot": 1 }, "value": [] }))
        }
    });
    let base = TokenAccountsFilter {
        owner: Some("O".into()),
        page: 1,
        limit: 2,
        show_zero_balance: false,
        ..Default::default()
    };
    let p1 = get_token_accounts(&upstream, &base).await.unwrap();
    assert_eq!(p1.total, 4);
    assert_eq!(p1.token_accounts.len(), 2);
    assert_eq!(p1.token_accounts[0].address, "ADDR_A");

    let p2 = get_token_accounts(
        &upstream,
        &TokenAccountsFilter {
            page: 2,
            ..base.clone()
        },
    )
    .await
    .unwrap();
    assert_eq!(p2.token_accounts[0].address, "ADDR_C");

    let past = get_token_accounts(
        &upstream,
        &TokenAccountsFilter {
            page: 99,
            ..base.clone()
        },
    )
    .await
    .unwrap();
    assert!(past.token_accounts.is_empty());
    assert_eq!(past.total, 4);
}

// ─── getBalances (Wallet API) ───────────────────────────────────────

#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn get_balances_combines_native_plus_tokens_and_hides_zero() {
    let upstream = FixtureUpstream::new()
        .with_method("getBalance", |_params| {
            Ok(serde_json::json!({
                "context": { "slot": 1 },
                "value": 42_000_000u64
            }))
        })
        .with_method("getTokenAccountsByOwner", |params| {
            let program = params
                .get(1)
                .and_then(|f| f.get("programId"))
                .and_then(serde_json::Value::as_str)
                .unwrap_or("");
            // SPL side returns two positions (one zero-balance, should hide).
            if program == "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA" {
                Ok(serde_json::json!({
                    "context": { "slot": 1 },
                    "value": [
                        {
                            "pubkey": "ATA_USDC",
                            "account": {
                                "data": {
                                    "parsed": {
                                        "type": "account",
                                        "info": {
                                            "mint": "USDCmint",
                                            "owner": "WALLET",
                                            "state": "initialized",
                                            "tokenAmount": {
                                                "amount": "1000000",
                                                "decimals": 6,
                                                "uiAmount": 1.0,
                                                "uiAmountString": "1"
                                            }
                                        }
                                    },
                                    "program": "spl-token"
                                }
                            }
                        },
                        {
                            "pubkey": "ATA_EMPTY",
                            "account": {
                                "data": {
                                    "parsed": {
                                        "type": "account",
                                        "info": {
                                            "mint": "OtherMint",
                                            "owner": "WALLET",
                                            "state": "initialized",
                                            "tokenAmount": {
                                                "amount": "0",
                                                "decimals": 0,
                                                "uiAmount": 0.0,
                                                "uiAmountString": "0"
                                            }
                                        }
                                    },
                                    "program": "spl-token"
                                }
                            }
                        }
                    ]
                }))
            } else {
                // Token-2022 returns one more position.
                Ok(serde_json::json!({
                    "context": { "slot": 1 },
                    "value": [{
                        "pubkey": "T22_ACCT",
                        "account": {
                            "data": {
                                "parsed": {
                                    "type": "account",
                                    "info": {
                                        "mint": "NewMint22",
                                        "owner": "WALLET",
                                        "state": "initialized",
                                        "tokenAmount": {
                                            "amount": "500",
                                            "decimals": 2,
                                            "uiAmount": 5.0,
                                            "uiAmountString": "5"
                                        }
                                    }
                                },
                                "program": "spl-token-2022"
                            }
                        }
                    }]
                }))
            }
        });

    let got = get_balances(&upstream, "WALLET").await.unwrap();
    assert_eq!(got.native_balance, 42_000_000);
    assert_eq!(got.tokens.len(), 2, "zero-balance ATA hidden");

    // Sorted by mint: NewMint22 before USDCmint.
    assert_eq!(got.tokens[0].mint, "NewMint22");
    assert_eq!(got.tokens[0].amount, 500);
    assert_eq!(got.tokens[0].decimals, 2);
    assert_eq!(got.tokens[0].token_account, "T22_ACCT");

    assert_eq!(got.tokens[1].mint, "USDCmint");
    assert_eq!(got.tokens[1].amount, 1_000_000);
    assert_eq!(got.tokens[1].decimals, 6);
}

#[tokio::test]
async fn get_balances_handles_wallet_with_no_tokens() {
    let upstream = FixtureUpstream::new()
        .with_method("getBalance", |_| {
            Ok(serde_json::json!({ "context": { "slot": 1 }, "value": 0u64 }))
        })
        .with_method("getTokenAccountsByOwner", |_| {
            Ok(serde_json::json!({ "context": { "slot": 1 }, "value": [] }))
        });
    let got = get_balances(&upstream, "EMPTY_WALLET").await.unwrap();
    assert_eq!(got.native_balance, 0);
    assert!(got.tokens.is_empty());
}

#[tokio::test]
async fn get_balances_response_shape_matches_helius_wire() {
    let upstream = FixtureUpstream::new()
        .with_method("getBalance", |_| {
            Ok(serde_json::json!({ "context": { "slot": 1 }, "value": 100u64 }))
        })
        .with_method("getTokenAccountsByOwner", |_| {
            Ok(serde_json::json!({ "context": { "slot": 1 }, "value": [] }))
        });
    let got = get_balances(&upstream, "W").await.unwrap();
    let json = serde_json::to_value(&got).unwrap();
    // Helius's REST wire shape: `{ tokens: [...], nativeBalance: <u64> }`.
    assert_eq!(json["nativeBalance"], 100);
    assert!(json["tokens"].is_array());
}
