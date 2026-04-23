//! Parse one `getTransaction` response → `EnhancedTransaction`. The
//! top-level glue: pull fields, run classifier, run transfer
//! extraction.

use serde_json::Value;

use super::classify::{classify, instruction_views, RawInstruction};
use super::events::derive_nft_event;
use super::transfers::{extract_native_transfers, extract_token_transfers};
use super::types::{AccountData, EnhancedEvents, EnhancedInstruction, EnhancedTransaction};

/// Parse a `getTransaction` JSON response (the `result` field, which
/// itself has `{slot, blockTime, transaction, meta}`). Returns `None`
/// when the envelope is malformed enough that we can't produce a
/// signature + slot.
#[must_use]
pub fn parse_enhanced_tx(signature: &str, tx: &Value) -> Option<EnhancedTransaction> {
    let slot = tx.get("slot").and_then(Value::as_u64)?;
    let timestamp = tx.get("blockTime").and_then(Value::as_i64);

    let meta = tx.get("meta").cloned().unwrap_or(Value::Null);
    let fee = meta.get("fee").and_then(Value::as_u64).unwrap_or(0);
    let transaction_error = meta.get("err").cloned().filter(|v| !v.is_null());

    let message = tx
        .pointer("/transaction/message")
        .cloned()
        .unwrap_or(Value::Null);
    let account_keys: Vec<String> = message
        .get("accountKeys")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    let fee_payer = account_keys.first().cloned().unwrap_or_default();

    // Decode outer instructions for classification + for the
    // EnhancedInstruction envelope (accounts resolved to pubkeys).
    let raw_ixs: Vec<RawInstruction> = message
        .get("instructions")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|v| serde_json::from_value(v.clone()).ok())
                .collect()
        })
        .unwrap_or_default();

    let views = instruction_views(&account_keys, &raw_ixs);
    let class = classify(&views);

    // Transfer extraction uses meta balance arrays.
    let pre_balances: Vec<u64> = meta
        .get("preBalances")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_u64).collect())
        .unwrap_or_default();
    let post_balances: Vec<u64> = meta
        .get("postBalances")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_u64).collect())
        .unwrap_or_default();
    let native_transfers = extract_native_transfers(
        &account_keys,
        &pre_balances,
        &post_balances,
        fee,
        0, // fee payer is account_keys[0] per tx format
    );

    let pre_tok = meta.get("preTokenBalances").cloned().unwrap_or(Value::Null);
    let post_tok = meta
        .get("postTokenBalances")
        .cloned()
        .unwrap_or(Value::Null);
    let token_transfers = extract_token_transfers(&account_keys, &pre_tok, &post_tok);

    // Build enhanced ix shape with inner ixs grouped by outer index.
    let inner_ix_groups: std::collections::BTreeMap<u32, Vec<RawInstruction>> = meta
        .get("innerInstructions")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|grp| {
                    let idx = grp.get("index").and_then(Value::as_u64)? as u32;
                    let ixs: Vec<RawInstruction> = grp
                        .get("instructions")
                        .and_then(Value::as_array)
                        .map(|a| {
                            a.iter()
                                .filter_map(|v| serde_json::from_value(v.clone()).ok())
                                .collect()
                        })
                        .unwrap_or_default();
                    Some((idx, ixs))
                })
                .collect()
        })
        .unwrap_or_default();

    let instructions = raw_ixs
        .iter()
        .enumerate()
        .map(|(idx, ix)| {
            let inner = inner_ix_groups
                .get(&(idx as u32))
                .cloned()
                .unwrap_or_default();
            build_enhanced_ix(&account_keys, ix, &inner)
        })
        .collect();

    // Per-account balance deltas. Helius emits one entry per account
    // in `accountKeys`, including ones with no change (change == 0) —
    // so downstream indexers can reconstruct the account list from
    // this field alone. Token-balance-changes isn't populated by our
    // classifier yet; we emit an empty array to keep the key present.
    let account_data = build_account_data(&account_keys, &pre_balances, &post_balances);

    // Two-pass build: we need the `EnhancedTransaction` (minus
    // `events`) to feed to `derive_nft_event`, since that helper reads
    // `tx_type`, `source`, and both transfer lists to derive buyer /
    // seller / nfts. So assemble the envelope first with default
    // events, then fill it in.
    let mut out = EnhancedTransaction {
        signature: signature.to_string(),
        slot,
        timestamp,
        tx_type: class.tx_type.to_string(),
        source: class.source.to_string(),
        fee,
        fee_payer,
        description: class.description,
        native_transfers,
        token_transfers,
        instructions,
        account_data,
        events: EnhancedEvents::default(),
        lighthouse_data: None,
        transaction_error,
    };
    out.events.nft = derive_nft_event(&out);
    Some(out)
}

/// Build per-account balance deltas from pre/postBalances. One entry
/// per account in `accountKeys`, in the same order — including accounts
/// with no change (delta == 0) so clients can use this as the
/// authoritative participant list without re-reading `accountKeys`.
fn build_account_data(
    account_keys: &[String],
    pre_balances: &[u64],
    post_balances: &[u64],
) -> Vec<AccountData> {
    account_keys
        .iter()
        .enumerate()
        .map(|(i, account)| {
            let pre = pre_balances.get(i).copied().unwrap_or(0);
            let post = post_balances.get(i).copied().unwrap_or(0);
            // Cast through i128 so the signed diff can't overflow
            // either direction of a u64 before we narrow to i64.
            let delta = i128::from(post) - i128::from(pre);
            AccountData {
                account: account.clone(),
                native_balance_change: i64::try_from(delta).unwrap_or(0),
                token_balance_changes: Vec::new(),
            }
        })
        .collect()
}

fn build_enhanced_ix(
    account_keys: &[String],
    ix: &RawInstruction,
    inner: &[RawInstruction],
) -> EnhancedInstruction {
    let program_id = account_keys
        .get(ix.program_id_index as usize)
        .cloned()
        .unwrap_or_default();
    let accounts: Vec<String> = ix
        .accounts
        .iter()
        .filter_map(|idx| account_keys.get(*idx as usize).cloned())
        .collect();
    let inner_instructions: Vec<EnhancedInstruction> = inner
        .iter()
        .map(|i| build_enhanced_ix(account_keys, i, &[]))
        .collect();
    EnhancedInstruction {
        program_id,
        accounts,
        data: ix.data.clone(),
        inner_instructions,
    }
}

/// Filter a list of signatures down to those whose parsed tx
/// classification has a non-UNKNOWN `type` (useful for the
/// `getTransactionsByAddress` fan-out when a caller wants to skip
/// the noise). The service-layer handler uses this to pare the
/// return list in cases where a downstream client asked for a
/// specific type only — we just compute and filter here rather than
/// supporting type filters end-to-end, since Helius's own behavior
/// is to fan-out-then-filter anyway.
#[must_use]
pub fn signatures_matching<'a>(
    sigs: impl IntoIterator<Item = &'a str>,
    types_allowed: &[&str],
    resolver: impl Fn(&str) -> Option<EnhancedTransaction>,
) -> Vec<EnhancedTransaction> {
    sigs.into_iter()
        .filter_map(resolver)
        .filter(|etx| types_allowed.is_empty() || types_allowed.contains(&etx.tx_type.as_str()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parse_full_tx_native_transfer_shape() {
        let tx = json!({
            "slot": 100,
            "blockTime": 1_700_000_000,
            "transaction": {
                "message": {
                    "accountKeys": ["A", "B", "11111111111111111111111111111111"],
                    "instructions": [
                        {
                            "programIdIndex": 2,
                            "accounts": [0, 1],
                            "data": "3Bxs3zyH4nXi" // arbitrary base58 body
                        }
                    ]
                }
            },
            "meta": {
                "fee": 5000,
                "err": null,
                "preBalances": [1_000_000, 0, 1],
                "postBalances": [494_000, 500_000, 1],
                "innerInstructions": []
            }
        });
        let out = parse_enhanced_tx("SIG1", &tx).expect("Some");
        assert_eq!(out.signature, "SIG1");
        assert_eq!(out.slot, 100);
        assert_eq!(out.timestamp, Some(1_700_000_000));
        assert_eq!(out.fee, 5000);
        assert_eq!(out.fee_payer, "A");
        assert_eq!(out.tx_type, "TRANSFER");
        assert_eq!(out.source, "SYSTEM_PROGRAM");
        assert_eq!(out.native_transfers.len(), 1);
        assert_eq!(out.native_transfers[0].amount, 500_000);
        assert!(out.token_transfers.is_empty());
    }

    #[test]
    fn parse_missing_slot_returns_none() {
        let tx = json!({ "transaction": {}, "meta": {} });
        assert!(parse_enhanced_tx("SIG", &tx).is_none());
    }

    #[test]
    fn parse_with_err_sets_transaction_error() {
        let tx = json!({
            "slot": 1,
            "transaction": { "message": { "accountKeys": [], "instructions": [] } },
            "meta": { "fee": 0, "err": { "InstructionError": [0, "Custom"] } }
        });
        let out = parse_enhanced_tx("S", &tx).unwrap();
        assert!(out.transaction_error.is_some());
    }
}
