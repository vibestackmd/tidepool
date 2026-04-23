//! WebSocket polyfill integration tests. Spin up a mock HTTP upstream
//! that returns canned `getSignatureStatuses` responses, connect via
//! tokio-tungstenite, send `signatureSubscribe`, assert the
//! `signatureNotification` arrives and carries the right subscription
//! id.

// tokio-tungstenite's Message::Text holds a Utf8Bytes / String that
// clippy flags as "useless conversion" when we deref through Display.
// The pattern here is idiomatic for ws tests; quieting the lint at
// file level keeps the test body readable.
#![allow(
    clippy::useless_conversion,
    clippy::needless_continue,
    clippy::implicit_clone
)]

use std::sync::{
    atomic::{AtomicU32, Ordering},
    Arc,
};
use std::time::Duration;

use axum::{extract::State, routing::post, Json, Router};
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::net::TcpListener;
use tokio_tungstenite::{connect_async, tungstenite::Message};

/// Mock HTTP upstream. Every getSignatureStatuses poll returns
/// `confirmationStatus = "finalized"` after the Nth call so we can
/// test both the "still pending" and "confirmed" branches.
async fn spawn_upstream(confirmed_after: u32) -> String {
    let counter: Arc<AtomicU32> = Arc::new(AtomicU32::new(0));
    let counter_for = Arc::clone(&counter);

    let app = Router::new()
        .route(
            "/",
            post(
                move |State(counter): State<Arc<AtomicU32>>, Json(body): Json<Value>| async move {
                    let method = body.get("method").and_then(Value::as_str).unwrap_or("");
                    if method != "getSignatureStatuses" {
                        return Json(
                            json!({ "jsonrpc": "2.0", "id": body.get("id"), "result": null }),
                        );
                    }
                    let calls = counter.fetch_add(1, Ordering::SeqCst) + 1;
                    let status = if calls >= confirmed_after {
                        json!({
                            "slot": 100,
                            "confirmations": null,
                            "err": null,
                            "confirmationStatus": "finalized"
                        })
                    } else {
                        json!(null)
                    };
                    Json(json!({
                        "jsonrpc": "2.0",
                        "id": body.get("id"),
                        "result": {
                            "context": { "slot": 100 },
                            "value": [status]
                        }
                    }))
                },
            ),
        )
        .with_state(counter_for);

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{addr}")
}

async fn spawn_ws(upstream_url: String) -> u16 {
    let probe = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = probe.local_addr().unwrap().port();
    drop(probe);
    tokio::spawn(async move {
        tidepool_rpc_server::run_ws(port, upstream_url, Duration::from_secs(5))
            .await
            .unwrap();
    });
    tokio::time::sleep(Duration::from_millis(120)).await;
    port
}

#[tokio::test]
async fn signature_subscribe_delivers_notification_on_finalized() {
    let upstream = spawn_upstream(2).await; // confirm after the 2nd poll
    let port = spawn_ws(upstream).await;
    let url = format!("ws://127.0.0.1:{port}/");

    let (mut socket, _) = connect_async(url).await.expect("ws connect");

    // Subscribe.
    socket
        .send(Message::Text(
            json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "signatureSubscribe",
                "params": [
                    "5KCCP2aCCNAg3uM8L7kZC3qZJjf4sWXLQNFfVwS6yV2MuzS8gK6eA9T2PqWu5rkH7Kf7UmhQySiMr8KKxkNpTrvj",
                    { "commitment": "finalized" }
                ]
            })
            .to_string()
            .into(),
        ))
        .await
        .unwrap();

    // First message back is the ack with the sub id.
    let ack_text = next_text(&mut socket).await;
    let ack: Value = serde_json::from_str(&ack_text).unwrap();
    assert_eq!(ack["id"], 1);
    let sub_id = ack["result"].as_u64().expect("numeric sub id");

    // Next message should be the signatureNotification.
    let notif_text = next_text(&mut socket).await;
    let notif: Value = serde_json::from_str(&notif_text).unwrap();
    assert_eq!(notif["method"], "signatureNotification");
    assert_eq!(notif["params"]["subscription"], sub_id);
    assert!(notif["params"]["result"]["value"]["err"].is_null());
}

#[tokio::test]
async fn signature_unsubscribe_acks_and_cancels() {
    // Configure upstream to never confirm — forces us to rely on
    // unsubscribe to cancel the polling task.
    let upstream = spawn_upstream(u32::MAX).await;
    let port = spawn_ws(upstream).await;
    let url = format!("ws://127.0.0.1:{port}/");

    let (mut socket, _) = connect_async(url).await.unwrap();

    socket
        .send(Message::Text(
            json!({
                "jsonrpc": "2.0",
                "id": 10,
                "method": "signatureSubscribe",
                "params": ["5KCCP2aCCNAg3uM8L7kZC3qZJjf4sWXLQNFfVwS6yV2MuzS8gK6eA9T2PqWu5rkH7Kf7UmhQySiMr8KKxkNpTrvj"]
            })
            .to_string()
            .into(),
        ))
        .await
        .unwrap();
    let ack: Value = serde_json::from_str(&next_text(&mut socket).await).unwrap();
    let sub_id = ack["result"].as_u64().unwrap();

    // Unsubscribe.
    socket
        .send(Message::Text(
            json!({
                "jsonrpc": "2.0",
                "id": 11,
                "method": "signatureUnsubscribe",
                "params": [sub_id]
            })
            .to_string()
            .into(),
        ))
        .await
        .unwrap();
    let unsub_resp: Value = serde_json::from_str(&next_text(&mut socket).await).unwrap();
    assert_eq!(unsub_resp["id"], 11);
    assert_eq!(unsub_resp["result"], true);
}

#[tokio::test]
async fn unsupported_method_returns_method_not_found() {
    // `programSubscribe` is representative of the WS methods we
    // haven't polyfilled yet — expect a clean -32601 response.
    let upstream = spawn_upstream(0).await;
    let port = spawn_ws(upstream).await;
    let url = format!("ws://127.0.0.1:{port}/");

    let (mut socket, _) = connect_async(url).await.unwrap();
    socket
        .send(Message::Text(
            json!({
                "jsonrpc": "2.0",
                "id": 99,
                "method": "programSubscribe",
                "params": []
            })
            .to_string()
            .into(),
        ))
        .await
        .unwrap();
    let err: Value = serde_json::from_str(&next_text(&mut socket).await).unwrap();
    assert_eq!(err["id"], 99);
    assert_eq!(err["error"]["code"], -32601);
}

#[tokio::test]
async fn logs_subscribe_rejects_all_filter() {
    // Polling-based polyfill can't fan out "all" traffic efficiently.
    // Clients asking for `"all"` / `"allWithVotes"` get a typed error
    // instead of silent acceptance.
    let upstream = spawn_upstream(0).await;
    let port = spawn_ws(upstream).await;
    let url = format!("ws://127.0.0.1:{port}/");

    let (mut socket, _) = connect_async(url).await.unwrap();
    socket
        .send(Message::Text(
            json!({
                "jsonrpc": "2.0",
                "id": 77,
                "method": "logsSubscribe",
                "params": ["all"]
            })
            .to_string()
            .into(),
        ))
        .await
        .unwrap();
    let err: Value = serde_json::from_str(&next_text(&mut socket).await).unwrap();
    assert_eq!(err["id"], 77);
    assert_eq!(err["error"]["code"], -32601);
    assert!(err["error"]["message"]
        .as_str()
        .unwrap()
        .contains("mentions"));
}

/// Upstream that serves `getSignaturesForAddress` + `getTransaction`
/// for the logsSubscribe polyfill. First sig response is a baseline
/// (no emit); on the 2nd call we inject a new signature so the
/// polyfill fans out to getTransaction and emits a logsNotification.
async fn spawn_logs_upstream(mention: &'static str) -> String {
    let state: Arc<tokio::sync::Mutex<u32>> = Arc::new(tokio::sync::Mutex::new(0));
    let state_clone = Arc::clone(&state);

    let app = Router::new()
        .route(
            "/",
            post(
                move |State(state): State<Arc<tokio::sync::Mutex<u32>>>,
                      Json(body): Json<Value>| {
                    let state = Arc::clone(&state);
                    async move {
                        let method = body.get("method").and_then(Value::as_str).unwrap_or("");
                        let id = body.get("id").cloned().unwrap_or(Value::Null);
                        match method {
                            "getSignaturesForAddress" => {
                                // Confirm the filter pubkey matches the subscribed mention.
                                assert_eq!(
                                    body["params"][0].as_str().unwrap_or(""),
                                    mention,
                                    "upstream saw wrong mention filter"
                                );
                                let mut calls = state.lock().await;
                                *calls += 1;
                                let sigs = if *calls == 1 {
                                    json!([{ "signature": "BASELINE_SIG", "slot": 100 }])
                                } else {
                                    json!([
                                        { "signature": "NEW_SIG_1", "slot": 101 },
                                        { "signature": "BASELINE_SIG", "slot": 100 }
                                    ])
                                };
                                Json(json!({ "jsonrpc": "2.0", "id": id, "result": sigs }))
                            }
                            "getTransaction" => Json(json!({
                                "jsonrpc": "2.0",
                                "id": id,
                                "result": {
                                    "slot": 101,
                                    "meta": {
                                        "err": null,
                                        "logMessages": [
                                            "Program log: hello",
                                            "Program log: world"
                                        ]
                                    }
                                }
                            })),
                            _ => Json(json!({ "jsonrpc": "2.0", "id": id, "result": null })),
                        }
                    }
                },
            ),
        )
        .with_state(state_clone);

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{addr}")
}

#[tokio::test]
async fn logs_subscribe_emits_notification_for_mention() {
    const PUBKEY: &str = "Mentions111111111111111111111111111111111111";
    let upstream = spawn_logs_upstream(PUBKEY).await;
    let port = spawn_ws(upstream).await;
    let url = format!("ws://127.0.0.1:{port}/");

    let (mut socket, _) = connect_async(url).await.unwrap();
    socket
        .send(Message::Text(
            json!({
                "jsonrpc": "2.0",
                "id": 42,
                "method": "logsSubscribe",
                "params": [
                    { "mentions": [PUBKEY] },
                    { "commitment": "finalized" }
                ]
            })
            .to_string()
            .into(),
        ))
        .await
        .unwrap();

    let ack: Value = serde_json::from_str(&next_text(&mut socket).await).unwrap();
    assert_eq!(ack["id"], 42);
    let sub_id = ack["result"].as_u64().expect("numeric sub id");

    // Next message is the logsNotification for NEW_SIG_1.
    let notif: Value = serde_json::from_str(&next_text(&mut socket).await).unwrap();
    assert_eq!(notif["method"], "logsNotification");
    assert_eq!(notif["params"]["subscription"], sub_id);
    let value = &notif["params"]["result"]["value"];
    assert_eq!(value["signature"], "NEW_SIG_1");
    assert!(value["err"].is_null());
    let logs = value["logs"].as_array().expect("logs array");
    assert_eq!(logs.len(), 2);
    assert_eq!(logs[0], "Program log: hello");
}

/// Mock upstream that varies `getAccountInfo` responses across polls
/// so accountSubscribe's change-detection logic is exercised.
async fn spawn_account_upstream(snapshots: Vec<serde_json::Value>) -> String {
    let snapshots = Arc::new(tokio::sync::Mutex::new(snapshots));
    let app = Router::new()
        .route(
            "/",
            post(
                move |State(snaps): State<Arc<tokio::sync::Mutex<Vec<Value>>>>,
                      Json(body): Json<Value>| {
                    let snaps = Arc::clone(&snaps);
                    async move {
                        let method = body.get("method").and_then(Value::as_str).unwrap_or("");
                        if method != "getAccountInfo" {
                            return Json(
                                json!({ "jsonrpc": "2.0", "id": body.get("id"), "result": null }),
                            );
                        }
                        let mut g = snaps.lock().await;
                        // Pop next snapshot; stick on the last one once exhausted.
                        let value = if g.len() > 1 {
                            g.remove(0)
                        } else {
                            g.first().cloned().unwrap_or(Value::Null)
                        };
                        Json(json!({
                            "jsonrpc": "2.0",
                            "id": body.get("id"),
                            "result": {
                                "context": { "slot": 123 },
                                "value": value
                            }
                        }))
                    }
                },
            ),
        )
        .with_state(snapshots);

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{addr}")
}

fn account_snapshot(lamports: u64, data: &str) -> Value {
    json!({
        "data": [data, "base64"],
        "executable": false,
        "lamports": lamports,
        "owner": "11111111111111111111111111111111",
        "rentEpoch": 0
    })
}

#[tokio::test]
async fn account_subscribe_emits_on_state_change() {
    // First two polls return snapshot A, then snapshots B, C — should
    // produce three notifications total (A baseline, B change, C change).
    let snapshots = vec![
        account_snapshot(1_000, "AA=="),
        account_snapshot(1_000, "AA=="),
        account_snapshot(2_000, "BB=="),
        account_snapshot(3_000, "CC=="),
    ];
    let upstream = spawn_account_upstream(snapshots).await;
    let port = spawn_ws(upstream).await;
    let url = format!("ws://127.0.0.1:{port}/");

    let (mut socket, _) = connect_async(url).await.unwrap();
    socket
        .send(Message::Text(
            json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "accountSubscribe",
                "params": [
                    "AcctTest1111111111111111111111111111111111",
                    { "commitment": "confirmed", "encoding": "base64" }
                ]
            })
            .to_string()
            .into(),
        ))
        .await
        .unwrap();

    let ack: Value = serde_json::from_str(&next_text(&mut socket).await).unwrap();
    let sub_id = ack["result"].as_u64().expect("sub id");

    // Read three notifications — polls beyond the 3rd (repeat of C)
    // should NOT emit because state is unchanged.
    let mut seen_lamports: Vec<u64> = Vec::new();
    for _ in 0..3 {
        let notif: Value = serde_json::from_str(&next_text(&mut socket).await).unwrap();
        assert_eq!(notif["method"], "accountNotification");
        assert_eq!(notif["params"]["subscription"], sub_id);
        let l = notif["params"]["result"]["value"]["lamports"]
            .as_u64()
            .expect("lamports");
        seen_lamports.push(l);
    }
    assert_eq!(seen_lamports, vec![1_000, 2_000, 3_000]);
}

#[tokio::test]
async fn account_unsubscribe_acks_and_cancels() {
    // Upstream returns a fixed snapshot — no change after baseline, so
    // polling is quiet and unsubscribe is what stops the task.
    let upstream = spawn_account_upstream(vec![account_snapshot(5_000, "ZZ==")]).await;
    let port = spawn_ws(upstream).await;
    let url = format!("ws://127.0.0.1:{port}/");

    let (mut socket, _) = connect_async(url).await.unwrap();
    socket
        .send(Message::Text(
            json!({
                "jsonrpc": "2.0",
                "id": 5,
                "method": "accountSubscribe",
                "params": ["AcctTest1111111111111111111111111111111111"]
            })
            .to_string()
            .into(),
        ))
        .await
        .unwrap();
    let ack: Value = serde_json::from_str(&next_text(&mut socket).await).unwrap();
    let sub_id = ack["result"].as_u64().unwrap();
    // Read the baseline notification so subsequent reads don't race.
    let _baseline: Value = serde_json::from_str(&next_text(&mut socket).await).unwrap();

    socket
        .send(Message::Text(
            json!({
                "jsonrpc": "2.0",
                "id": 6,
                "method": "accountUnsubscribe",
                "params": [sub_id]
            })
            .to_string()
            .into(),
        ))
        .await
        .unwrap();
    let resp: Value = serde_json::from_str(&next_text(&mut socket).await).unwrap();
    assert_eq!(resp["id"], 6);
    assert_eq!(resp["result"], true);
}

async fn next_text<S>(socket: &mut S) -> String
where
    S: StreamExt<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin,
{
    loop {
        match tokio::time::timeout(Duration::from_secs(10), socket.next()).await {
            Ok(Some(Ok(Message::Text(t)))) => return t.to_string(),
            Ok(Some(Ok(_))) => continue, // skip pings / binary
            other => panic!("expected text frame, got {other:?}"),
        }
    }
}
