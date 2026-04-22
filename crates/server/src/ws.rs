//! WebSocket server with `signatureSubscribe` polyfill.
//!
//! Surfpool's native WS doesn't implement Solana's subscription
//! methods, so `@solana/web3.js`'s `confirmTransaction()` and
//! `sendAndConfirm()` hang against it. We polyfill
//! `signatureSubscribe` / `signatureUnsubscribe` via periodic HTTP
//! polling of `getSignatureStatuses` against the upstream RPC URL.
//! Other subscription methods (`accountSubscribe`, `logsSubscribe`,
//! etc.) are currently dropped; the TS version forwards them to the
//! upstream WS, which we can add in a follow-up when Surfpool ships
//! them or when consumers ask.
//!
//! Per-connection state lives for the lifetime of the WS upgrade —
//! when the client disconnects, all outstanding polling tasks are
//! cancelled.

use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};
use std::time::Duration;

use axum::{
    extract::{
        ws::{Message, WebSocket},
        State, WebSocketUpgrade,
    },
    response::Response,
    routing::get,
    Router,
};
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::net::TcpListener;
use tokio::sync::{mpsc, Mutex};
use tokio::task::JoinHandle;
use tracing::{info, warn};

/// Poll interval for getSignatureStatuses. Matches the TS version.
/// Solana finality is slow enough (~13s average) that 500 ms feels
/// snappy without hammering the upstream.
const POLL_INTERVAL: Duration = Duration::from_millis(500);

/// How many polls before we give up — matches `confirmTransaction`'s
/// client-side timeout roughly. Surfpool-local txs confirm in
/// seconds; real mainnet queries might run longer, but 120s is a
/// reasonable ceiling.
const MAX_POLLS: u32 = 240; // 240 × 500 ms = 120 s

/// Global subscription id counter. Scoped per-process so ids stay
/// unique across connections — matches what real RPC nodes do.
static NEXT_SUB_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Clone)]
pub struct WsState {
    pub upstream_url: String,
    pub rpc_timeout: Duration,
}

/// Spawn the WS server on `port`. Returns a handle that can be
/// awaited to block until the WS server shuts down — callers
/// typically just `tokio::spawn` it and let it run alongside the HTTP
/// server.
pub async fn run_ws(
    port: u16,
    upstream_url: String,
    rpc_timeout: Duration,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let state = WsState {
        upstream_url,
        rpc_timeout,
    };
    let app = Router::new().route("/", get(ws_upgrade)).with_state(state);
    let addr = std::net::SocketAddr::from(([0, 0, 0, 0], port));
    let listener = TcpListener::bind(&addr).await?;
    info!("tidepool WS listening on ws://{addr}");
    axum::serve(listener, app).await?;
    Ok(())
}

async fn ws_upgrade(ws: WebSocketUpgrade, State(state): State<WsState>) -> Response {
    ws.on_upgrade(move |socket| handle_connection(socket, state))
}

/// One connection's lifetime. The main task owns the outbound sink;
/// polling tasks forward notifications via an mpsc channel.
async fn handle_connection(socket: WebSocket, state: WsState) {
    let (mut sink, mut stream) = socket.split();
    let (tx, mut rx) = mpsc::unbounded_channel::<Message>();

    // Shared map of active subscriptions so signatureUnsubscribe can
    // cancel the corresponding polling task.
    let subs: Arc<Mutex<std::collections::HashMap<u64, JoinHandle<()>>>> =
        Arc::new(Mutex::new(std::collections::HashMap::new()));

    // Spawn a task that writes every outgoing message sequentially.
    // Multiple polling tasks push into `tx`; ordering per-
    // subscription is preserved; cross-subscription ordering doesn't
    // matter (clients dispatch by subscription id).
    let write_task = tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            if sink.send(msg).await.is_err() {
                break;
            }
        }
    });

    // Main read loop.
    while let Some(Ok(msg)) = stream.next().await {
        let Message::Text(text) = msg else {
            // Binary / ping / pong / close — let axum's ws layer
            // handle control frames; binary we don't use.
            if matches!(msg, Message::Close(_)) {
                break;
            }
            continue;
        };

        let Ok(req) = serde_json::from_str::<Value>(&text) else {
            continue;
        };
        let method = req.get("method").and_then(Value::as_str).unwrap_or("");
        let id = req.get("id").cloned().unwrap_or(Value::Null);

        match method {
            "signatureSubscribe" => {
                let sub_id = NEXT_SUB_ID.fetch_add(1, Ordering::Relaxed);
                let Some(signature) = req
                    .get("params")
                    .and_then(Value::as_array)
                    .and_then(|a| a.first())
                    .and_then(Value::as_str)
                    .map(String::from)
                else {
                    send(&tx, &error_msg(&id, -32602, "missing signature param"));
                    continue;
                };
                let commitment = req
                    .get("params")
                    .and_then(Value::as_array)
                    .and_then(|a| a.get(1))
                    .and_then(|v| v.get("commitment"))
                    .and_then(Value::as_str)
                    .unwrap_or("finalized")
                    .to_string();

                // Ack immediately with the subscription id.
                send(&tx, &json!({ "jsonrpc": "2.0", "id": id, "result": sub_id }));

                // Spawn the polling task.
                let poll_tx = tx.clone();
                let state_clone = state.clone();
                let subs_clone = Arc::clone(&subs);
                let handle = tokio::spawn(async move {
                    poll_signature(sub_id, signature, commitment, state_clone, poll_tx).await;
                    // Remove the sub from the map on completion so
                    // signatureUnsubscribe doesn't try to abort a
                    // finished task.
                    subs_clone.lock().await.remove(&sub_id);
                });
                subs.lock().await.insert(sub_id, handle);
            }

            "signatureUnsubscribe" => {
                let Some(sub_id) = req
                    .get("params")
                    .and_then(Value::as_array)
                    .and_then(|a| a.first())
                    .and_then(Value::as_u64)
                else {
                    send(&tx, &error_msg(&id, -32602, "missing subscription id"));
                    continue;
                };
                let removed = subs.lock().await.remove(&sub_id);
                let was_present = removed.is_some();
                if let Some(handle) = removed {
                    handle.abort();
                }
                send(
                    &tx,
                    &json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": was_present
                    }),
                );
            }

            // Every other method is silently dropped for now.
            // Forwarding to upstream WS is a follow-up.
            _ => {
                send(
                    &tx,
                    &error_msg(
                        &id,
                        -32601,
                        &format!("method '{method}' is not supported by the tidepool WS polyfill"),
                    ),
                );
            }
        }
    }

    // Client disconnected. Cancel every outstanding poll and drop the
    // outbound channel (the write task exits when rx closes).
    let mut subs = subs.lock().await;
    for (_, handle) in subs.drain() {
        handle.abort();
    }
    drop(tx);
    let _ = write_task.await;
}

// ─── signature polling ──────────────────────────────────────────────

async fn poll_signature(
    sub_id: u64,
    signature: String,
    commitment: String,
    state: WsState,
    tx: mpsc::UnboundedSender<Message>,
) {
    let client = match reqwest::Client::builder()
        .timeout(state.rpc_timeout)
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            warn!(err = %e, "failed to build reqwest client for ws polling");
            return;
        }
    };
    for _ in 0..MAX_POLLS {
        tokio::time::sleep(POLL_INTERVAL).await;
        let body = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "getSignatureStatuses",
            "params": [[signature], { "searchTransactionHistory": true }]
        });
        let Ok(resp) = client.post(&state.upstream_url).json(&body).send().await else {
            continue;
        };
        let Ok(json): Result<Value, _> = resp.json().await else {
            continue;
        };
        let Some(statuses) = json
            .get("result")
            .and_then(|r| r.get("value"))
            .and_then(Value::as_array)
        else {
            continue;
        };
        let Some(status) = statuses.first() else {
            continue;
        };
        if status.is_null() {
            continue; // not yet seen
        }
        let status_conf = status
            .get("confirmationStatus")
            .and_then(Value::as_str)
            .unwrap_or("");
        if commitment_matches(&commitment, status_conf) {
            // Emit notification and exit.
            let notif = json!({
                "jsonrpc": "2.0",
                "method": "signatureNotification",
                "params": {
                    "result": {
                        "context": json.get("result").and_then(|r| r.get("context")).cloned().unwrap_or(Value::Null),
                        "value": { "err": status.get("err").cloned().unwrap_or(Value::Null) }
                    },
                    "subscription": sub_id
                }
            });
            send(&tx, &notif);
            return;
        }
    }
    warn!(sub_id, signature, "signatureSubscribe poll timed out");
}

fn commitment_matches(requested: &str, actual: &str) -> bool {
    // Solana's commitment ladder: processed < confirmed < finalized.
    // If the request asked for `confirmed`, either `confirmed` or
    // `finalized` actual satisfies. Same pattern Helius uses.
    let rank = |s: &str| match s {
        "processed" => 1,
        "confirmed" => 2,
        "finalized" => 3,
        _ => 0,
    };
    rank(actual) >= rank(requested)
}

// ─── helpers ────────────────────────────────────────────────────────

fn send(tx: &mpsc::UnboundedSender<Message>, value: &Value) {
    let _ = tx.send(Message::Text(value.to_string().into()));
}

fn error_msg(id: &Value, code: i32, message: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": { "code": code, "message": message }
    })
}
