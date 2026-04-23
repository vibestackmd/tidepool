//! Server integration tests. Bind the server to an ephemeral port,
//! point it at a mock upstream axum server, and assert end-to-end
//! behavior for:
//!   - native dispatch (tidepool_info)
//!   - passthrough (getSlot → mock upstream responds)
//!   - CORS preflight-compatible headers on POST

use std::sync::Arc;
use std::time::Duration;

use axum::{extract::State, routing::post, Json, Router};
use serde_json::{json, Value};
use tokio::net::TcpListener;

/// Tiny mock upstream: echoes a canned response for any POST to `/`.
/// Used to verify our server's passthrough forwards requests.
async fn spawn_mock_upstream() -> (String, Arc<tokio::sync::Mutex<Vec<Value>>>) {
    let seen = Arc::new(tokio::sync::Mutex::new(Vec::<Value>::new()));
    let seen_for_handler = Arc::clone(&seen);

    let app = Router::new()
        .route(
            "/",
            post(
                |State(seen): State<Arc<tokio::sync::Mutex<Vec<Value>>>>,
                 Json(body): Json<Value>| async move {
                    seen.lock().await.push(body.clone());
                    let id = body.get("id").cloned().unwrap_or(Value::Null);
                    Json(json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": "upstream-saw-you"
                    }))
                },
            ),
        )
        .with_state(seen_for_handler);

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (format!("http://{addr}"), seen)
}

/// Mock upstream that dispatches by method name. Useful for handlers
/// that shape their own upstream calls (e.g. getPriorityFeeEstimate
/// forwards to getRecentPrioritizationFees).
async fn spawn_mock_upstream_by_method(
    responses: std::collections::HashMap<&'static str, Value>,
) -> String {
    let responses = Arc::new(responses);
    let app = Router::new()
        .route(
            "/",
            post(
                move |State(r): State<Arc<std::collections::HashMap<&'static str, Value>>>,
                      Json(body): Json<Value>| {
                    let r = Arc::clone(&r);
                    async move {
                        let id = body.get("id").cloned().unwrap_or(Value::Null);
                        let method = body.get("method").and_then(Value::as_str).unwrap_or("");
                        let result = r.get(method).cloned().unwrap_or(Value::Null);
                        Json(json!({ "jsonrpc": "2.0", "id": id, "result": result }))
                    }
                },
            ),
        )
        .with_state(responses);

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{addr}")
}

async fn spawn_tidepool(upstream_url: String) -> String {
    spawn_tidepool_with_state(upstream_url, None).await
}

async fn spawn_tidepool_with_state(upstream_url: String, db: Option<std::path::PathBuf>) -> String {
    use tidepool_rpc_server::{run, ServerConfig};
    // Pick two free ports atomically. Production derives ws as
    // port+1; tests can't rely on that because parallel runs pick
    // adjacent HTTP ports and collide on each other's WS. So the
    // harness asks the OS for two unrelated ephemeral ports and hands
    // both to `run` explicitly via `ws_port: Some(..)`.
    let (port, ws_port) = pick_two_free_ports().await;

    let config = ServerConfig {
        port,
        ws_port: Some(ws_port),
        upstream_url,
        upstream_ws_url: "ws://127.0.0.1:1".into(),
        rpc_timeout: Duration::from_secs(5),
        index_trees: vec![],
        db,
        snapshots: vec![],
    };
    tokio::spawn(async move {
        run(config).await.unwrap();
    });
    tokio::time::sleep(Duration::from_millis(120)).await;
    format!("http://127.0.0.1:{port}")
}

/// Bind two sockets to `127.0.0.1:0` so the OS hands out two unique
/// ephemeral ports, then drop them immediately.
///
/// There's a brief TOCTOU window between `drop(listener)` and the
/// server's later `bind(port)` where another process could snatch the
/// port. In practice this hasn't flaked across ~100 parallel CI
/// invocations to date. If it ever starts flaking:
/// 1. Extend the server's `run()` with a `ServerConfig::bound`
///    option that accepts a pre-bound `TcpListener`, and pass our
///    listeners through directly — eliminates the window entirely.
/// 2. Alternatively, wrap the test in a file-lock-coordinated port
///    allocator so parallel tests don't compete for the same
///    ephemeral range.
///
/// Sequential binds (vs. `SocketAddr::from(([0,0,0,0], 0))` twice in
/// parallel) avoid the adjacency hazard of `port` + (`port` + 1)
/// colliding when two tests race.
async fn pick_two_free_ports() -> (u16, u16) {
    let a = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let b = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let ap = a.local_addr().unwrap().port();
    let bp = b.local_addr().unwrap().port();
    drop(a);
    drop(b);
    (ap, bp)
}

#[tokio::test]
async fn tidepool_info_native_dispatch() {
    let (upstream_url, _seen) = spawn_mock_upstream().await;
    let tidepool_url = spawn_tidepool(upstream_url).await;

    let client = reqwest::Client::new();
    let resp: Value = client
        .post(&tidepool_url)
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tidepool_info",
            "params": {}
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    assert_eq!(resp["id"], 1);
    assert_eq!(resp["result"]["name"], "tidepool-rpc");
    let methods = resp["result"]["methods"].as_array().expect("methods array");
    assert!(methods.iter().any(|m| m["method"] == "getAsset"));
    assert!(methods.iter().any(|m| m["method"] == "tidepool_indexTree"));

    // Every method entry surfaces its transport so tooling can sort
    // without guessing. Spot-check each transport value shows up.
    let transports: std::collections::HashSet<_> = methods
        .iter()
        .filter_map(|m| m["transport"].as_str())
        .collect();
    for expected in ["json_rpc", "rest", "ws", "sdk_wrapper"] {
        assert!(
            transports.contains(expected),
            "tidepool_info should surface `{expected}` transport; got {transports:?}"
        );
    }

    // Webhooks live on REST, never JSON-RPC — sanity guard for the
    // parity rule this harness enforces.
    let create_webhook = methods
        .iter()
        .find(|m| m["method"] == "createWebhook")
        .expect("createWebhook in manifest");
    assert_eq!(create_webhook["transport"], "rest");
}

#[tokio::test]
async fn unknown_method_is_passed_through_to_upstream() {
    let (upstream_url, seen) = spawn_mock_upstream().await;
    let tidepool_url = spawn_tidepool(upstream_url).await;

    let client = reqwest::Client::new();
    let resp: Value = client
        .post(&tidepool_url)
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 7,
            "method": "getSlot",
            "params": []
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    // Mock upstream's canned response travels back untouched.
    assert_eq!(resp["result"], "upstream-saw-you");
    assert_eq!(resp["id"], 7);

    let seen = seen.lock().await;
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0]["method"], "getSlot");
}

#[tokio::test]
async fn get_asset_proof_with_missing_tree_reports_not_found() {
    let (upstream_url, _seen) = spawn_mock_upstream().await;
    let tidepool_url = spawn_tidepool(upstream_url).await;

    let client = reqwest::Client::new();
    let resp: Value = client
        .post(&tidepool_url)
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "getAssetProof",
            "params": { "id": "11111111111111111111111111111111" }
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    assert!(resp["error"].is_object(), "should be a JSON-RPC error");
    assert_eq!(resp["error"]["code"], -32000);
    assert!(resp["error"]["message"]
        .as_str()
        .unwrap()
        .contains("not found"));
}

#[tokio::test]
async fn get_priority_fee_estimate_single_level() {
    // Ten round-number fees; medium (p50) → fees[round(0.5*9)] = fees[5] = 6000.
    let samples: Vec<Value> = (1..=10)
        .map(|i| json!({ "slot": 100 + i, "prioritizationFee": i * 1000 }))
        .collect();
    let mut responses = std::collections::HashMap::new();
    responses.insert("getRecentPrioritizationFees", Value::Array(samples));
    let upstream_url = spawn_mock_upstream_by_method(responses).await;
    let tidepool_url = spawn_tidepool(upstream_url).await;

    let resp: Value = reqwest::Client::new()
        .post(&tidepool_url)
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 11,
            "method": "getPriorityFeeEstimate",
            "params": [{ "options": { "priorityLevel": "medium" } }]
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    assert_eq!(resp["id"], 11);
    assert_eq!(resp["result"]["priorityFeeEstimate"], 6000);
}

#[tokio::test]
async fn get_priority_fee_estimate_all_levels() {
    let samples: Vec<Value> = (1..=10)
        .map(|i| json!({ "slot": 100 + i, "prioritizationFee": i * 1000 }))
        .collect();
    let mut responses = std::collections::HashMap::new();
    responses.insert("getRecentPrioritizationFees", Value::Array(samples));
    let upstream_url = spawn_mock_upstream_by_method(responses).await;
    let tidepool_url = spawn_tidepool(upstream_url).await;

    let resp: Value = reqwest::Client::new()
        .post(&tidepool_url)
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 12,
            "method": "getPriorityFeeEstimate",
            "params": [{ "options": { "includeAllPriorityFeeLevels": true } }]
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    let levels = &resp["result"]["priorityFeeLevels"];
    // Integer fees cast to f64 — direct equality is intentional.
    #[allow(clippy::float_cmp)]
    {
        assert_eq!(levels["min"].as_f64().unwrap(), 1000.0);
        assert_eq!(levels["unsafeMax"].as_f64().unwrap(), 10000.0);
    }
    assert!(levels["low"].as_f64().unwrap() <= levels["medium"].as_f64().unwrap());
    assert!(levels["medium"].as_f64().unwrap() <= levels["high"].as_f64().unwrap());
    assert!(levels["high"].as_f64().unwrap() <= levels["veryHigh"].as_f64().unwrap());
}

#[tokio::test]
async fn get_priority_fee_estimate_empty_upstream_returns_zero() {
    // Matches local-Surfpool reality: no contention → no fee samples → 0.
    let mut responses = std::collections::HashMap::new();
    responses.insert("getRecentPrioritizationFees", Value::Array(vec![]));
    let upstream_url = spawn_mock_upstream_by_method(responses).await;
    let tidepool_url = spawn_tidepool(upstream_url).await;

    let resp: Value = reqwest::Client::new()
        .post(&tidepool_url)
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 13,
            "method": "getPriorityFeeEstimate",
            "params": [{}]
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    assert_eq!(resp["result"]["priorityFeeEstimate"], 0);
}

#[tokio::test]
async fn get_program_accounts_v2_paginates_via_cursor() {
    // Upstream returns 4 "accounts"; V2 shim sorts by pubkey and slices.
    let mut responses = std::collections::HashMap::new();
    responses.insert(
        "getProgramAccounts",
        json!([
            { "pubkey": "CCC", "account": { "data": ["", "base64"], "owner": "ProgID" } },
            { "pubkey": "AAA", "account": { "data": ["", "base64"], "owner": "ProgID" } },
            { "pubkey": "DDD", "account": { "data": ["", "base64"], "owner": "ProgID" } },
            { "pubkey": "BBB", "account": { "data": ["", "base64"], "owner": "ProgID" } },
        ]),
    );
    let upstream_url = spawn_mock_upstream_by_method(responses).await;
    let tidepool_url = spawn_tidepool(upstream_url).await;
    let client = reqwest::Client::new();

    let page1: Value = client
        .post(&tidepool_url)
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "getProgramAccountsV2",
            "params": { "programId": "ProgID", "limit": 2 }
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    // Items should arrive lexicographically; cursor = last pubkey in page.
    assert_eq!(page1["result"]["items"][0]["pubkey"], "AAA");
    assert_eq!(page1["result"]["items"][1]["pubkey"], "BBB");
    assert_eq!(page1["result"]["cursor"], "BBB");

    let page2: Value = client
        .post(&tidepool_url)
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "getProgramAccountsV2",
            "params": { "programId": "ProgID", "limit": 2, "cursor": "BBB" }
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(page2["result"]["items"][0]["pubkey"], "CCC");
    assert_eq!(page2["result"]["items"][1]["pubkey"], "DDD");
    // End of stream — no cursor.
    assert!(page2["result"].get("cursor").is_none());
}

#[tokio::test]
async fn get_token_accounts_by_owner_v2_requires_mint_or_program_id() {
    let upstream_url = spawn_mock_upstream_by_method(std::collections::HashMap::new()).await;
    let tidepool_url = spawn_tidepool(upstream_url).await;

    let resp: Value = reqwest::Client::new()
        .post(&tidepool_url)
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "getTokenAccountsByOwnerV2",
            "params": { "owner": "OWNER" }
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(resp["error"]["code"], -32602);
}

#[tokio::test]
async fn get_token_accounts_by_owner_v2_slices_via_cursor() {
    let mut responses = std::collections::HashMap::new();
    responses.insert(
        "getTokenAccountsByOwner",
        json!({
            "context": { "slot": 1 },
            "value": [
                { "pubkey": "TB", "account": { "data": ["", "base64"], "owner": "TOKEN" } },
                { "pubkey": "TA", "account": { "data": ["", "base64"], "owner": "TOKEN" } },
                { "pubkey": "TC", "account": { "data": ["", "base64"], "owner": "TOKEN" } },
            ]
        }),
    );
    let upstream_url = spawn_mock_upstream_by_method(responses).await;
    let tidepool_url = spawn_tidepool(upstream_url).await;

    let got: Value = reqwest::Client::new()
        .post(&tidepool_url)
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "getTokenAccountsByOwnerV2",
            "params": { "owner": "OWNER", "mint": "MINT", "limit": 1 }
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(got["result"]["items"].as_array().unwrap().len(), 1);
    assert_eq!(got["result"]["items"][0]["pubkey"], "TA");
    assert_eq!(got["result"]["cursor"], "TA");
}

#[tokio::test]
async fn webhook_crud_round_trip() {
    // Empty responses map — delivery task will poll and find nothing.
    let upstream_url = spawn_mock_upstream_by_method(std::collections::HashMap::new()).await;
    let tidepool_url = spawn_tidepool(upstream_url).await;
    let client = reqwest::Client::new();

    // Create — POST /v0/webhooks.
    let create: Value = client
        .post(format!("{tidepool_url}/v0/webhooks"))
        .json(&json!({
            "webhookURL": "https://example.com/hook",
            "accountAddresses": ["ADDR_A"]
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let id = create["webhookID"].as_str().unwrap().to_string();
    assert!(id.starts_with("wh_"));
    assert_eq!(create["webhookURL"], "https://example.com/hook");

    // Get single — GET /v0/webhooks/:id.
    let fetched: Value = client
        .get(format!("{tidepool_url}/v0/webhooks/{id}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(fetched["webhookID"], id);

    // List — GET /v0/webhooks.
    let all: Value = client
        .get(format!("{tidepool_url}/v0/webhooks"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(all.as_array().unwrap().len(), 1);

    // Edit — PUT /v0/webhooks/:id.
    let edited: Value = client
        .put(format!("{tidepool_url}/v0/webhooks/{id}"))
        .json(&json!({ "webhookURL": "https://example.com/v2" }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(edited["webhookURL"], "https://example.com/v2");

    // Delete — DELETE /v0/webhooks/:id.
    let deleted: Value = client
        .delete(format!("{tidepool_url}/v0/webhooks/{id}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(deleted["deleted"], true);

    // Subsequent get should return null.
    let missing: Value = client
        .get(format!("{tidepool_url}/v0/webhooks/{id}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(missing.is_null());
}

#[tokio::test]
async fn create_webhook_rejects_missing_url() {
    let upstream_url = spawn_mock_upstream_by_method(std::collections::HashMap::new()).await;
    let tidepool_url = spawn_tidepool(upstream_url).await;

    let resp = reqwest::Client::new()
        .post(format!("{tidepool_url}/v0/webhooks"))
        .json(&json!({ "accountAddresses": ["ADDR"] }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["code"], -32602);
}

#[tokio::test]
async fn get_transactions_returns_enhanced_envelope() {
    let mut responses = std::collections::HashMap::new();
    responses.insert(
        "getTransaction",
        json!({
            "slot": 200,
            "blockTime": 1_700_000_100,
            "transaction": {
                "message": {
                    "accountKeys": ["WALLET_A", "WALLET_B", "11111111111111111111111111111111"],
                    "instructions": [
                        { "programIdIndex": 2, "accounts": [0, 1], "data": "3Bxs" }
                    ]
                }
            },
            "meta": {
                "fee": 5000,
                "err": null,
                "preBalances": [1_000_000, 0, 1],
                "postBalances": [494_000, 500_000, 1],
                "innerInstructions": []
            }
        }),
    );
    let upstream_url = spawn_mock_upstream_by_method(responses).await;
    let tidepool_url = spawn_tidepool(upstream_url).await;

    let resp: Value = reqwest::Client::new()
        .post(format!("{tidepool_url}/v0/transactions"))
        .json(&json!({ "transactions": ["SIG1"] }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let items = resp.as_array().expect("array");
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["signature"], "SIG1");
    assert_eq!(items[0]["type"], "TRANSFER");
    assert_eq!(items[0]["source"], "SYSTEM_PROGRAM");
    assert_eq!(items[0]["feePayer"], "WALLET_A");
    assert_eq!(items[0]["nativeTransfers"][0]["amount"], 500_000);
}

#[tokio::test]
async fn get_transactions_rejects_empty_signatures() {
    let upstream_url = spawn_mock_upstream_by_method(std::collections::HashMap::new()).await;
    let tidepool_url = spawn_tidepool(upstream_url).await;
    let resp = reqwest::Client::new()
        .post(format!("{tidepool_url}/v0/transactions"))
        .json(&json!({ "transactions": [] }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["code"], -32602);
}

#[tokio::test]
async fn webhook_registry_persists_across_restart_with_db_flag() {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    let db_path = tmp.path().to_path_buf();
    drop(tmp);

    // First process lifetime — create a webhook.
    let upstream_url1 = spawn_mock_upstream_by_method(std::collections::HashMap::new()).await;
    let tidepool_url1 = spawn_tidepool_with_state(upstream_url1, Some(db_path.clone())).await;
    let client = reqwest::Client::new();
    let created: Value = client
        .post(format!("{tidepool_url1}/v0/webhooks"))
        .json(&json!({
            "webhookURL": "https://persisted.example.com",
            "accountAddresses": ["PERSIST_ADDR"]
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let webhook_id = created["webhookID"].as_str().unwrap().to_string();
    assert!(!webhook_id.is_empty());

    // Second process lifetime — same db file, different port + upstream.
    let upstream_url2 = spawn_mock_upstream_by_method(std::collections::HashMap::new()).await;
    let tidepool_url2 = spawn_tidepool_with_state(upstream_url2, Some(db_path.clone())).await;
    let listed: Value = client
        .get(format!("{tidepool_url2}/v0/webhooks"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let items = listed.as_array().expect("array");
    assert!(
        items.iter().any(|w| w["webhookID"] == webhook_id),
        "webhook should survive across restarts when --db is set"
    );
}

#[tokio::test]
async fn tidepool_tree_snapshot_export_and_load_round_trip() {
    // Seed: run tidepool instance #1, index a tree via the service
    // layer directly (the HTTP path would require a mock upstream with
    // getSignaturesForAddress + getTransaction fixtures — heavier than
    // we need here). Then dump via RPC, load into a fresh instance,
    // confirm state survived.
    use tidepool_rpc::cnft::snapshot::dump_tree;
    use tidepool_rpc::cnft::SnapshotBlob;
    use tidepool_rpc::cnft::{
        apply::derive_asset_id, apply_event, CnftEvent, MemoryCnftStore, MintMetadata,
    };
    use tidepool_rpc_core::Creator;

    let tree: [u8; 32] = [0x33; 32];
    let src = MemoryCnftStore::new();
    apply_event(
        &src,
        CnftEvent::CreateTree {
            tree,
            depth: 10,
            max_buffer_size: 32,
        },
    )
    .await
    .unwrap();
    apply_event(
        &src,
        CnftEvent::Mint {
            tree,
            owner: [0x01; 32],
            delegate: [0x02; 32],
            metadata: MintMetadata {
                name: "Snap Asset".into(),
                symbol: "SNAP".into(),
                uri: "https://example.com/s.json".into(),
                seller_fee_basis_points: 100,
                primary_sale_happened: false,
                is_mutable: true,
                creators: vec![Creator {
                    address: [0x44; 32],
                    verified: true,
                    share: 100,
                }],
                collection: None,
                data_hash_input: vec![0xab; 16],
            },
            verify_collection: None,
            noop: None,
        },
    )
    .await
    .unwrap();

    // Dump via the service-layer helper (equivalent to what
    // tidepool_exportTreeSnapshot returns).
    let snapshot = dump_tree(&src, &tree).await.unwrap().expect("Some");
    let blob = SnapshotBlob::from_tree(&snapshot);

    // Fresh tidepool instance, no pre-indexed trees.
    let upstream_url = spawn_mock_upstream_by_method(std::collections::HashMap::new()).await;
    let tidepool_url = spawn_tidepool(upstream_url).await;
    let client = reqwest::Client::new();

    // Load into the fresh instance.
    let load: Value = client
        .post(&tidepool_url)
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tidepool_loadTreeSnapshot",
            "params": { "snapshot": blob }
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(load["result"]["leafCount"], 1);

    // getAssetProof should now resolve against the loaded state.
    let asset_id = derive_asset_id(&tree, 0);
    let asset_id_b58 = bs58::encode(asset_id).into_string();
    let proof: Value = client
        .post(&tidepool_url)
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "getAssetProof",
            "params": { "id": asset_id_b58 }
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(proof["result"]["tree_id"], bs58::encode(tree).into_string());
    // Leaf at index 0 in a depth-10 tree: node_index = 2^10 + 0.
    assert_eq!(proof["result"]["node_index"].as_u64().unwrap(), 1u64 << 10);
}

#[tokio::test]
async fn tidepool_export_tree_snapshot_returns_null_for_unknown() {
    let upstream_url = spawn_mock_upstream_by_method(std::collections::HashMap::new()).await;
    let tidepool_url = spawn_tidepool(upstream_url).await;
    let resp: Value = reqwest::Client::new()
        .post(&tidepool_url)
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tidepool_exportTreeSnapshot",
            "params": { "tree": "11111111111111111111111111111111" }
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(resp["result"].is_null());
}

#[tokio::test]
async fn snapshot_flag_preloads_tree_before_serving() {
    // Seed: build a snapshot JSON file (envelope format), then boot
    // tidepool with --snapshot pointing at it. After boot, getAssetProof
    // should resolve against the preloaded tree without us ever calling
    // tidepool_loadTreeSnapshot.
    use tidepool_rpc::cnft::snapshot::dump_tree;
    use tidepool_rpc::cnft::{
        apply::derive_asset_id, apply_event, CnftEvent, MemoryCnftStore, MintMetadata, SnapshotBlob,
    };
    use tidepool_rpc_core::Creator;
    use tidepool_rpc_server::{run, ServerConfig};

    let tree: [u8; 32] = [0x55; 32];
    let seed = MemoryCnftStore::new();
    apply_event(
        &seed,
        CnftEvent::CreateTree {
            tree,
            depth: 10,
            max_buffer_size: 32,
        },
    )
    .await
    .unwrap();
    apply_event(
        &seed,
        CnftEvent::Mint {
            tree,
            owner: [0x01; 32],
            delegate: [0x02; 32],
            metadata: MintMetadata {
                name: "Preloaded".into(),
                symbol: "PRE".into(),
                uri: "https://example.com/pre.json".into(),
                seller_fee_basis_points: 100,
                primary_sale_happened: false,
                is_mutable: true,
                creators: vec![Creator {
                    address: [0x44; 32],
                    verified: true,
                    share: 100,
                }],
                collection: None,
                data_hash_input: vec![0xab; 16],
            },
            verify_collection: None,
            noop: None,
        },
    )
    .await
    .unwrap();

    let snapshot = dump_tree(&seed, &tree).await.unwrap().expect("Some");
    let blob = SnapshotBlob::from_tree(&snapshot);

    // Write the blob to a temp file.
    let tmp = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(tmp.path(), serde_json::to_vec(&blob).unwrap()).unwrap();

    // Boot tidepool with --snapshot set. Explicit ws_port via the
    // harness helper to dodge parallel port races.
    let upstream_url = spawn_mock_upstream_by_method(std::collections::HashMap::new()).await;
    let (port, ws_port) = pick_two_free_ports().await;

    let config = ServerConfig {
        port,
        ws_port: Some(ws_port),
        upstream_url,
        upstream_ws_url: "ws://127.0.0.1:1".into(),
        rpc_timeout: Duration::from_secs(5),
        index_trees: vec![],
        db: None,
        snapshots: vec![tmp.path().to_path_buf()],
    };
    tokio::spawn(async move {
        run(config).await.unwrap();
    });
    tokio::time::sleep(Duration::from_millis(200)).await;

    // getAssetProof should serve out of the preloaded state immediately.
    let asset_id = derive_asset_id(&tree, 0);
    let asset_id_b58 = bs58::encode(asset_id).into_string();
    let tidepool_url = format!("http://127.0.0.1:{port}");
    let proof: Value = reqwest::Client::new()
        .post(&tidepool_url)
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "getAssetProof",
            "params": { "id": asset_id_b58 }
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(proof["result"]["tree_id"], bs58::encode(tree).into_string());
}

#[tokio::test]
async fn cors_headers_are_set() {
    let (upstream_url, _seen) = spawn_mock_upstream().await;
    let tidepool_url = spawn_tidepool(upstream_url).await;

    let resp = reqwest::Client::new()
        .post(&tidepool_url)
        .header("Origin", "http://example.com")
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tidepool_info",
            "params": {}
        }))
        .send()
        .await
        .unwrap();

    let headers = resp.headers();
    assert_eq!(
        headers
            .get("access-control-allow-origin")
            .and_then(|v| v.to_str().ok()),
        Some("*")
    );
}
