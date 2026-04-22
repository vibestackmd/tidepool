//! Server integration tests. Bind the server to an ephemeral port,
//! point it at a mock upstream axum server, and assert end-to-end
//! behavior for:
//!   - native dispatch (surfpoolHeliusInfo)
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
            post(|State(seen): State<Arc<tokio::sync::Mutex<Vec<Value>>>>, Json(body): Json<Value>| async move {
                seen.lock().await.push(body.clone());
                let id = body.get("id").cloned().unwrap_or(Value::Null);
                Json(json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": "upstream-saw-you"
                }))
            }),
        )
        .with_state(seen_for_handler);

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (format!("http://{addr}"), seen)
}

async fn spawn_tidepool(upstream_url: String) -> String {
    use tidepool_rpc_server::{run, ServerConfig};
    // Pick a free port by binding briefly.
    let probe = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = probe.local_addr().unwrap().port();
    drop(probe);

    let config = ServerConfig {
        port,
        upstream_url,
        upstream_ws_url: "ws://127.0.0.1:1".into(),
        rpc_timeout: Duration::from_secs(5),
        index_trees: vec![],
    };
    tokio::spawn(async move {
        run(config).await.unwrap();
    });
    // Give axum a beat to bind.
    tokio::time::sleep(Duration::from_millis(120)).await;
    format!("http://127.0.0.1:{port}")
}

#[tokio::test]
async fn surfpool_helius_info_native_dispatch() {
    let (upstream_url, _seen) = spawn_mock_upstream().await;
    let tidepool_url = spawn_tidepool(upstream_url).await;

    let client = reqwest::Client::new();
    let resp: Value = client
        .post(&tidepool_url)
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "surfpoolHeliusInfo",
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
    assert!(methods
        .iter()
        .any(|m| m["method"] == "surfpoolHeliusIndexTree"));
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
async fn cors_headers_are_set() {
    let (upstream_url, _seen) = spawn_mock_upstream().await;
    let tidepool_url = spawn_tidepool(upstream_url).await;

    let resp = reqwest::Client::new()
        .post(&tidepool_url)
        .header("Origin", "http://example.com")
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "surfpoolHeliusInfo",
            "params": {}
        }))
        .send()
        .await
        .unwrap();

    let headers = resp.headers();
    assert_eq!(
        headers.get("access-control-allow-origin").and_then(|v| v.to_str().ok()),
        Some("*")
    );
}
