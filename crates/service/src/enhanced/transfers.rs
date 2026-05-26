//! Native-SOL + SPL-Token transfer extraction via pre/post balance
//! diffs. No instruction-level parsing — just balance accounting.
//!
//! Limitations:
//! - Can't distinguish self-pays (same account on both sides) from
//!   noops: we skip zero-net entries either way.
//! - When multiple transfers between the same pair happen in one tx,
//!   we collapse them into a single net-transfer line.
//! - Token transfers use `preTokenBalances` / `postTokenBalances` which
//!   only cover accounts the tx touched. Accounts the tx opens or
//!   closes during the tx may have partial coverage.

use serde::Deserialize;
use serde_json::Value;

use super::types::{EnhancedNativeTransfer, EnhancedTokenTransfer};

#[derive(Debug, Deserialize)]
struct RawTokenBalance {
    #[serde(rename = "accountIndex")]
    account_index: u32,
    mint: String,
    #[serde(default)]
    owner: Option<String>,
    #[serde(rename = "uiTokenAmount")]
    ui_token_amount: UiTokenAmount,
}

#[derive(Debug, Deserialize)]
struct UiTokenAmount {
    amount: String,
}

/// Net native-SOL transfers, derived from `preBalances` vs
/// `postBalances`. Fee is borne by the fee payer and does NOT appear
/// as a transfer (matches Helius's convention — fee is surfaced
/// separately in the envelope).
#[must_use]
pub fn extract_native_transfers(
    account_keys: &[String],
    pre_balances: &[u64],
    post_balances: &[u64],
    fee: u64,
    fee_payer_index: usize,
) -> Vec<EnhancedNativeTransfer> {
    // Per-account net lamport change. Fee payer's net is post - pre +
    // fee so the diff reflects transfers only, not the fee outflow.
    let n = account_keys
        .len()
        .min(pre_balances.len())
        .min(post_balances.len());
    let mut deltas: Vec<i128> = Vec::with_capacity(n);
    for i in 0..n {
        let pre = i128::from(pre_balances[i]);
        let post = i128::from(post_balances[i]);
        let mut delta = post - pre;
        if i == fee_payer_index {
            delta += i128::from(fee);
        }
        deltas.push(delta);
    }

    // Greedy matching: pair the largest net-negative (sender) with the
    // largest net-positive (receiver), subtract, repeat. Doesn't
    // attempt to match specific lamport amounts — Solana's tx shape
    // doesn't expose which sender funded which receiver anyway.
    let mut senders: Vec<(usize, i128)> = deltas
        .iter()
        .enumerate()
        .filter(|(_, d)| **d < 0)
        .map(|(i, d)| (i, -*d))
        .collect();
    let mut receivers: Vec<(usize, i128)> = deltas
        .iter()
        .enumerate()
        .filter(|(_, d)| **d > 0)
        .map(|(i, d)| (i, *d))
        .collect();

    senders.sort_by_key(|s| std::cmp::Reverse(s.1));
    receivers.sort_by_key(|r| std::cmp::Reverse(r.1));

    let mut out = Vec::new();
    let mut si = 0;
    let mut ri = 0;
    while si < senders.len() && ri < receivers.len() {
        let amount = senders[si].1.min(receivers[ri].1);
        if amount > 0 {
            out.push(EnhancedNativeTransfer {
                from_user_account: account_keys[senders[si].0].clone(),
                to_user_account: account_keys[receivers[ri].0].clone(),
                amount: u64::try_from(amount).unwrap_or(0),
            });
        }
        senders[si].1 -= amount;
        receivers[ri].1 -= amount;
        if senders[si].1 == 0 {
            si += 1;
        }
        if receivers[ri].1 == 0 {
            ri += 1;
        }
    }
    out
}

/// Net SPL-Token transfers, grouped by (mint, owner). Uses the
/// jsonParsed `preTokenBalances` / `postTokenBalances` arrays — these
/// only cover token accounts the tx touched. Works for both SPL Token
/// and Token-2022 (same envelope shape).
#[must_use]
pub fn extract_token_transfers(
    account_keys: &[String],
    pre: &Value,
    post: &Value,
) -> Vec<EnhancedTokenTransfer> {
    let pre_entries: Vec<RawTokenBalance> = pre
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|v| serde_json::from_value(v.clone()).ok())
                .collect()
        })
        .unwrap_or_default();
    let post_entries: Vec<RawTokenBalance> = post
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|v| serde_json::from_value(v.clone()).ok())
                .collect()
        })
        .unwrap_or_default();

    // Compute per-account amount deltas (post - pre). Keyed by
    // account_index so we can look up the pubkey at the end.
    let mut deltas: std::collections::BTreeMap<u32, (String, Option<String>, i128)> =
        std::collections::BTreeMap::new();
    for b in &pre_entries {
        let amt: i128 = b.ui_token_amount.amount.parse::<i128>().unwrap_or(0);
        let entry = deltas
            .entry(b.account_index)
            .or_insert((b.mint.clone(), b.owner.clone(), 0));
        entry.2 -= amt;
    }
    for b in &post_entries {
        let amt: i128 = b.ui_token_amount.amount.parse::<i128>().unwrap_or(0);
        let entry = deltas
            .entry(b.account_index)
            .or_insert((b.mint.clone(), b.owner.clone(), 0));
        entry.2 += amt;
    }

    // Group by mint, then greedy-match senders → receivers.
    let mut by_mint: std::collections::BTreeMap<String, Vec<(u32, Option<String>, i128)>> =
        std::collections::BTreeMap::new();
    for (idx, (mint, owner, delta)) in deltas {
        if delta == 0 {
            continue;
        }
        by_mint.entry(mint).or_default().push((idx, owner, delta));
    }

    let mut out = Vec::new();
    for (mint, mut entries) in by_mint {
        let (mut senders, mut receivers): (Vec<_>, Vec<_>) =
            entries.drain(..).partition(|e| e.2 < 0);
        // Senders' amounts are negative — flip for consistency.
        for e in &mut senders {
            e.2 = -e.2;
        }
        senders.sort_by_key(|s| std::cmp::Reverse(s.2));
        receivers.sort_by_key(|r| std::cmp::Reverse(r.2));
        let mut si = 0;
        let mut ri = 0;
        while si < senders.len() && ri < receivers.len() {
            let amount = senders[si].2.min(receivers[ri].2);
            if amount > 0 {
                let from_token_account = account_keys.get(senders[si].0 as usize).cloned();
                let to_token_account = account_keys.get(receivers[ri].0 as usize).cloned();
                out.push(EnhancedTokenTransfer {
                    from_user_account: senders[si].1.clone(),
                    to_user_account: receivers[ri].1.clone(),
                    from_token_account,
                    to_token_account,
                    mint: mint.clone(),
                    token_amount: u64::try_from(amount).unwrap_or(0),
                    token_standard: None,
                });
            }
            senders[si].2 -= amount;
            receivers[ri].2 -= amount;
            if senders[si].2 == 0 {
                si += 1;
            }
            if receivers[ri].2 == 0 {
                ri += 1;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn native_transfer_from_single_sender_to_receiver() {
        let keys = vec!["A".into(), "B".into(), "C".into()];
        let pre = vec![10_000_000, 1_000_000, 500_000];
        let post = vec![9_000_000, 2_000_000, 500_000];
        let fee = 0;
        let got = extract_native_transfers(&keys, &pre, &post, fee, 0);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].from_user_account, "A");
        assert_eq!(got[0].to_user_account, "B");
        assert_eq!(got[0].amount, 1_000_000);
    }

    #[test]
    fn native_transfer_excludes_fee_from_fee_payer_delta() {
        let keys = vec!["A".into(), "B".into()];
        // A pays 100k lamports to B + 5k fee.
        let pre = vec![200_000, 50_000];
        let post = vec![95_000, 150_000];
        let got = extract_native_transfers(&keys, &pre, &post, 5_000, 0);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].amount, 100_000, "transfer net of fee");
    }

    #[test]
    fn native_transfer_returns_empty_when_no_net_change() {
        let keys = vec!["A".into(), "B".into()];
        let pre = vec![100, 100];
        let post = vec![100, 100];
        assert!(extract_native_transfers(&keys, &pre, &post, 0, 0).is_empty());
    }

    #[test]
    fn token_transfer_derived_from_pre_post_diff() {
        let keys = vec!["ATA_A".into(), "ATA_B".into()];
        let pre = json!([
            { "accountIndex": 0, "mint": "M", "owner": "OWNER_A", "uiTokenAmount": { "amount": "1000", "decimals": 0, "uiAmount": 1000.0, "uiAmountString": "1000" } },
            { "accountIndex": 1, "mint": "M", "owner": "OWNER_B", "uiTokenAmount": { "amount": "0", "decimals": 0, "uiAmount": 0.0, "uiAmountString": "0" } },
        ]);
        let post = json!([
            { "accountIndex": 0, "mint": "M", "owner": "OWNER_A", "uiTokenAmount": { "amount": "700", "decimals": 0, "uiAmount": 700.0, "uiAmountString": "700" } },
            { "accountIndex": 1, "mint": "M", "owner": "OWNER_B", "uiTokenAmount": { "amount": "300", "decimals": 0, "uiAmount": 300.0, "uiAmountString": "300" } },
        ]);
        let got = extract_token_transfers(&keys, &pre, &post);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].from_user_account.as_deref(), Some("OWNER_A"));
        assert_eq!(got[0].to_user_account.as_deref(), Some("OWNER_B"));
        assert_eq!(got[0].from_token_account.as_deref(), Some("ATA_A"));
        assert_eq!(got[0].to_token_account.as_deref(), Some("ATA_B"));
        assert_eq!(got[0].token_amount, 300);
        assert_eq!(got[0].mint, "M");
    }

    #[test]
    fn token_transfer_empty_on_no_delta() {
        let got = extract_token_transfers(&[], &Value::Null, &Value::Null);
        assert!(got.is_empty());
    }
}
