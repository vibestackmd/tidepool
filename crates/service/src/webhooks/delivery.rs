//! Per-webhook polling + delivery loop.
//!
//! For each registered webhook we spawn one background task. The task:
//!   1. Polls `getSignaturesForAddress` across every address on the
//!      webhook's `account_addresses` list every `POLL_INTERVAL`.
//!   2. Tracks a "last seen signature" cursor per address so repeat
//!      scans don't re-deliver old txs.
//!   3. Applies the webhook's `txn_status` filter (`success` /
//!      `failed` / default `all`) at the signature stage — cheap pre-
//!      filter before we spend an extra `getTransaction` call.
//!   4. Runs each surviving signature through the Enhanced
//!      Transactions parser to produce a full `EnhancedTransaction`.
//!   5. Applies the webhook's `transaction_types` filter on the
//!      parsed result.
//!   6. POSTs the resulting `Vec<EnhancedTransaction>` to the user's
//!      `webhook_url`.
//!
//! Pure plumbing around the `UpstreamClient` trait — delivery uses
//! an injected HTTP `PostClient` so tests can capture payloads without
//! spinning up a real HTTP server.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::sync::Mutex;
use tracing::warn;

use super::types::Webhook;
use crate::enhanced::{parse_enhanced_tx, EnhancedTransaction};
use crate::upstream::UpstreamClient;

/// 500 ms matches the WS polling cadence and gives snappy local-dev
/// latency without hammering the upstream.
pub const POLL_INTERVAL: Duration = Duration::from_millis(500);

/// Abstraction over the HTTP POST used for delivery. Tests inject a
/// recording impl; production uses `ReqwestPostClient`.
#[async_trait]
pub trait PostClient: Send + Sync {
    async fn post_json(&self, url: &str, auth: Option<&str>, body: &Value) -> Result<(), String>;
}

/// Run one delivery tick for `webhook`. Exposed (vs. embedded in a
/// loop) so tests can drive a single poll deterministically.
#[allow(clippy::implicit_hasher)]
pub async fn tick_once<U, P>(
    webhook: &Webhook,
    upstream: &U,
    poster: &P,
    cursors: &Mutex<HashMap<String, String>>,
) -> Vec<EnhancedTransaction>
where
    U: UpstreamClient + ?Sized,
    P: PostClient + ?Sized,
{
    let mut out: Vec<EnhancedTransaction> = Vec::new();

    for address in &webhook.account_addresses {
        let cursor = cursors.lock().await.get(address).cloned();
        let Some((fresh_sigs, newest_signature)) =
            fetch_new_signatures(upstream, address, cursor.as_deref(), webhook).await
        else {
            continue;
        };
        if let Some(sig) = newest_signature {
            cursors.lock().await.insert(address.clone(), sig);
        }
        for sig in fresh_sigs {
            if let Some(etx) = fetch_enhanced(upstream, &sig).await {
                // Helius's own webhook payloads don't carry a "which
                // registered address fired this" field — receivers
                // infer from the tx body. We match that shape.
                out.push(etx);
            }
        }
    }

    // `transaction_types` filter: empty list = deliver everything;
    // non-empty = keep only matching `tx_type`.
    if !webhook.transaction_types.is_empty() {
        out.retain(|e| webhook.transaction_types.iter().any(|t| t == &e.tx_type));
    }

    if !out.is_empty() {
        let payload = json!(out);
        if let Err(e) = poster
            .post_json(
                &webhook.webhook_url,
                webhook.auth_header.as_deref(),
                &payload,
            )
            .await
        {
            warn!(webhook = %webhook.webhook_id, err = %e, "webhook delivery failed");
        }
    }
    out
}

/// Fetch signatures newer than `cursor` for a single address and
/// apply the webhook's `txn_status` filter. Returns
/// `(surviving_signatures, newest_sig_seen_even_if_filtered)` so the
/// cursor advances past filtered-out txs too.
async fn fetch_new_signatures<U: UpstreamClient + ?Sized>(
    upstream: &U,
    address: &str,
    cursor: Option<&str>,
    webhook: &Webhook,
) -> Option<(Vec<String>, Option<String>)> {
    let mut opts = serde_json::Map::new();
    opts.insert("limit".into(), json!(50));
    if let Some(c) = cursor {
        // `until` is Solana RPC's "stop when you hit this signature"
        // sentinel. Results are newest-first; we reverse below.
        opts.insert("until".into(), json!(c));
    }
    let raw = upstream
        .rpc_call(
            "getSignaturesForAddress",
            json!([address, serde_json::Value::Object(opts)]),
        )
        .await
        .ok()?;
    let parsed: Value = serde_json::from_slice(&raw).ok()?;
    let arr = parsed.as_array().cloned().unwrap_or_default();
    if arr.is_empty() {
        return Some((Vec::new(), None));
    }

    let newest_sig = arr
        .first()
        .and_then(|e| e.get("signature"))
        .and_then(Value::as_str)
        .map(String::from);

    let want_status = webhook.txn_status.as_deref().unwrap_or("all");
    let mut out = Vec::with_capacity(arr.len());
    for entry in arr.into_iter().rev() {
        let Some(signature) = entry.get("signature").and_then(Value::as_str) else {
            continue;
        };
        let err = entry.get("err").cloned().filter(|v| !v.is_null());
        let failed = err.is_some();
        let keep = match want_status {
            "success" => !failed,
            "failed" => failed,
            _ => true,
        };
        if !keep {
            continue;
        }
        out.push(signature.to_string());
    }

    Some((out, newest_sig))
}

async fn fetch_enhanced<U: UpstreamClient + ?Sized>(
    upstream: &U,
    signature: &str,
) -> Option<EnhancedTransaction> {
    let params = json!([
        signature,
        {
            "encoding": "json",
            "maxSupportedTransactionVersion": 0,
            "commitment": "confirmed"
        }
    ]);
    let raw = upstream.rpc_call("getTransaction", params).await.ok()?;
    if raw.is_empty() || raw == b"null" {
        return None;
    }
    let parsed: Value = serde_json::from_slice(&raw).ok()?;
    parse_enhanced_tx(signature, &parsed)
}

/// Spawn a long-lived polling task for `webhook`. Returns a JoinHandle
/// the caller stores so `deleteWebhook` can abort it cleanly.
pub fn spawn_delivery_task<U, P>(
    webhook: Webhook,
    upstream: Arc<U>,
    poster: Arc<P>,
) -> tokio::task::JoinHandle<()>
where
    U: UpstreamClient + ?Sized + 'static,
    P: PostClient + ?Sized + 'static,
{
    tokio::spawn(async move {
        let cursors: Mutex<HashMap<String, String>> = Mutex::new(HashMap::new());
        loop {
            tokio::time::sleep(POLL_INTERVAL).await;
            let _events = tick_once(&webhook, &*upstream, &*poster, &cursors).await;
        }
    })
}
