//! Indexer orchestrator. Given an upstream + store, pulls signature
//! history for a tree, walks each transaction, extracts Bubblegum ixs
//! (outer + inner, paired with their noop LeafSchemaEvents), parses
//! them into `CnftEvent`s, and applies each to the store.
//!
//! Not a daemon — each call does one incremental pass from the store's
//! last-indexed-signature cursor forward and returns. Callers choose
//! when to re-run. That keeps this file free of timers, cancellation
//! state, and background tasks — easier to reason about, easier to
//! port to other transport layers.
//!
//! Failure strategy: individual tx / ix failures never halt the scan.
//! We log via `tracing` and continue. Advance-and-skip is the policy
//! for known-failed txs (so a re-scan doesn't re-process them); skip-
//! without-advance is the policy for transient fetch errors (so they
//! get retried on the next pass).

use serde_json::json;
use thiserror::Error;
use tracing::warn;

use super::apply::{apply_event, ApplyError};
use super::parser::parse_bubblegum_instruction;
use super::store::{CnftStore, StoreError};
use super::tx_extract::{extract_bubblegum_ixs, RpcTransactionResponse};
use crate::upstream::{UpstreamClient, UpstreamError};

#[derive(Debug, Error)]
pub enum IndexError {
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error(transparent)]
    Apply(#[from] ApplyError),
    #[error(transparent)]
    Upstream(#[from] UpstreamError),
    #[error("malformed signature batch from upstream: {0}")]
    MalformedSignatures(String),
}

pub type IndexResult<T> = Result<T, IndexError>;

#[derive(Debug, Clone)]
pub struct IndexTreeOptions {
    /// Cap on total signatures fetched in a single `index_tree` call.
    /// Safety rail against accidentally backfilling a 100k-tx tree in
    /// one shot. `None` = uncapped.
    pub max_signatures: Option<usize>,
    /// Page size for `getSignaturesForAddress`. Solana RPC caps this
    /// at 1000.
    pub page_size: usize,
}

impl Default for IndexTreeOptions {
    fn default() -> Self {
        Self {
            max_signatures: Some(10_000),
            page_size: 1000,
        }
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct IndexTreeResult {
    /// Signatures we fetched and fully processed this call.
    pub processed: usize,
    /// Ixs parsed + applied across all processed txs.
    pub applied: usize,
    /// Ixs we encountered but skipped (parse errors, unknown
    /// discriminators, apply errors logged and moved past).
    pub skipped: usize,
}

/// One incremental pass. Safe to call repeatedly; picks up from the
/// store's `last_signature` cursor each time.
pub async fn index_tree<U, S>(
    upstream: &U,
    store: &S,
    tree: [u8; 32],
    options: &IndexTreeOptions,
) -> IndexResult<IndexTreeResult>
where
    U: UpstreamClient + ?Sized,
    S: CnftStore + ?Sized,
{
    let cursor = store.get_last_signature(&tree).await?;
    let sigs = fetch_signatures_until(upstream, &tree, cursor.as_deref(), options).await?;

    let mut result = IndexTreeResult::default();
    // RPC returns newest-first; we reverse to chronological so state
    // transitions replay in order.
    for entry in sigs.into_iter().rev() {
        if !entry.err_is_null() {
            // On-chain failure — no state to replay. Still advance the
            // cursor so a re-scan skips cheaply.
            store.set_last_signature(&tree, entry.signature.clone()).await?;
            result.processed += 1;
            continue;
        }

        let Some(tx) = fetch_transaction(upstream, &entry.signature).await else {
            // Skip without advancing — retry on next pass.
            result.skipped += 1;
            continue;
        };

        let ixs = extract_bubblegum_ixs(&tx);
        for ix in ixs {
            match parse_bubblegum_instruction(&ix.data, &ix.accounts, ix.noop_event.as_ref()) {
                Ok(Some(event)) => match apply_event(store, event).await {
                    Ok(()) => result.applied += 1,
                    Err(e) => {
                        warn!(sig = %entry.signature, err = %e, "apply_event failed; skipping");
                        result.skipped += 1;
                    }
                },
                Ok(None) => {
                    // Known Bubblegum ix we don't track (Redeem, V2 family, ...).
                    result.skipped += 1;
                }
                Err(e) => {
                    warn!(sig = %entry.signature, err = %e, "parse failed; skipping");
                    result.skipped += 1;
                }
            }
        }

        store.set_last_signature(&tree, entry.signature.clone()).await?;
        result.processed += 1;
    }

    Ok(result)
}

// ─── signature paging ──────────────────────────────────────────────

#[derive(Debug, Clone)]
struct SignatureEntry {
    signature: String,
    err: serde_json::Value,
}

impl SignatureEntry {
    fn err_is_null(&self) -> bool {
        self.err.is_null()
    }
}

async fn fetch_signatures_until<U: UpstreamClient + ?Sized>(
    upstream: &U,
    tree: &[u8; 32],
    until: Option<&str>,
    options: &IndexTreeOptions,
) -> IndexResult<Vec<SignatureEntry>> {
    let tree_b58 = bs58::encode(tree).into_string();
    let mut collected: Vec<SignatureEntry> = Vec::new();
    let mut before: Option<String> = None;
    let cap = options.max_signatures.unwrap_or(usize::MAX);

    while collected.len() < cap {
        let remaining = cap.saturating_sub(collected.len());
        let limit = options.page_size.min(remaining);
        let mut opts = serde_json::Map::new();
        opts.insert("limit".into(), json!(limit));
        if let Some(b) = &before {
            opts.insert("before".into(), json!(b));
        }
        if let Some(u) = until {
            opts.insert("until".into(), json!(u));
        }
        let params = json!([tree_b58, serde_json::Value::Object(opts)]);
        let raw = upstream.rpc_call("getSignaturesForAddress", params).await?;
        let page: serde_json::Value = serde_json::from_slice(&raw)
            .map_err(|e| IndexError::MalformedSignatures(e.to_string()))?;

        let entries = match page {
            serde_json::Value::Array(a) => a,
            serde_json::Value::Null => break,
            other => {
                return Err(IndexError::MalformedSignatures(format!(
                    "expected array, got {other:?}"
                )));
            }
        };
        if entries.is_empty() {
            break;
        }
        let batch_len = entries.len();
        for entry in entries {
            let signature = entry
                .get("signature")
                .and_then(|v| v.as_str())
                .ok_or_else(|| {
                    IndexError::MalformedSignatures("missing `signature` field".into())
                })?
                .to_string();
            let err = entry.get("err").cloned().unwrap_or(serde_json::Value::Null);
            collected.push(SignatureEntry { signature, err });
        }
        if batch_len < limit {
            break;
        }
        before = collected.last().map(|e| e.signature.clone());
    }

    Ok(collected)
}

async fn fetch_transaction<U: UpstreamClient + ?Sized>(
    upstream: &U,
    signature: &str,
) -> Option<RpcTransactionResponse> {
    let params = json!([
        signature,
        {
            "encoding": "json",
            "maxSupportedTransactionVersion": 0,
            "commitment": "confirmed"
        }
    ]);
    let raw = match upstream.rpc_call("getTransaction", params).await {
        Ok(r) => r,
        Err(e) => {
            warn!(sig = %signature, err = %e, "getTransaction failed; will retry next pass");
            return None;
        }
    };
    if raw.is_empty() || raw == b"null" {
        return None;
    }
    serde_json::from_slice::<RpcTransactionResponse>(&raw).ok()
}
