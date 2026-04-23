//! WebSocket server with polling-based subscription polyfills.
//!
//! Surfpool's native WS doesn't implement Solana's subscription
//! methods, so `@solana/web3.js`'s `confirmTransaction()` and similar
//! hang against it. We polyfill the commonly-needed subscriptions via
//! periodic HTTP polling of the upstream RPC URL:
//!
//! - `signatureSubscribe` → `getSignatureStatuses` (one-shot; resolves
//!   when the tx reaches the requested commitment).
//! - `accountSubscribe` → `getAccountInfo` (long-lived; emits a
//!   notification every time the account's observed state changes).
//! - `logsSubscribe({mentions: [pubkey]})` → `getSignaturesForAddress` + `getTransaction` fan-out. Each new sig that mentions the given pubkey emits a `logsNotification` with the extracted log array. `all` / `allWithVotes` filters aren't polyfilled (no efficient polling shim) — clients asking for those get a typed error.
//!
//! Other subscription methods (`programSubscribe`, `slotSubscribe`,
//! etc.) are not yet polyfilled.
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
#[allow(clippy::too_many_lines)]
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

            "accountSubscribe" => {
                let sub_id = NEXT_SUB_ID.fetch_add(1, Ordering::Relaxed);
                let Some(pubkey) = req
                    .get("params")
                    .and_then(Value::as_array)
                    .and_then(|a| a.first())
                    .and_then(Value::as_str)
                    .map(String::from)
                else {
                    send(&tx, &error_msg(&id, -32602, "missing account pubkey param"));
                    continue;
                };
                let opts = req
                    .get("params")
                    .and_then(Value::as_array)
                    .and_then(|a| a.get(1))
                    .cloned()
                    .unwrap_or(Value::Null);
                let commitment = opts
                    .get("commitment")
                    .and_then(Value::as_str)
                    .unwrap_or("finalized")
                    .to_string();
                // Default Solana RPC encoding for accountSubscribe is
                // base58; clients usually want base64 or jsonParsed.
                let encoding = opts
                    .get("encoding")
                    .and_then(Value::as_str)
                    .unwrap_or("base64")
                    .to_string();

                send(&tx, &json!({ "jsonrpc": "2.0", "id": id, "result": sub_id }));

                let poll_tx = tx.clone();
                let state_clone = state.clone();
                let subs_clone = Arc::clone(&subs);
                let handle = tokio::spawn(async move {
                    poll_account(sub_id, pubkey, commitment, encoding, state_clone, poll_tx).await;
                    subs_clone.lock().await.remove(&sub_id);
                });
                subs.lock().await.insert(sub_id, handle);
            }

            "logsSubscribe" => {
                let sub_id = NEXT_SUB_ID.fetch_add(1, Ordering::Relaxed);
                let params = req.get("params").and_then(Value::as_array);
                let filter = params.and_then(|a| a.first()).cloned().unwrap_or(Value::Null);
                // Supported filter shapes: `{ mentions: [pubkey] }`.
                // Reject `"all"` / `"allWithVotes"` with a typed error —
                // there's no cheap polling shim for them.
                let mention = match &filter {
                    Value::Object(map) => map
                        .get("mentions")
                        .and_then(Value::as_array)
                        .and_then(|a| a.first())
                        .and_then(Value::as_str)
                        .map(String::from),
                    Value::String(s) if s == "all" || s == "allWithVotes" => {
                        send(
                            &tx,
                            &error_msg(
                                &id,
                                -32601,
                                "logsSubscribe with filter 'all' / 'allWithVotes' is not \
                                 polyfilled by the tidepool WS shim; use { mentions: [pubkey] }",
                            ),
                        );
                        continue;
                    }
                    _ => None,
                };
                let Some(mention) = mention else {
                    send(
                        &tx,
                        &error_msg(
                            &id,
                            -32602,
                            "logsSubscribe requires `{ mentions: [pubkey] }` filter",
                        ),
                    );
                    continue;
                };
                let commitment = params
                    .and_then(|a| a.get(1))
                    .and_then(|v| v.get("commitment"))
                    .and_then(Value::as_str)
                    .unwrap_or("finalized")
                    .to_string();

                send(&tx, &json!({ "jsonrpc": "2.0", "id": id, "result": sub_id }));

                let poll_tx = tx.clone();
                let state_clone = state.clone();
                let subs_clone = Arc::clone(&subs);
                let handle = tokio::spawn(async move {
                    poll_logs(sub_id, mention, commitment, state_clone, poll_tx).await;
                    subs_clone.lock().await.remove(&sub_id);
                });
                subs.lock().await.insert(sub_id, handle);
            }

            "signatureUnsubscribe" | "accountUnsubscribe" | "logsUnsubscribe" => {
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

// ─── account polling ────────────────────────────────────────────────

/// Long-lived account polling loop. Emits an `accountNotification`
/// each time `getAccountInfo` returns a state that differs from the
/// previously observed value. First poll emits the current state as
/// the baseline; subsequent polls compare `{data, owner, lamports,
/// executable, rentEpoch}` and only push on change.
///
/// Runs until the task is aborted (on `accountUnsubscribe` or client
/// disconnect). Transient HTTP errors skip a cycle; we don't fail the
/// subscription so clients stay connected across brief upstream
/// flakes.
async fn poll_account(
    sub_id: u64,
    pubkey: String,
    commitment: String,
    encoding: String,
    state: WsState,
    tx: mpsc::UnboundedSender<Message>,
) {
    let client = match reqwest::Client::builder()
        .timeout(state.rpc_timeout)
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            warn!(err = %e, "failed to build reqwest client for account polling");
            return;
        }
    };
    let mut last: Option<Value> = None;
    loop {
        tokio::time::sleep(POLL_INTERVAL).await;
        let body = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "getAccountInfo",
            "params": [pubkey, { "commitment": commitment, "encoding": encoding }]
        });
        let Ok(resp) = client.post(&state.upstream_url).json(&body).send().await else {
            continue;
        };
        let Ok(json): Result<Value, _> = resp.json().await else {
            continue;
        };
        let Some(result) = json.get("result") else {
            continue;
        };
        // The `value` field is the account snapshot (may be Null when
        // the account doesn't exist yet). `context` we include in the
        // notification to match Helius / native Solana RPC shape.
        let value = result.get("value").cloned().unwrap_or(Value::Null);
        if last.as_ref() == Some(&value) {
            continue;
        }
        last = Some(value.clone());
        let notif = json!({
            "jsonrpc": "2.0",
            "method": "accountNotification",
            "params": {
                "result": {
                    "context": result.get("context").cloned().unwrap_or(Value::Null),
                    "value": value
                },
                "subscription": sub_id
            }
        });
        send(&tx, &notif);
    }
}

// ─── logs polling (mentions filter) ─────────────────────────────────

/// Poll `getSignaturesForAddress(mention)` at `POLL_INTERVAL` and fan
/// out to `getTransaction` for each new sig. Emits one
/// `logsNotification` per new tx with the `logMessages` array extracted
/// from meta. Runs until aborted.
///
/// Dedup strategy: remember the last-seen signature and page fresh
/// results ahead of it. The first poll sets the baseline without
/// emitting — matches Solana's "only notify on state change after
/// subscribe" semantics for the other subscriptions.
async fn poll_logs(
    sub_id: u64,
    mention: String,
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
            warn!(err = %e, "failed to build reqwest client for logs polling");
            return;
        }
    };
    let mut last_seen: Option<String> = None;
    loop {
        tokio::time::sleep(POLL_INTERVAL).await;
        let sigs_body = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "getSignaturesForAddress",
            "params": [mention, { "commitment": commitment, "limit": 25 }]
        });
        let Ok(resp) = client.post(&state.upstream_url).json(&sigs_body).send().await else {
            continue;
        };
        let Ok(json): Result<Value, _> = resp.json().await else {
            continue;
        };
        let Some(entries) = json.get("result").and_then(Value::as_array) else {
            continue;
        };

        // Collect new sigs in chronological order (upstream returns
        // newest-first, so iterate reversed, stopping at the last-seen
        // boundary).
        let mut new_sigs: Vec<String> = Vec::new();
        for entry in entries.iter().rev() {
            let Some(sig) = entry.get("signature").and_then(Value::as_str) else {
                continue;
            };
            if last_seen.as_deref() == Some(sig) {
                new_sigs.clear();
                continue;
            }
            new_sigs.push(sig.to_string());
        }
        // If we had no baseline, just set it and skip emitting — the
        // subscription model emits on *future* events, not on the
        // already-landed tx list.
        if last_seen.is_none() {
            if let Some(sig) = entries
                .first()
                .and_then(|e| e.get("signature"))
                .and_then(Value::as_str)
            {
                last_seen = Some(sig.to_string());
            }
            continue;
        }

        for sig in &new_sigs {
            if let Some(notif) = fetch_logs_notification(&client, &state, &commitment, sub_id, sig).await {
                send(&tx, &notif);
            }
        }
        if let Some(last) = new_sigs.last() {
            last_seen = Some(last.clone());
        }
    }
}

/// One-shot `getTransaction` → `logsNotification` payload build. None
/// on transient upstream error — caller continues polling.
async fn fetch_logs_notification(
    client: &reqwest::Client,
    state: &WsState,
    commitment: &str,
    sub_id: u64,
    signature: &str,
) -> Option<Value> {
    let body = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "getTransaction",
        "params": [
            signature,
            { "commitment": commitment, "encoding": "json", "maxSupportedTransactionVersion": 0 }
        ]
    });
    let resp = client.post(&state.upstream_url).json(&body).send().await.ok()?;
    let json: Value = resp.json().await.ok()?;
    let result = json.get("result")?;
    let slot = result.get("slot").and_then(Value::as_u64).unwrap_or(0);
    let meta = result.get("meta").cloned().unwrap_or(Value::Null);
    let err = meta.get("err").cloned().unwrap_or(Value::Null);
    let logs = meta
        .get("logMessages")
        .cloned()
        .unwrap_or(Value::Array(Vec::new()));
    Some(json!({
        "jsonrpc": "2.0",
        "method": "logsNotification",
        "params": {
            "result": {
                "context": { "slot": slot },
                "value": {
                    "signature": signature,
                    "err": err,
                    "logs": logs
                }
            },
            "subscription": sub_id
        }
    }))
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
