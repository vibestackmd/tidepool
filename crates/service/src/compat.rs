//! Compatibility manifest — structured metadata about every Helius
//! method, with a documented compat level so consumers know what's
//! genuinely 1:1 vs what's best-effort local simulation.
//!
//! Served by the `surfpoolHeliusInfo` RPC. Also usable as a plain
//! Rust constant for tooling that wants to diff against upstream
//! Helius SDK changes at build time.

use serde::{Deserialize, Serialize};

/// Fidelity classification, matches the TS version one-for-one so
/// clients that parse the manifest work against both.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CompatLevel {
    /// Byte-for-byte Helius compatibility. Pure account read or pure
    /// local computation; no state we don't own.
    Exact,
    /// Serves from local state we've observed. Queries about
    /// untouched addresses return empty.
    LocalIndex,
    /// Reduced fidelity vs real Helius. Documented in `notes`.
    BestEffort,
    /// Behaviorally close but mechanically different (e.g. HTTP
    /// polling instead of push WebSocket subscriptions).
    Shim,
    /// A helius-sdk client-side helper that composes wire methods.
    /// We don't intercept — the SDK runs in-process in the user's
    /// app and its underlying RPC calls pass through unchanged.
    SdkWrapper,
    /// Known method, not yet implemented.
    Planned,
    /// Known method, explicitly out of scope (requires cloud
    /// infrastructure we can't reproduce locally).
    Skipped,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Namespace {
    Das,
    Enhanced,
    Tx,
    Rpc,
    Staking,
    Webhooks,
    Ws,
    Wallet,
    Compat,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MethodEntry {
    pub method: &'static str,
    pub namespace: Namespace,
    pub helius_sdk_path: &'static str,
    pub compat: CompatLevel,
    /// Tidepool version this method became available. `None` for
    /// `Planned` / `Skipped` entries.
    pub since_version: Option<&'static str>,
    pub source_doc: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes: Option<&'static str>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default)]
pub struct ManifestSummary {
    pub exact: u32,
    pub local_index: u32,
    pub best_effort: u32,
    pub shim: u32,
    pub sdk_wrapper: u32,
    pub planned: u32,
    pub skipped: u32,
    pub total: u32,
}

#[must_use]
pub fn summarize(entries: &[MethodEntry]) -> ManifestSummary {
    let mut s = ManifestSummary {
        total: entries.len() as u32,
        ..ManifestSummary::default()
    };
    for e in entries {
        match e.compat {
            CompatLevel::Exact => s.exact += 1,
            CompatLevel::LocalIndex => s.local_index += 1,
            CompatLevel::BestEffort => s.best_effort += 1,
            CompatLevel::Shim => s.shim += 1,
            CompatLevel::SdkWrapper => s.sdk_wrapper += 1,
            CompatLevel::Planned => s.planned += 1,
            CompatLevel::Skipped => s.skipped += 1,
        }
    }
    s
}

/// The manifest. Mirrors helius-sdk v2.x namespace structure; kept
/// in sync by hand today, SDK-diff CI is a future task.
#[must_use]
pub fn manifest() -> &'static [MethodEntry] {
    MANIFEST
}

const MANIFEST: &[MethodEntry] = &[
    // ─── das ──────────────────────────────────────────────────────────
    MethodEntry {
        method: "getAsset",
        namespace: Namespace::Das,
        helius_sdk_path: "helius.das.getAsset",
        compat: CompatLevel::LocalIndex,
        since_version: Some("1.0.0"),
        source_doc: "https://www.helius.dev/docs/api-reference/das/getasset",
        notes: Some(
            "cNFTs resolve from the local Bubblegum indexer; uncompressed assets resolve via upstream getAccountInfo + MplCore / Token Metadata decoders.",
        ),
    },
    MethodEntry {
        method: "getAssetBatch",
        namespace: Namespace::Das,
        helius_sdk_path: "helius.das.getAssetBatch",
        compat: CompatLevel::LocalIndex,
        since_version: Some("1.0.0"),
        source_doc: "https://www.helius.dev/docs/api-reference/das",
        notes: Some("Sequential fanout over getAsset; up to 1000 ids."),
    },
    MethodEntry {
        method: "getAssetProof",
        namespace: Namespace::Das,
        helius_sdk_path: "helius.das.getAssetProof",
        compat: CompatLevel::LocalIndex,
        since_version: Some("1.0.0"),
        source_doc: "https://www.helius.dev/docs/api-reference/das/getassetproof",
        notes: Some(
            "cNFT merkle proof computed from the local indexer. Trees must be registered via --index-tree or surfpoolHeliusIndexTree before resolution works.",
        ),
    },
    MethodEntry {
        method: "getAssetProofBatch",
        namespace: Namespace::Das,
        helius_sdk_path: "helius.das.getAssetProofBatch",
        compat: CompatLevel::LocalIndex,
        since_version: Some("1.0.0"),
        source_doc: "https://www.helius.dev/docs/api-reference/das",
        notes: Some("Shares TreeState materialization across ids in the batch."),
    },
    MethodEntry {
        method: "getAssetsByOwner",
        namespace: Namespace::Das,
        helius_sdk_path: "helius.das.getAssetsByOwner",
        compat: CompatLevel::LocalIndex,
        since_version: Some("1.0.0"),
        source_doc: "https://www.helius.dev/docs/api-reference/das/getassetsbyowner",
        notes: Some(
            "Returns assets the proxy has indexed via prior getAsset / getAssetBatch calls. Owners never touched return empty.",
        ),
    },
    MethodEntry {
        method: "getAssetsByAuthority",
        namespace: Namespace::Das,
        helius_sdk_path: "helius.das.getAssetsByAuthority",
        compat: CompatLevel::LocalIndex,
        since_version: Some("1.0.0"),
        source_doc: "https://www.helius.dev/docs/api-reference/das",
        notes: None,
    },
    MethodEntry {
        method: "getAssetsByCreator",
        namespace: Namespace::Das,
        helius_sdk_path: "helius.das.getAssetsByCreator",
        compat: CompatLevel::LocalIndex,
        since_version: Some("1.0.0"),
        source_doc: "https://www.helius.dev/docs/api-reference/das",
        notes: Some("`onlyVerified` filters to creators whose verified flag is true."),
    },
    MethodEntry {
        method: "getAssetsByGroup",
        namespace: Namespace::Das,
        helius_sdk_path: "helius.das.getAssetsByGroup",
        compat: CompatLevel::LocalIndex,
        since_version: Some("1.0.0"),
        source_doc: "https://www.helius.dev/docs/api-reference/das",
        notes: Some("For MplCore: groupKey=\"collection\"."),
    },
    MethodEntry {
        method: "searchAssets",
        namespace: Namespace::Das,
        helius_sdk_path: "helius.das.searchAssets",
        compat: CompatLevel::LocalIndex,
        since_version: Some("1.0.0"),
        source_doc: "https://www.helius.dev/docs/api-reference/das/searchassets",
        notes: Some(
            "AND-combines owner / authority / creator / grouping / interface / burnt filters. Smallest-index-first narrowing keeps multi-filter queries fast.",
        ),
    },
    MethodEntry {
        method: "getNftEditions",
        namespace: Namespace::Das,
        helius_sdk_path: "helius.das.getNftEditions",
        compat: CompatLevel::Planned,
        since_version: None,
        source_doc: "https://www.helius.dev/docs/api-reference/das",
        notes: Some("Needs Token Metadata EditionV1 indexing side-effect on getAsset."),
    },
    MethodEntry {
        method: "getTokenAccounts",
        namespace: Namespace::Das,
        helius_sdk_path: "helius.das.getTokenAccounts",
        compat: CompatLevel::Planned,
        since_version: None,
        source_doc: "https://www.helius.dev/docs/api-reference/das/gettokenaccounts",
        notes: None,
    },
    // ─── enhanced ─────────────────────────────────────────────────────
    MethodEntry {
        method: "getTransactions",
        namespace: Namespace::Enhanced,
        helius_sdk_path: "helius.enhanced.getTransactions",
        compat: CompatLevel::Planned,
        since_version: None,
        source_doc: "https://www.helius.dev/docs/api-reference/enhanced-transactions/gettransactions",
        notes: Some("Needs local tx parsers for NFT_MINT / NFT_SALE / TRANSFER / SWAP / STAKE."),
    },
    MethodEntry {
        method: "getTransactionsByAddress",
        namespace: Namespace::Enhanced,
        helius_sdk_path: "helius.enhanced.getTransactionsByAddress",
        compat: CompatLevel::Planned,
        since_version: None,
        source_doc: "https://www.helius.dev/docs/enhanced-transactions",
        notes: None,
    },
    // ─── tx ───────────────────────────────────────────────────────────
    MethodEntry {
        method: "getPriorityFeeEstimate",
        namespace: Namespace::Tx,
        helius_sdk_path: "helius.tx.getPriorityFeeEstimate",
        compat: CompatLevel::Planned,
        since_version: None,
        source_doc: "https://www.helius.dev/docs/priority-fee-api",
        notes: Some("Local percentile computation over getRecentPrioritizationFees samples."),
    },
    MethodEntry {
        method: "sendSmartTransaction",
        namespace: Namespace::Tx,
        helius_sdk_path: "helius.tx.sendSmartTransaction",
        compat: CompatLevel::SdkWrapper,
        since_version: Some("1.0.0"),
        source_doc: "https://www.helius.dev/docs/sending-transactions/optimizing-transactions",
        notes: Some(
            "helius-sdk client-side composition: simulate → priority fee → getLatestBlockhash → send → confirm. Every underlying call passes through.",
        ),
    },
    MethodEntry {
        method: "broadcastTransaction",
        namespace: Namespace::Tx,
        helius_sdk_path: "helius.tx.broadcastTransaction",
        compat: CompatLevel::SdkWrapper,
        since_version: Some("1.0.0"),
        source_doc: "https://www.helius.dev/docs/sending-transactions",
        notes: None,
    },
    MethodEntry {
        method: "pollTransactionConfirmation",
        namespace: Namespace::Tx,
        helius_sdk_path: "helius.tx.pollTransactionConfirmation",
        compat: CompatLevel::SdkWrapper,
        since_version: Some("1.0.0"),
        source_doc: "https://www.helius.dev/docs/sending-transactions",
        notes: None,
    },
    MethodEntry {
        method: "sendTransactionWithSender",
        namespace: Namespace::Tx,
        helius_sdk_path: "helius.tx.sendTransactionWithSender",
        compat: CompatLevel::Skipped,
        since_version: None,
        source_doc: "https://www.helius.dev/docs/sending-transactions/sender",
        notes: Some(
            "Parallel Jito routing requires infrastructure we can't reproduce locally. Shim as regular sendTransaction if needed.",
        ),
    },
    // ─── rpc (Helius v2 extensions) ───────────────────────────────────
    MethodEntry {
        method: "getProgramAccountsV2",
        namespace: Namespace::Rpc,
        helius_sdk_path: "helius.getProgramAccountsV2",
        compat: CompatLevel::Planned,
        since_version: None,
        source_doc: "https://www.helius.dev/docs/api-reference/rpc/http/getprogramaccountsv2",
        notes: Some("Cursor-paginated wrapper over getProgramAccounts."),
    },
    MethodEntry {
        method: "getTokenAccountsByOwnerV2",
        namespace: Namespace::Rpc,
        helius_sdk_path: "helius.getTokenAccountsByOwnerV2",
        compat: CompatLevel::Planned,
        since_version: None,
        source_doc: "https://www.helius.dev/docs/api-reference/rpc/http/gettokenaccountsbyownerv2",
        notes: None,
    },
    // ─── staking (all SDK_WRAPPER) ────────────────────────────────────
    MethodEntry {
        method: "createStakeTransaction",
        namespace: Namespace::Staking,
        helius_sdk_path: "helius.staking.createStakeTransaction",
        compat: CompatLevel::SdkWrapper,
        since_version: Some("1.0.0"),
        source_doc: "https://www.helius.dev/docs/staking/how-to-stake-with-helius-programmatically",
        notes: None,
    },
    MethodEntry {
        method: "createUnstakeTransaction",
        namespace: Namespace::Staking,
        helius_sdk_path: "helius.staking.createUnstakeTransaction",
        compat: CompatLevel::SdkWrapper,
        since_version: Some("1.0.0"),
        source_doc: "https://www.helius.dev/docs/staking/how-to-stake-with-helius-programmatically",
        notes: None,
    },
    // ─── webhooks ─────────────────────────────────────────────────────
    MethodEntry {
        method: "createWebhook",
        namespace: Namespace::Webhooks,
        helius_sdk_path: "helius.webhooks.createWebhook",
        compat: CompatLevel::Planned,
        since_version: None,
        source_doc: "https://www.helius.dev/docs/api-reference/webhooks",
        notes: Some("Future: local polling simulator. Not push delivery."),
    },
    // ─── ws ───────────────────────────────────────────────────────────
    MethodEntry {
        method: "signatureSubscribe",
        namespace: Namespace::Ws,
        helius_sdk_path: "helius.ws.signatureNotifications",
        compat: CompatLevel::Shim,
        since_version: Some("1.0.0"),
        source_doc: "https://www.helius.dev/docs/api-reference/rpc/websocket-methods",
        notes: Some(
            "Polyfilled via HTTP polling of getSignatureStatuses. Makes confirmTransaction() / sendAndConfirm() actually work on Surfpool — Surfpool's native WS doesn't implement subscription methods.",
        ),
    },
    MethodEntry {
        method: "signatureUnsubscribe",
        namespace: Namespace::Ws,
        helius_sdk_path: "helius.ws.close",
        compat: CompatLevel::Shim,
        since_version: Some("1.0.0"),
        source_doc: "https://www.helius.dev/docs/api-reference/rpc/websocket-methods",
        notes: None,
    },
    MethodEntry {
        method: "accountSubscribe",
        namespace: Namespace::Ws,
        helius_sdk_path: "helius.ws.accountNotifications",
        compat: CompatLevel::Planned,
        since_version: None,
        source_doc: "https://www.helius.dev/docs/api-reference/rpc/websocket-methods",
        notes: None,
    },
    // ─── wallet (beta) ────────────────────────────────────────────────
    MethodEntry {
        method: "getBalances",
        namespace: Namespace::Wallet,
        helius_sdk_path: "helius.wallet.getBalances",
        compat: CompatLevel::Planned,
        since_version: None,
        source_doc: "https://www.helius.dev/docs/api-reference/wallet-api",
        notes: None,
    },
    // ─── compat (Tidepool-specific introspection) ─────────────────────
    MethodEntry {
        method: "surfpoolHeliusInfo",
        namespace: Namespace::Compat,
        helius_sdk_path: "(tidepool extension)",
        compat: CompatLevel::Exact,
        since_version: Some("1.0.0"),
        source_doc: "https://github.com/TylerTheBuildor/tidepool",
        notes: Some("Returns this manifest + runtime summary. Named with the surfpool- prefix for historical continuity with v0.x (TS)."),
    },
    MethodEntry {
        method: "surfpoolHeliusIndexTree",
        namespace: Namespace::Compat,
        helius_sdk_path: "(tidepool extension)",
        compat: CompatLevel::Exact,
        since_version: Some("1.0.0"),
        source_doc: "https://github.com/TylerTheBuildor/tidepool",
        notes: Some(
            "Runtime tree registration for the cNFT indexer. Params: { tree: string, maxSignatures?, pageSize? }.",
        ),
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_is_non_empty_and_has_all_shipped_methods() {
        let m = manifest();
        assert!(m.len() > 15);
        assert!(m.iter().any(|e| e.method == "getAsset"));
        assert!(m.iter().any(|e| e.method == "getAssetProof"));
        assert!(m.iter().any(|e| e.method == "signatureSubscribe"));
        assert!(m.iter().any(|e| e.method == "surfpoolHeliusIndexTree"));
    }

    #[test]
    fn summarize_counts_every_entry_exactly_once() {
        let m = manifest();
        let s = summarize(m);
        let sum = s.exact
            + s.local_index
            + s.best_effort
            + s.shim
            + s.sdk_wrapper
            + s.planned
            + s.skipped;
        assert_eq!(sum, s.total);
        assert_eq!(s.total as usize, m.len());
    }

    #[test]
    fn every_shipped_entry_has_a_since_version() {
        for e in manifest() {
            match e.compat {
                CompatLevel::Planned | CompatLevel::Skipped => {
                    assert!(e.since_version.is_none(), "planned/skipped should not have since_version: {}", e.method);
                }
                _ => {
                    assert!(e.since_version.is_some(), "shipped method missing since_version: {}", e.method);
                }
            }
        }
    }
}
