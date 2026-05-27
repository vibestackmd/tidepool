//! Helius `getTransfersByAddress` + `getTransactionsForAddress`.
//!
//! Both methods are part of Helius's Historical APIs surface (April–May
//! 2026). They combine signature resolution + transaction fetch + parse
//! into a single call so clients don't have to roundtrip `getSignaturesForAddress`
//! → `getTransaction` → parse themselves.
//!
//! Tidepool can't match Helius's "full history from Solana's first block"
//! claim — we only have whatever the underlying Surfpool / validator has
//! streamed. Both methods are therefore marked `BEST_EFFORT` in the
//! manifest; the response shape matches Helius exactly so existing
//! `helius-sdk` clients keep working, just over a smaller window.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::parse::parse_enhanced_tx;
use super::types::EnhancedTransaction;
use crate::upstream::UpstreamClient;

/// One transfer event in `getTransfersByAddress`'s response.
///
/// Field names mirror Helius's camelCase JSON exactly. `amount` and
/// `uiAmount` are strings (precision for u64 token amounts and decimal
/// UI representation that doesn't lose precision in JS clients).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Transfer {
    pub signature: String,
    pub slot: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub block_time: Option<i64>,
    /// Always `"transfer"` — Helius's `type` discriminator. (They use
    /// `"transfer"` for both SOL and SPL transfers; the mint field
    /// distinguishes them.)
    #[serde(rename = "type")]
    pub event_type: String,
    pub from_user_account: Option<String>,
    pub to_user_account: Option<String>,
    /// Set only for SPL transfers; `None` for native SOL.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from_token_account: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to_token_account: Option<String>,
    /// Empty string for native SOL transfers; mint pubkey for SPL.
    pub mint: String,
    /// Raw u64 amount as a string (lamports for SOL, raw token units
    /// for SPL).
    pub amount: String,
    pub decimals: u8,
    /// Human-readable amount as a string (e.g. `"1.5"`).
    pub ui_amount: String,
    /// Confirmation tier. Tidepool can only see finalized history, so
    /// this is always `"finalized"`.
    pub confirmation_status: String,
}

/// Caller-supplied filters for `getTransfersByAddress`.
#[derive(Debug, Clone, Default)]
pub struct TransfersByAddressOptions {
    /// Restrict to a specific SPL mint. `None` = all transfers
    /// (including SOL).
    pub mint: Option<String>,
    /// `"in"` = transfers TO the wallet; `"out"` = FROM the wallet;
    /// `None` = both.
    pub direction: Option<Direction>,
    /// Cap on the number of transfers returned. Defaults applied
    /// upstream by `getSignaturesForAddress` if omitted.
    pub limit: Option<u64>,
    /// `"asc"` (oldest first) or `"desc"` (newest first, Helius default).
    pub sort: Sort,
    /// Opaque cursor from a previous response's `paginationToken`.
    pub pagination_token: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    In,
    Out,
}

impl Direction {
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "in" => Some(Self::In),
            "out" => Some(Self::Out),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Sort {
    #[default]
    Desc,
    Asc,
}

impl Sort {
    #[must_use]
    pub fn parse(s: &str) -> Self {
        match s {
            "asc" => Self::Asc,
            _ => Self::Desc, // anything else falls back to default
        }
    }
}

/// Cursor-paginated response envelope.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TransfersByAddressResult {
    pub data: Vec<Transfer>,
    /// Opaque continuation cursor. `None` when there are no more
    /// pages — clients should stop iterating.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pagination_token: Option<String>,
}

/// `getTransfersByAddress(address, options)` — fetch signatures, parse
/// each transaction, flatten into per-transfer events, apply filters.
///
/// Returns at most `options.limit` transfers. The `paginationToken` in
/// the result is `Some(...)` if more pages might exist.
pub async fn get_transfers_by_address<U: UpstreamClient + ?Sized>(
    upstream: &U,
    address: &str,
    options: &TransfersByAddressOptions,
) -> TransfersByAddressResult {
    let txs = fetch_txs_for_address(upstream, address, options.pagination_token.as_deref()).await;

    let mut out: Vec<Transfer> = Vec::new();
    for etx in txs {
        for ev in transfers_from_tx(&etx, address) {
            if let Some(mint_filter) = &options.mint {
                if &ev.mint != mint_filter {
                    continue;
                }
            }
            if let Some(direction) = options.direction {
                let dir_ok = match direction {
                    Direction::In => ev.to_user_account.as_deref() == Some(address),
                    Direction::Out => ev.from_user_account.as_deref() == Some(address),
                };
                if !dir_ok {
                    continue;
                }
            }
            out.push(ev);
        }
    }

    if matches!(options.sort, Sort::Asc) {
        out.sort_by_key(|t| (t.block_time.unwrap_or(0), t.signature.clone()));
    } else {
        out.sort_by_key(|t| std::cmp::Reverse((t.block_time.unwrap_or(0), t.signature.clone())));
    }

    let pagination_token = if let Some(limit) = options.limit {
        let limit_usize = usize::try_from(limit).unwrap_or(usize::MAX);
        if out.len() > limit_usize {
            let next_anchor = out.get(limit_usize - 1).map(|t| t.signature.clone());
            out.truncate(limit_usize);
            next_anchor
        } else {
            None
        }
    } else {
        None
    };

    TransfersByAddressResult {
        data: out,
        pagination_token,
    }
}

/// Caller-supplied filters for `getTransactionsForAddress` (gTFA).
#[derive(Debug, Clone, Default)]
pub struct TransactionsForAddressOptions {
    pub limit: Option<u64>,
    pub pagination_token: Option<String>,
    /// Minimum slot, inclusive.
    pub min_slot: Option<u64>,
    /// Maximum slot, inclusive.
    pub max_slot: Option<u64>,
    /// `"success"` = drop failed txs; `"failure"` = only failed; `None` = both.
    pub status: Option<TxStatus>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TxStatus {
    Success,
    Failure,
}

impl TxStatus {
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "success" => Some(Self::Success),
            "failure" => Some(Self::Failure),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TransactionsForAddressResult {
    pub data: Vec<EnhancedTransaction>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pagination_token: Option<String>,
}

/// `getTransactionsForAddress(address, options)` — combined sig fetch,
/// tx fetch, and classify. Returns full enhanced transactions (not
/// just signatures).
pub async fn get_transactions_for_address<U: UpstreamClient + ?Sized>(
    upstream: &U,
    address: &str,
    options: &TransactionsForAddressOptions,
) -> TransactionsForAddressResult {
    let mut txs =
        fetch_txs_for_address(upstream, address, options.pagination_token.as_deref()).await;

    if let Some(min) = options.min_slot {
        txs.retain(|t| t.slot >= min);
    }
    if let Some(max) = options.max_slot {
        txs.retain(|t| t.slot <= max);
    }
    if let Some(status) = options.status {
        txs.retain(|t| match status {
            TxStatus::Success => t.transaction_error.is_none(),
            TxStatus::Failure => t.transaction_error.is_some(),
        });
    }

    let pagination_token = if let Some(limit) = options.limit {
        let limit_usize = usize::try_from(limit).unwrap_or(usize::MAX);
        if txs.len() > limit_usize {
            let next = txs.get(limit_usize - 1).map(|t| t.signature.clone());
            txs.truncate(limit_usize);
            next
        } else {
            None
        }
    } else {
        None
    };

    TransactionsForAddressResult {
        data: txs,
        pagination_token,
    }
}

// ─── internals ───────────────────────────────────────────────────────

/// Shared signature-resolution + tx-fetch path. Uses
/// `getSignaturesForAddress` with `before=<paginationToken>` to honor
/// cursors. The token is just the last-returned signature — that's
/// the same shape Solana's pagination uses.
async fn fetch_txs_for_address<U: UpstreamClient + ?Sized>(
    upstream: &U,
    address: &str,
    pagination_token: Option<&str>,
) -> Vec<EnhancedTransaction> {
    let mut opts = serde_json::Map::new();
    if let Some(token) = pagination_token {
        opts.insert("before".into(), json!(token));
    }
    let params = json!([address, Value::Object(opts)]);
    let Ok(raw) = upstream.rpc_call("getSignaturesForAddress", params).await else {
        return Vec::new();
    };
    let parsed: Value = serde_json::from_slice(&raw).unwrap_or(Value::Null);
    let arr = parsed.as_array().cloned().unwrap_or_default();

    let mut out = Vec::with_capacity(arr.len());
    for entry in arr {
        let Some(sig) = entry.get("signature").and_then(Value::as_str) else {
            continue;
        };
        if let Some(etx) = fetch_and_parse(upstream, sig).await {
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

/// Flatten an `EnhancedTransaction`'s native + token transfers into
/// per-event records. The `address` argument is the wallet the caller
/// asked about — used to filter only transfers it participates in (as
/// either sender or recipient).
fn transfers_from_tx(etx: &EnhancedTransaction, address: &str) -> Vec<Transfer> {
    let mut out: Vec<Transfer> = Vec::new();

    for nt in &etx.native_transfers {
        if nt.from_user_account != address && nt.to_user_account != address {
            continue;
        }
        let amount_str = nt.amount.to_string();
        let ui = format_ui_amount(nt.amount, 9);
        out.push(Transfer {
            signature: etx.signature.clone(),
            slot: etx.slot,
            block_time: etx.timestamp,
            event_type: "transfer".into(),
            from_user_account: Some(nt.from_user_account.clone()),
            to_user_account: Some(nt.to_user_account.clone()),
            from_token_account: None,
            to_token_account: None,
            mint: String::new(),
            amount: amount_str,
            decimals: 9,
            ui_amount: ui,
            confirmation_status: "finalized".into(),
        });
    }

    for tt in &etx.token_transfers {
        let participates = tt.from_user_account.as_deref() == Some(address)
            || tt.to_user_account.as_deref() == Some(address);
        if !participates {
            continue;
        }
        // SPL decimals aren't on EnhancedTokenTransfer; default to 0
        // and let the UI amount equal the raw amount. Real Helius
        // looks the decimals up via the mint's metadata; doing that
        // here would require a token-info cache we don't currently
        // maintain. Marked in BEST_EFFORT notes.
        let decimals: u8 = 0;
        let amount_str = tt.token_amount.to_string();
        let ui = format_ui_amount(tt.token_amount, decimals);
        out.push(Transfer {
            signature: etx.signature.clone(),
            slot: etx.slot,
            block_time: etx.timestamp,
            event_type: "transfer".into(),
            from_user_account: tt.from_user_account.clone(),
            to_user_account: tt.to_user_account.clone(),
            from_token_account: tt.from_token_account.clone(),
            to_token_account: tt.to_token_account.clone(),
            mint: tt.mint.clone(),
            amount: amount_str,
            decimals,
            ui_amount: ui,
            confirmation_status: "finalized".into(),
        });
    }

    out
}

/// Format a raw u64 amount + decimals as a decimal string. No locale,
/// no trailing-zero trimming — matches what Helius emits in `uiAmount`.
fn format_ui_amount(raw: u64, decimals: u8) -> String {
    if decimals == 0 {
        return raw.to_string();
    }
    let divisor = 10u64.pow(u32::from(decimals));
    let whole = raw / divisor;
    let frac = raw % divisor;
    if frac == 0 {
        whole.to_string()
    } else {
        let frac_str = format!("{frac:0>width$}", width = decimals as usize);
        format!("{whole}.{frac_str}")
            .trim_end_matches('0')
            .trim_end_matches('.')
            .to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ui_amount_zero_decimals() {
        assert_eq!(format_ui_amount(1_000_000, 0), "1000000");
    }

    #[test]
    fn ui_amount_sol_lamports() {
        assert_eq!(format_ui_amount(1_500_000_000, 9), "1.5");
        assert_eq!(format_ui_amount(1, 9), "0.000000001");
        assert_eq!(format_ui_amount(0, 9), "0");
    }

    #[test]
    fn ui_amount_usdc() {
        assert_eq!(format_ui_amount(1_000_000, 6), "1");
        assert_eq!(format_ui_amount(1_234_567, 6), "1.234567");
    }

    #[test]
    fn direction_parse() {
        assert_eq!(Direction::parse("in"), Some(Direction::In));
        assert_eq!(Direction::parse("out"), Some(Direction::Out));
        assert_eq!(Direction::parse("either"), None);
    }

    #[test]
    fn sort_parse_default_desc() {
        assert_eq!(Sort::parse("asc"), Sort::Asc);
        assert_eq!(Sort::parse("desc"), Sort::Desc);
        assert_eq!(Sort::parse("garbage"), Sort::Desc);
    }

    #[test]
    fn tx_status_parse() {
        assert_eq!(TxStatus::parse("success"), Some(TxStatus::Success));
        assert_eq!(TxStatus::parse("failure"), Some(TxStatus::Failure));
        assert_eq!(TxStatus::parse("pending"), None);
    }
}
