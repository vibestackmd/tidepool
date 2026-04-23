//! Compatibility manifest — structured metadata about every Helius
//! method, with a documented compat level so consumers know what's
//! genuinely 1:1 vs what's best-effort local simulation.
//!
//! Served by the `tidepool_info` RPC. Also usable as a plain
//! Rust constant for tooling that wants to diff against upstream
//! Helius SDK changes at build time.

use serde::{Deserialize, Serialize};

/// Fidelity classification, matches the TS version one-for-one so
/// clients that parse the manifest work against both.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CompatLevel {
    /// Deterministic local computation with no external state. Covers
    /// two groups:
    ///
    /// 1. Methods where the local answer is provably identical to
    ///    Helius (pure reads, pure math).
    /// 2. Tidepool-native introspection methods that have no Helius
    ///    counterpart (e.g. `tidepool_info`,
    ///    `tidepool_indexTree`). Still "exact" because there's
    ///    no upstream fidelity gap to worry about.
    Exact,
    /// Serves primarily from local state we've observed. Some entries
    /// in this tier fall through to live upstream fetch + decode when
    /// the upstream has a cheap primitive (e.g. `getAsset` uses
    /// `getAccountInfo` + decoder for uncompressed NFTs). Others
    /// genuinely return empty on miss because no upstream primitive
    /// exists — cNFT proofs, by-owner NFT queries, anything the whole
    /// point of a DAS index. Each entry's `notes` spells out the
    /// specific behavior.
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

/// Transport a method lives on. We mirror whatever Helius chose:
/// JSON-RPC for the core `mainnet.helius-rpc.com` surface, REST for
/// Wallet / Enhanced / Webhooks / etc. that live on `api.helius.xyz`.
/// Clients should never write local code that'd fail against prod
/// because of a transport mismatch.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Transport {
    #[default]
    /// JSON-RPC 2.0 over POST. The default for most DAS + RPC
    /// methods.
    JsonRpc,
    /// REST. Helius's Wallet / Enhanced Transactions / Webhooks
    /// surfaces. Path + verb given in the entry's notes.
    Rest,
    /// WebSocket subscription.
    Ws,
    /// helius-sdk-only — the method is a client-side SDK helper that
    /// composes one or more wire calls. Not a transport Tidepool
    /// serves directly.
    SdkWrapper,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MethodEntry {
    pub method: &'static str,
    pub namespace: Namespace,
    pub helius_sdk_path: &'static str,
    pub compat: CompatLevel,
    /// Transport the method lives on. Must match Helius's transport
    /// choice — asymmetric transports mean client code breaks when
    /// pointed at real Helius. Defaults to JSON-RPC (the majority
    /// case); REST / WS / SdkWrapper are explicit overrides.
    #[serde(default)]
    pub transport: Transport,
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
        transport: Transport::JsonRpc,
        since_version: Some("1.0.0"),
        source_doc: "https://www.helius.dev/docs/api-reference/das/getasset",
        notes: Some(
            "Hybrid: uncompressed NFTs (Token Metadata, MplCore) work cold — resolve via upstream getAccountInfo + decoder on cache miss. Compressed NFTs require local Bubblegum indexing (no upstream equivalent exists) and return null for trees we haven't observed. LOCAL_INDEX reflects the cNFT constraint.",
        ),
    },
    MethodEntry {
        method: "getAssetBatch",
        namespace: Namespace::Das,
        helius_sdk_path: "helius.das.getAssetBatch",
        compat: CompatLevel::LocalIndex,
        transport: Transport::JsonRpc,
        since_version: Some("1.0.0"),
        source_doc: "https://www.helius.dev/docs/api-reference/das",
        notes: Some("Sequential fanout over getAsset (up to 1000 ids). Same hybrid behavior: uncompressed entries hit upstream on miss, compressed require indexing."),
    },
    MethodEntry {
        method: "getAssetProof",
        namespace: Namespace::Das,
        helius_sdk_path: "helius.das.getAssetProof",
        compat: CompatLevel::LocalIndex,
        transport: Transport::JsonRpc,
        since_version: Some("1.0.0"),
        source_doc: "https://www.helius.dev/docs/api-reference/das/getassetproof",
        notes: Some(
            "cNFT merkle proof computed from the local indexer. Trees must be registered via --index-tree or tidepool_indexTree before resolution works.",
        ),
    },
    MethodEntry {
        method: "getAssetProofBatch",
        namespace: Namespace::Das,
        helius_sdk_path: "helius.das.getAssetProofBatch",
        compat: CompatLevel::LocalIndex,
        transport: Transport::JsonRpc,
        since_version: Some("1.0.0"),
        source_doc: "https://www.helius.dev/docs/api-reference/das",
        notes: Some("Shares TreeState materialization across ids in the batch."),
    },
    MethodEntry {
        method: "getAssetsByOwner",
        namespace: Namespace::Das,
        helius_sdk_path: "helius.das.getAssetsByOwner",
        compat: CompatLevel::LocalIndex,
        transport: Transport::JsonRpc,
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
        transport: Transport::JsonRpc,
        since_version: Some("1.0.0"),
        source_doc: "https://www.helius.dev/docs/api-reference/das",
        notes: None,
    },
    MethodEntry {
        method: "getAssetsByCreator",
        namespace: Namespace::Das,
        helius_sdk_path: "helius.das.getAssetsByCreator",
        compat: CompatLevel::LocalIndex,
        transport: Transport::JsonRpc,
        since_version: Some("1.0.0"),
        source_doc: "https://www.helius.dev/docs/api-reference/das",
        notes: Some("`onlyVerified` filters to creators whose verified flag is true."),
    },
    MethodEntry {
        method: "getAssetsByGroup",
        namespace: Namespace::Das,
        helius_sdk_path: "helius.das.getAssetsByGroup",
        compat: CompatLevel::LocalIndex,
        transport: Transport::JsonRpc,
        since_version: Some("1.0.0"),
        source_doc: "https://www.helius.dev/docs/api-reference/das",
        notes: Some("For MplCore: groupKey=\"collection\"."),
    },
    MethodEntry {
        method: "searchAssets",
        namespace: Namespace::Das,
        helius_sdk_path: "helius.das.searchAssets",
        compat: CompatLevel::LocalIndex,
        transport: Transport::JsonRpc,
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
        compat: CompatLevel::LocalIndex,
        transport: Transport::JsonRpc,
        since_version: Some("1.0.0"),
        source_doc: "https://www.helius.dev/docs/api-reference/das",
        notes: Some("Cold call on an unknown master mint fetches + indexes the master's Edition PDA via getAsset first, then serves from the index. Print editions are accumulated lazily as they're individually fetched via getAsset — list reflects what Tidepool has observed."),
    },
    MethodEntry {
        method: "getTokenAccounts",
        namespace: Namespace::Das,
        helius_sdk_path: "helius.das.getTokenAccounts",
        compat: CompatLevel::Shim,
        transport: Transport::JsonRpc,
        since_version: Some("1.0.0"),
        source_doc: "https://www.helius.dev/docs/api-reference/das/gettokenaccounts",
        notes: Some("Forwards to getTokenAccountsByOwner (owner filter) or getProgramAccounts memcmp (mint-only filter). Queries both SPL Token and Token-2022. Paginates locally; no cursor support."),
    },
    // ─── enhanced ─────────────────────────────────────────────────────
    MethodEntry {
        method: "getTransactions",
        namespace: Namespace::Enhanced,
        helius_sdk_path: "helius.enhanced.getTransactions",
        compat: CompatLevel::BestEffort,
        transport: Transport::Rest,
        since_version: Some("1.0.0"),
        source_doc: "https://www.helius.dev/docs/api-reference/enhanced-transactions/gettransactions",
        notes: Some("REST: POST /v0/transactions. Body: { transactions: [sig, ...] }. Narrow classifier — TRANSFER / COMPRESSED_NFT_* / NFT_MINT / UNKNOWN; SWAP/STAKE/DEFI parsers not implemented."),
    },
    MethodEntry {
        method: "getTransactionsByAddress",
        namespace: Namespace::Enhanced,
        helius_sdk_path: "helius.enhanced.getTransactionsByAddress",
        compat: CompatLevel::BestEffort,
        transport: Transport::Rest,
        since_version: Some("1.0.0"),
        source_doc: "https://www.helius.dev/docs/enhanced-transactions",
        notes: Some("REST: GET /v0/addresses/:address/transactions. Resolves signatures via getSignaturesForAddress + runs classifier. Same narrow scope as getTransactions."),
    },
    // ─── tx ───────────────────────────────────────────────────────────
    MethodEntry {
        method: "getPriorityFeeEstimate",
        namespace: Namespace::Tx,
        helius_sdk_path: "helius.tx.getPriorityFeeEstimate",
        compat: CompatLevel::BestEffort,
        transport: Transport::JsonRpc,
        since_version: Some("1.0.0"),
        source_doc: "https://www.helius.dev/docs/priority-fee-api",
        notes: Some("Local percentile ladder (min/low/medium/high/veryHigh/unsafeMax) computed over getRecentPrioritizationFees samples."),
    },
    MethodEntry {
        method: "sendSmartTransaction",
        namespace: Namespace::Tx,
        helius_sdk_path: "helius.tx.sendSmartTransaction",
        compat: CompatLevel::SdkWrapper,
        transport: Transport::SdkWrapper,
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
        transport: Transport::SdkWrapper,
        since_version: Some("1.0.0"),
        source_doc: "https://www.helius.dev/docs/sending-transactions",
        notes: None,
    },
    MethodEntry {
        method: "pollTransactionConfirmation",
        namespace: Namespace::Tx,
        helius_sdk_path: "helius.tx.pollTransactionConfirmation",
        compat: CompatLevel::SdkWrapper,
        transport: Transport::SdkWrapper,
        since_version: Some("1.0.0"),
        source_doc: "https://www.helius.dev/docs/sending-transactions",
        notes: None,
    },
    MethodEntry {
        method: "sendTransactionWithSender",
        namespace: Namespace::Tx,
        helius_sdk_path: "helius.tx.sendTransactionWithSender",
        compat: CompatLevel::Shim,
        transport: Transport::JsonRpc,
        since_version: Some("1.0.0"),
        source_doc: "https://www.helius.dev/docs/sending-transactions/sender",
        notes: Some(
            "Forwards to upstream sendTransaction. Helius's production impl runs the tx through its parallel Jito-relay fleet for faster landing; locally we can't reproduce that, so inclusion latency matches whatever Surfpool gives you. The method works — callers still get a signature back.",
        ),
    },
    // ─── rpc (Helius v2 extensions) ───────────────────────────────────
    MethodEntry {
        method: "getProgramAccountsV2",
        namespace: Namespace::Rpc,
        helius_sdk_path: "helius.getProgramAccountsV2",
        compat: CompatLevel::Shim,
        transport: Transport::JsonRpc,
        since_version: Some("1.0.0"),
        source_doc: "https://www.helius.dev/docs/api-reference/rpc/http/getprogramaccountsv2",
        notes: Some("Forwards to getProgramAccounts; sorts by pubkey and slices locally by cursor+limit. Cursor is the last pubkey from the prior page."),
    },
    MethodEntry {
        method: "getTokenAccountsByOwnerV2",
        namespace: Namespace::Rpc,
        helius_sdk_path: "helius.getTokenAccountsByOwnerV2",
        compat: CompatLevel::Shim,
        transport: Transport::JsonRpc,
        since_version: Some("1.0.0"),
        source_doc: "https://www.helius.dev/docs/api-reference/rpc/http/gettokenaccountsbyownerv2",
        notes: Some("Forwards to getTokenAccountsByOwner; cursor semantics match getProgramAccountsV2."),
    },
    // ─── staking (all SDK_WRAPPER) ────────────────────────────────────
    MethodEntry {
        method: "createStakeTransaction",
        namespace: Namespace::Staking,
        helius_sdk_path: "helius.staking.createStakeTransaction",
        compat: CompatLevel::SdkWrapper,
        transport: Transport::SdkWrapper,
        since_version: Some("1.0.0"),
        source_doc: "https://www.helius.dev/docs/staking/how-to-stake-with-helius-programmatically",
        notes: None,
    },
    MethodEntry {
        method: "createUnstakeTransaction",
        namespace: Namespace::Staking,
        helius_sdk_path: "helius.staking.createUnstakeTransaction",
        compat: CompatLevel::SdkWrapper,
        transport: Transport::SdkWrapper,
        since_version: Some("1.0.0"),
        source_doc: "https://www.helius.dev/docs/staking/how-to-stake-with-helius-programmatically",
        notes: None,
    },
    // ─── webhooks ─────────────────────────────────────────────────────
    MethodEntry {
        method: "createWebhook",
        namespace: Namespace::Webhooks,
        helius_sdk_path: "helius.webhooks.createWebhook",
        compat: CompatLevel::Shim,
        transport: Transport::Rest,
        since_version: Some("1.0.0"),
        source_doc: "https://www.helius.dev/docs/api-reference/webhooks",
        notes: Some("REST: POST /v0/webhooks. Local polling simulator — spawns a background task that polls getSignaturesForAddress every 500ms and POSTs a simplified tx envelope to the user URL."),
    },
    MethodEntry {
        method: "getAllWebhooks",
        namespace: Namespace::Webhooks,
        helius_sdk_path: "helius.webhooks.getAllWebhooks",
        compat: CompatLevel::Shim,
        transport: Transport::Rest,
        since_version: Some("1.0.0"),
        source_doc: "https://www.helius.dev/docs/api-reference/webhooks",
        notes: Some("REST: GET /v0/webhooks. In-memory registry by default; persistent behind --db."),
    },
    MethodEntry {
        method: "getWebhookByID",
        namespace: Namespace::Webhooks,
        helius_sdk_path: "helius.webhooks.getWebhookByID",
        compat: CompatLevel::Shim,
        transport: Transport::Rest,
        since_version: Some("1.0.0"),
        source_doc: "https://www.helius.dev/docs/api-reference/webhooks",
        notes: Some("REST: GET /v0/webhooks/:id."),
    },
    MethodEntry {
        method: "editWebhook",
        namespace: Namespace::Webhooks,
        helius_sdk_path: "helius.webhooks.editWebhook",
        compat: CompatLevel::Shim,
        transport: Transport::Rest,
        since_version: Some("1.0.0"),
        source_doc: "https://www.helius.dev/docs/api-reference/webhooks",
        notes: Some("REST: PUT /v0/webhooks/:id. Merges into existing record; restarts the polling task with fresh cursor state."),
    },
    MethodEntry {
        method: "deleteWebhook",
        namespace: Namespace::Webhooks,
        helius_sdk_path: "helius.webhooks.deleteWebhook",
        compat: CompatLevel::Shim,
        transport: Transport::Rest,
        since_version: Some("1.0.0"),
        source_doc: "https://www.helius.dev/docs/api-reference/webhooks",
        notes: Some("REST: DELETE /v0/webhooks/:id."),
    },
    // ─── ws ───────────────────────────────────────────────────────────
    MethodEntry {
        method: "signatureSubscribe",
        namespace: Namespace::Ws,
        helius_sdk_path: "helius.ws.signatureNotifications",
        compat: CompatLevel::Shim,
        transport: Transport::Ws,
        since_version: Some("1.0.0"),
        source_doc: "https://www.helius.dev/docs/api-reference/rpc/websocket-methods",
        notes: Some(
            "WebSocket. Polyfilled via HTTP polling of getSignatureStatuses. Makes confirmTransaction() / sendAndConfirm() actually work on Surfpool — Surfpool's native WS doesn't implement subscription methods.",
        ),
    },
    MethodEntry {
        method: "signatureUnsubscribe",
        namespace: Namespace::Ws,
        helius_sdk_path: "helius.ws.close",
        compat: CompatLevel::Shim,
        transport: Transport::Ws,
        since_version: Some("1.0.0"),
        source_doc: "https://www.helius.dev/docs/api-reference/rpc/websocket-methods",
        notes: None,
    },
    MethodEntry {
        method: "accountSubscribe",
        namespace: Namespace::Ws,
        helius_sdk_path: "helius.ws.accountNotifications",
        compat: CompatLevel::Shim,
        transport: Transport::Ws,
        since_version: Some("1.0.0"),
        source_doc: "https://www.helius.dev/docs/api-reference/rpc/websocket-methods",
        notes: Some("WebSocket. Polls getAccountInfo every 500ms and emits accountNotification on change. Long-lived until accountUnsubscribe."),
    },
    MethodEntry {
        method: "logsSubscribe",
        namespace: Namespace::Ws,
        helius_sdk_path: "helius.ws.logsNotifications",
        compat: CompatLevel::Shim,
        transport: Transport::Ws,
        since_version: Some("1.0.0"),
        source_doc: "https://www.helius.dev/docs/api-reference/rpc/websocket-methods",
        notes: Some("WebSocket. Polls getSignaturesForAddress + getTransaction to emit logsNotification. `{ mentions: [pubkey] }` filter only — `all`/`allWithVotes` return -32601 since no efficient polling shim exists."),
    },
    MethodEntry {
        method: "logsUnsubscribe",
        namespace: Namespace::Ws,
        helius_sdk_path: "helius.ws.close",
        compat: CompatLevel::Shim,
        transport: Transport::Ws,
        since_version: Some("1.0.0"),
        source_doc: "https://www.helius.dev/docs/api-reference/rpc/websocket-methods",
        notes: None,
    },
    // ─── wallet (beta) ────────────────────────────────────────────────
    MethodEntry {
        method: "getBalances",
        namespace: Namespace::Wallet,
        helius_sdk_path: "helius.wallet.getBalances",
        compat: CompatLevel::Shim,
        transport: Transport::Rest,
        since_version: Some("1.0.0"),
        source_doc: "https://www.helius.dev/docs/api-reference/wallet-api",
        notes: Some("REST: GET /v0/addresses/:address/balances. Fans out to getBalance + getTokenAccountsByOwner (SPL + Token-2022). USD pricing fields are null by design — no local price feed."),
    },
    // ─── compat (Tidepool-specific introspection) ─────────────────────
    MethodEntry {
        method: "tidepool_info",
        namespace: Namespace::Compat,
        helius_sdk_path: "(tidepool extension)",
        compat: CompatLevel::Exact,
        transport: Transport::JsonRpc,
        since_version: Some("1.0.0"),
        source_doc: "https://github.com/TylerTheBuildor/tidepool",
        notes: Some("Returns this manifest + runtime summary."),
    },
    MethodEntry {
        method: "tidepool_indexTree",
        namespace: Namespace::Compat,
        helius_sdk_path: "(tidepool extension)",
        compat: CompatLevel::Exact,
        transport: Transport::JsonRpc,
        since_version: Some("1.0.0"),
        source_doc: "https://github.com/TylerTheBuildor/tidepool",
        notes: Some(
            "Runtime tree registration for the cNFT indexer. Params: { tree: string, maxSignatures?, pageSize? }.",
        ),
    },
    MethodEntry {
        method: "tidepool_exportTreeSnapshot",
        namespace: Namespace::Compat,
        helius_sdk_path: "(tidepool extension)",
        compat: CompatLevel::Exact,
        transport: Transport::JsonRpc,
        since_version: Some("1.0.0"),
        source_doc: "https://github.com/TylerTheBuildor/tidepool",
        notes: Some(
            "Export one tree's indexed state as a portable snapshot envelope (base64-wrapped JSON). Params: { tree: string }. Pair with tidepool_loadTreeSnapshot for fresh-boot preload.",
        ),
    },
    MethodEntry {
        method: "tidepool_loadTreeSnapshot",
        namespace: Namespace::Compat,
        helius_sdk_path: "(tidepool extension)",
        compat: CompatLevel::Exact,
        transport: Transport::JsonRpc,
        since_version: Some("1.0.0"),
        source_doc: "https://github.com/TylerTheBuildor/tidepool",
        notes: Some(
            "Apply a previously-dumped snapshot to the local store. Params: { snapshot: SnapshotBlob }. Overwrites any existing state for that tree.",
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
        assert!(m.iter().any(|e| e.method == "tidepool_indexTree"));
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
