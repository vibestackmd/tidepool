//! Webhook domain types — what a webhook looks like on the wire and
//! in our store.

use serde::{Deserialize, Serialize};

/// One registered webhook. `id` is Tidepool-assigned (a short
/// UUID-like string — nothing Helius-specific). `account_addresses`
/// drive the polling loop; the rest is stored verbatim so GETs round-
/// trip user input.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Webhook {
    /// Helius uses `webhookID` in its JSON; preserve the casing for
    /// wire compat.
    #[serde(rename = "webhookID")]
    pub webhook_id: String,
    #[serde(rename = "webhookURL")]
    pub webhook_url: String,
    pub account_addresses: Vec<String>,
    /// Event-type filter strings (e.g. `"NFT_SALE"`). Stored but not
    /// applied in v1 — delivery fires on every signature.
    #[serde(default)]
    pub transaction_types: Vec<String>,
    /// `"all"` | `"success"` | `"failed"` — used to filter deliveries.
    /// Defaults to `"all"` when omitted.
    #[serde(default)]
    pub txn_status: Option<String>,
    /// `"enhanced"` | `"raw"`; we treat both the same (raw delivery).
    #[serde(default)]
    pub webhook_type: Option<String>,
    /// Optional auth header Helius attaches to delivery requests. We
    /// echo it back on delivery (set as `Authorization: <value>`) but
    /// don't validate format.
    #[serde(default)]
    pub auth_header: Option<String>,
}

/// User input for create / edit. Omitted fields on edit keep prior
/// values (in the handler layer).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct WebhookInput {
    #[serde(rename = "webhookURL")]
    pub webhook_url: Option<String>,
    pub account_addresses: Option<Vec<String>>,
    #[serde(default)]
    pub transaction_types: Vec<String>,
    #[serde(default)]
    pub txn_status: Option<String>,
    #[serde(default)]
    pub webhook_type: Option<String>,
    #[serde(default)]
    pub auth_header: Option<String>,
}

/// Minimal delivery envelope. One of these per signature we observe.
/// The Enhanced Transactions parser (P3) will upgrade this shape to
/// match Helius's richer enhanced-tx payload — until then we send the
/// fields we can reliably produce from a `getTransaction` response.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WebhookEvent {
    pub signature: String,
    pub slot: u64,
    /// Block time (unix seconds) when known; absent on early-slot or
    /// very-recent txs the upstream hasn't backfilled.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<i64>,
    /// `Some(serde_json::Value)` when the tx failed on-chain; `None`
    /// on success.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub err: Option<serde_json::Value>,
    /// The address(es) from our registered list that caused this event
    /// to fire. Useful when a webhook listens to multiple addresses.
    pub account_addresses: Vec<String>,
}
