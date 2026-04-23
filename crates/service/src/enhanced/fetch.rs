//! Upstream fan-out: take signatures (directly or via
//! getSignaturesForAddress), fetch each tx, produce
//! `EnhancedTransaction`s.

use serde_json::{json, Value};

use super::parse::parse_enhanced_tx;
use super::types::EnhancedTransaction;
use crate::upstream::UpstreamClient;

/// Options Helius accepts on `getTransactionsByAddress`. All optional;
/// we only honor `before`, `until`, `limit` locally and pass `type`
/// through as a post-fetch filter.
#[derive(Debug, Clone, Default)]
pub struct TransactionsByAddressOptions {
    pub before: Option<String>,
    pub until: Option<String>,
    pub limit: Option<u64>,
    /// If set, drop any enhanced tx whose `tx_type` isn't in this list.
    /// Useful since our classifier collapses many Helius types to
    /// UNKNOWN — callers can filter those out cheaply.
    pub types: Vec<String>,
}

/// `getTransactions([sig, sig, ...])` — fan out, preserving order.
/// Signatures that don't resolve upstream are silently dropped.
pub async fn get_transactions<U: UpstreamClient + ?Sized>(
    upstream: &U,
    signatures: &[String],
) -> Vec<EnhancedTransaction> {
    let mut out = Vec::with_capacity(signatures.len());
    for sig in signatures {
        if let Some(etx) = fetch_and_parse(upstream, sig).await {
            out.push(etx);
        }
    }
    out
}

/// `getTransactionsByAddress(address, options)` — resolve signatures
/// via `getSignaturesForAddress` then fan out.
pub async fn get_transactions_by_address<U: UpstreamClient + ?Sized>(
    upstream: &U,
    address: &str,
    options: &TransactionsByAddressOptions,
) -> Vec<EnhancedTransaction> {
    let mut opts_map = serde_json::Map::new();
    if let Some(limit) = options.limit {
        opts_map.insert("limit".into(), json!(limit));
    }
    if let Some(b) = &options.before {
        opts_map.insert("before".into(), json!(b));
    }
    if let Some(u) = &options.until {
        opts_map.insert("until".into(), json!(u));
    }
    let params = json!([address, Value::Object(opts_map)]);
    let Ok(raw) = upstream.rpc_call("getSignaturesForAddress", params).await else {
        return Vec::new();
    };
    let parsed: Value = serde_json::from_slice(&raw).unwrap_or(Value::Null);
    let arr = parsed.as_array().cloned().unwrap_or_default();

    // Solana returns newest-first; preserve that order in the output
    // — matches Helius's default shape.
    let mut out = Vec::with_capacity(arr.len());
    for entry in arr {
        let Some(sig) = entry.get("signature").and_then(Value::as_str) else {
            continue;
        };
        // Skip failed txs up-front when the caller didn't explicitly
        // ask for them via an `include_err` signal; our simpler rule
        // is "deliver everything, let type-filter do the rest". So:
        if let Some(etx) = fetch_and_parse(upstream, sig).await {
            if !options.types.is_empty()
                && !options.types.iter().any(|t| t == &etx.tx_type)
            {
                continue;
            }
            out.push(etx);
        }
    }
    out
}

async fn fetch_and_parse<U: UpstreamClient + ?Sized>(
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
