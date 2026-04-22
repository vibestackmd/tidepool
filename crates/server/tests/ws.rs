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

    let app = Router::new().route(
        "/",
        post(move |State(counter): State<Arc<AtomicU32>>, Json(body): Json<Value>| async move {
            let method = body.get("method").and_then(Value::as_str).unwrap_or("");
            if method != "getSignatureStatuses" {
                return Json(json!({ "jsonrpc": "2.0", "id": body.get("id"), "result": null }));
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
        }),
    ).with_state(counter_for);

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
    let upstream = spawn_upstream(0).await;
    let port = spawn_ws(upstream).await;
    let url = format!("ws://127.0.0.1:{port}/");

    let (mut socket, _) = connect_async(url).await.unwrap();
    socket
        .send(Message::Text(
            json!({
                "jsonrpc": "2.0",
                "id": 99,
                "method": "accountSubscribe",
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
