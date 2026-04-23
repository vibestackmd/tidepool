//! Webhook delivery tests. Drive `tick_once` against a fixture
//! upstream (providing getSignaturesForAddress + getTransaction) and
//! a recording `PostClient` — no real HTTP, no timers, deterministic.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::sync::Mutex;

use tidepool_rpc::upstream::FixtureUpstream;
use tidepool_rpc::webhooks::{tick_once, PostClient, Webhook};

type RecordedCall = (String, Option<String>, Value);

struct RecordingPoster {
    calls: Arc<Mutex<Vec<RecordedCall>>>,
}

impl RecordingPoster {
    fn new() -> Self {
        Self {
            calls: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

#[async_trait]
impl PostClient for RecordingPoster {
    async fn post_json(
        &self,
        url: &str,
        auth: Option<&str>,
        body: &Value,
    ) -> Result<(), String> {
        self.calls
            .lock()
            .await
            .push((url.to_string(), auth.map(String::from), body.clone()));
        Ok(())
    }
}

fn webhook(addresses: &[&str]) -> Webhook {
    Webhook {
        webhook_id: "wh_1".into(),
        webhook_url: "https://example.com/hook".into(),
        account_addresses: addresses.iter().map(|s| (*s).to_string()).collect(),
        transaction_types: vec![],
        txn_status: None,
        webhook_type: None,
        auth_header: None,
    }
}

fn sig_entry(signature: &str, slot: u64, err: &Value, block_time: i64) -> Value {
    json!({
        "signature": signature,
        "slot": slot,
        "err": err,
        "blockTime": block_time
    })
}

/// Minimal getTransaction JSON payload shaped for the enhanced
/// parser. Uses the System Program so the classifier labels it as a
/// plain TRANSFER.
fn tx_payload(slot: u64, block_time: i64, err: &Value) -> Value {
    json!({
        "slot": slot,
        "blockTime": block_time,
        "transaction": {
            "message": {
                "accountKeys": ["PAYER", "RCPT", "11111111111111111111111111111111"],
                "instructions": [
                    { "programIdIndex": 2, "accounts": [0, 1], "data": "3Bxs" }
                ]
            }
        },
        "meta": {
            "fee": 5000,
            "err": err,
            "preBalances": [1_000_000, 0, 1],
            "postBalances": [494_000, 500_000, 1],
            "innerInstructions": []
        }
    })
}

#[tokio::test]
async fn tick_once_delivers_enhanced_tx_per_fresh_signature() {
    let upstream = FixtureUpstream::new()
        .with_method("getSignaturesForAddress", |_params| {
            // Solana returns newest-first; `tick_once` reverses for delivery.
            Ok(json!([
                sig_entry("SIG3", 103, &Value::Null, 1_700_000_030),
                sig_entry("SIG2", 102, &Value::Null, 1_700_000_020),
                sig_entry("SIG1", 101, &Value::Null, 1_700_000_010),
            ]))
        })
        .with_method("getTransaction", |params| {
            let sig = params
                .get(0)
                .and_then(Value::as_str)
                .unwrap_or("");
            // Vary slot per signature so events are distinguishable.
            let slot = match sig {
                "SIG1" => 101,
                "SIG2" => 102,
                _ => 103,
            };
            let time = match sig {
                "SIG1" => 1_700_000_010,
                "SIG2" => 1_700_000_020,
                _ => 1_700_000_030,
            };
            Ok(tx_payload(slot, time, &Value::Null))
        });
    let poster = RecordingPoster::new();
    let cursors: Mutex<HashMap<String, String>> = Mutex::new(HashMap::new());

    let delivered = tick_once(&webhook(&["ADDR"]), &upstream, &poster, &cursors).await;

    // Three enhanced transactions, oldest-first.
    assert_eq!(delivered.len(), 3);
    assert_eq!(delivered[0].signature, "SIG1");
    assert_eq!(delivered[2].signature, "SIG3");
    assert_eq!(delivered[0].slot, 101);
    assert_eq!(delivered[0].tx_type, "TRANSFER");
    assert_eq!(delivered[0].source, "SYSTEM_PROGRAM");

    let calls = poster.calls.lock().await;
    assert_eq!(calls.len(), 1);
    let body = calls[0].2.as_array().expect("array body");
    assert_eq!(body.len(), 3);

    let c = cursors.lock().await;
    assert_eq!(c.get("ADDR").cloned(), Some("SIG3".to_string()));
}

#[tokio::test]
async fn tick_once_skips_delivery_when_no_new_signatures() {
    let upstream = FixtureUpstream::new()
        .with_method("getSignaturesForAddress", |_| Ok(json!([])));
    let poster = RecordingPoster::new();
    let cursors: Mutex<HashMap<String, String>> = Mutex::new(HashMap::new());
    let delivered = tick_once(&webhook(&["ADDR"]), &upstream, &poster, &cursors).await;
    assert!(delivered.is_empty());
    assert!(poster.calls.lock().await.is_empty());
}

#[tokio::test]
async fn tick_once_respects_txn_status_failed_filter() {
    let upstream = FixtureUpstream::new()
        .with_method("getSignaturesForAddress", |_| {
            Ok(json!([
                sig_entry("OK_SIG", 100, &Value::Null, 1_700_000_000),
                sig_entry(
                    "ERR_SIG",
                    99,
                    &json!({ "InstructionError": [0, "Custom"] }),
                    1_699_999_900,
                ),
            ]))
        })
        .with_method("getTransaction", |params| {
            let sig = params.get(0).and_then(Value::as_str).unwrap_or("");
            let err = if sig == "ERR_SIG" {
                json!({ "InstructionError": [0, "Custom"] })
            } else {
                Value::Null
            };
            Ok(tx_payload(if sig == "ERR_SIG" { 99 } else { 100 }, 0, &err))
        });
    let poster = RecordingPoster::new();
    let mut wh = webhook(&["ADDR"]);
    wh.txn_status = Some("failed".into());
    let cursors: Mutex<HashMap<String, String>> = Mutex::new(HashMap::new());

    let delivered = tick_once(&wh, &upstream, &poster, &cursors).await;
    assert_eq!(delivered.len(), 1);
    assert_eq!(delivered[0].signature, "ERR_SIG");
    assert!(delivered[0].transaction_error.is_some());
}

#[tokio::test]
async fn tick_once_respects_transaction_types_filter() {
    // Two signatures, one parses as TRANSFER, one as UNKNOWN (empty
    // ix list). Webhook subscribes only to UNKNOWN; should drop the
    // TRANSFER.
    let upstream = FixtureUpstream::new()
        .with_method("getSignaturesForAddress", |_| {
            Ok(json!([
                sig_entry("UNK_SIG", 100, &Value::Null, 1_700_000_000),
                sig_entry("TX_SIG", 99, &Value::Null, 1_699_999_900),
            ]))
        })
        .with_method("getTransaction", |params| {
            let sig = params.get(0).and_then(Value::as_str).unwrap_or("");
            if sig == "UNK_SIG" {
                Ok(json!({
                    "slot": 100,
                    "blockTime": 1_700_000_000,
                    "transaction": {
                        "message": { "accountKeys": [], "instructions": [] }
                    },
                    "meta": { "fee": 0, "err": null }
                }))
            } else {
                Ok(tx_payload(99, 0, &Value::Null))
            }
        });
    let poster = RecordingPoster::new();
    let mut wh = webhook(&["ADDR"]);
    wh.transaction_types = vec!["UNKNOWN".into()];
    let cursors: Mutex<HashMap<String, String>> = Mutex::new(HashMap::new());

    let delivered = tick_once(&wh, &upstream, &poster, &cursors).await;
    assert_eq!(delivered.len(), 1);
    assert_eq!(delivered[0].signature, "UNK_SIG");
    assert_eq!(delivered[0].tx_type, "UNKNOWN");
}

#[tokio::test]
async fn tick_once_aggregates_across_multiple_addresses() {
    let upstream = FixtureUpstream::new()
        .with_method("getSignaturesForAddress", |params| {
            let addr = params.get(0).and_then(Value::as_str).unwrap_or("");
            if addr == "A" {
                Ok(json!([sig_entry("A_SIG", 100, &Value::Null, 1_700_000_000)]))
            } else {
                Ok(json!([sig_entry("B_SIG", 100, &Value::Null, 1_700_000_000)]))
            }
        })
        .with_method("getTransaction", |params| {
            let sig = params.get(0).and_then(Value::as_str).unwrap_or("");
            let slot = if sig == "A_SIG" { 100 } else { 101 };
            Ok(tx_payload(slot, 1_700_000_000, &Value::Null))
        });
    let poster = RecordingPoster::new();
    let cursors: Mutex<HashMap<String, String>> = Mutex::new(HashMap::new());
    let delivered = tick_once(&webhook(&["A", "B"]), &upstream, &poster, &cursors).await;
    assert_eq!(delivered.len(), 2);
    let calls = poster.calls.lock().await;
    assert_eq!(calls.len(), 1);
    let body = calls[0].2.as_array().expect("array");
    assert_eq!(body.len(), 2);
}
