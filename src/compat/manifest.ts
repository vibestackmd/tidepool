// The compatibility manifest — source of truth for every Helius method
// this proxy either already implements, plans to implement, or has
// explicitly decided to skip.
//
// Every entry is classified by compat level:
//
//   EXACT        — byte-for-byte Helius compatibility. Pure account read
//                  or pure local computation; no state we don't own.
//   LOCAL_INDEX  — works against state the proxy has seen. Queries about
//                  addresses the user hasn't touched return empty.
//   BEST_EFFORT  — reduced fidelity vs. real Helius. Documented limits.
//   SHIM         — behaviorally close but mechanically different (e.g.
//                  local polling vs. push webhook delivery).
//   PLANNED      — known method, not yet implemented.
//   SKIPPED      — known method, explicitly out of scope (requires cloud
//                  infrastructure we can't reproduce locally).
//
// This list mirrors helius-sdk v2.x namespace structure. When helius-sdk
// ships a new release, the SDK-diff CI job (future) compares its exports
// against this file and files issues for any new methods.

export type CompatLevel =
  | "EXACT"
  | "LOCAL_INDEX"
  | "BEST_EFFORT"
  | "SHIM"
  | "PLANNED"
  | "SKIPPED";

export type Namespace =
  | "das"
  | "enhanced"
  | "tx"
  | "rpc"
  | "staking"
  | "webhooks"
  | "ws"
  | "wallet"
  | "compat";

export interface MethodEntry {
  method: string;
  namespace: Namespace;
  heliusSdkPath: string;
  compat: CompatLevel;
  sinceVersion: string | null;
  sourceDoc: string;
  notes?: string;
}

export const manifest: readonly MethodEntry[] = [
  // ─── das ──────────────────────────────────────────────────────────
  {
    method: "getAsset",
    namespace: "das",
    heliusSdkPath: "helius.das.getAsset",
    compat: "EXACT",
    sinceVersion: "0.1.0",
    sourceDoc: "https://www.helius.dev/docs/api-reference/das/getasset",
  },
  {
    method: "searchAssets",
    namespace: "das",
    heliusSdkPath: "helius.das.searchAssets",
    compat: "LOCAL_INDEX",
    sinceVersion: "0.1.0",
    sourceDoc: "https://www.helius.dev/docs/api-reference/das/searchassets",
    notes:
      "Searches only assets the proxy has fetched via getAsset. Assets never touched return empty.",
  },
  {
    method: "getAssetBatch",
    namespace: "das",
    heliusSdkPath: "helius.das.getAssetBatch",
    compat: "PLANNED",
    sinceVersion: null,
    sourceDoc: "https://www.helius.dev/docs/api-reference/das",
  },
  {
    method: "getAssetProof",
    namespace: "das",
    heliusSdkPath: "helius.das.getAssetProof",
    compat: "PLANNED",
    sinceVersion: null,
    sourceDoc: "https://www.helius.dev/docs/api-reference/das/getassetproof",
  },
  {
    method: "getAssetProofBatch",
    namespace: "das",
    heliusSdkPath: "helius.das.getAssetProofBatch",
    compat: "PLANNED",
    sinceVersion: null,
    sourceDoc: "https://www.helius.dev/docs/api-reference/das",
  },
  {
    method: "getAssetsByOwner",
    namespace: "das",
    heliusSdkPath: "helius.das.getAssetsByOwner",
    compat: "PLANNED",
    sinceVersion: null,
    sourceDoc: "https://www.helius.dev/docs/api-reference/das/getassetsbyowner",
  },
  {
    method: "getAssetsByGroup",
    namespace: "das",
    heliusSdkPath: "helius.das.getAssetsByGroup",
    compat: "PLANNED",
    sinceVersion: null,
    sourceDoc: "https://www.helius.dev/docs/api-reference/das/getassetsbygroup",
  },
  {
    method: "getAssetsByCreator",
    namespace: "das",
    heliusSdkPath: "helius.das.getAssetsByCreator",
    compat: "PLANNED",
    sinceVersion: null,
    sourceDoc: "https://www.helius.dev/docs/api-reference/das/getassetsbycreator",
  },
  {
    method: "getAssetsByAuthority",
    namespace: "das",
    heliusSdkPath: "helius.das.getAssetsByAuthority",
    compat: "PLANNED",
    sinceVersion: null,
    sourceDoc: "https://www.helius.dev/docs/api-reference/das",
  },
  {
    method: "getSignaturesForAsset",
    namespace: "das",
    heliusSdkPath: "helius.das.getSignaturesForAsset",
    compat: "PLANNED",
    sinceVersion: null,
    sourceDoc: "https://www.helius.dev/docs/api-reference/das",
  },
  {
    method: "getNftEditions",
    namespace: "das",
    heliusSdkPath: "helius.das.getNftEditions",
    compat: "PLANNED",
    sinceVersion: null,
    sourceDoc: "https://www.helius.dev/docs/api-reference/das",
  },
  {
    method: "getTokenAccounts",
    namespace: "das",
    heliusSdkPath: "helius.das.getTokenAccounts",
    compat: "PLANNED",
    sinceVersion: null,
    sourceDoc: "https://www.helius.dev/docs/api-reference/das/gettokenaccounts",
  },

  // ─── enhanced ─────────────────────────────────────────────────────
  {
    method: "getTransactions",
    namespace: "enhanced",
    heliusSdkPath: "helius.enhanced.getTransactions",
    compat: "PLANNED",
    sinceVersion: null,
    sourceDoc:
      "https://www.helius.dev/docs/api-reference/enhanced-transactions/gettransactions",
    notes:
      "Will ship with parsers for the top ~10 transaction types (NFT_MINT, NFT_SALE, TRANSFER, SWAP, STAKE_SOL, etc). Unknown types return type: UNKNOWN with raw data.",
  },
  {
    method: "getTransactionsByAddress",
    namespace: "enhanced",
    heliusSdkPath: "helius.enhanced.getTransactionsByAddress",
    compat: "PLANNED",
    sinceVersion: null,
    sourceDoc: "https://www.helius.dev/docs/enhanced-transactions",
  },

  // ─── tx ────────────────────────────────────────────────────────────
  {
    method: "getPriorityFeeEstimate",
    namespace: "tx",
    heliusSdkPath: "helius.tx.getPriorityFeeEstimate",
    compat: "PLANNED",
    sinceVersion: null,
    sourceDoc: "https://www.helius.dev/docs/priority-fee-api",
    notes:
      "Will be implemented via local percentile computation over getRecentPrioritizationFees. Close approximation, not identical to Helius's statistical model.",
  },
  {
    method: "getComputeUnits",
    namespace: "tx",
    heliusSdkPath: "helius.tx.getComputeUnits",
    compat: "PLANNED",
    sinceVersion: null,
    sourceDoc: "https://www.helius.dev/docs/sending-transactions/optimizing-transactions",
  },
  {
    method: "sendSmartTransaction",
    namespace: "tx",
    heliusSdkPath: "helius.tx.sendSmartTransaction",
    compat: "PLANNED",
    sinceVersion: null,
    sourceDoc: "https://www.helius.dev/docs/sending-transactions/optimizing-transactions",
    notes:
      "Will implement compute estimation + basic retry. Lookup table auto-management not planned for v0.3; caller can provide tables manually.",
  },
  {
    method: "createSmartTransaction",
    namespace: "tx",
    heliusSdkPath: "helius.tx.createSmartTransaction",
    compat: "PLANNED",
    sinceVersion: null,
    sourceDoc: "https://www.helius.dev/docs/sending-transactions/optimizing-transactions",
  },
  {
    method: "broadcastTransaction",
    namespace: "tx",
    heliusSdkPath: "helius.tx.broadcastTransaction",
    compat: "PLANNED",
    sinceVersion: null,
    sourceDoc: "https://www.helius.dev/docs/sending-transactions",
  },
  {
    method: "pollTransactionConfirmation",
    namespace: "tx",
    heliusSdkPath: "helius.tx.pollTransactionConfirmation",
    compat: "PLANNED",
    sinceVersion: null,
    sourceDoc: "https://www.helius.dev/docs/sending-transactions",
  },
  {
    method: "sendTransactionWithSender",
    namespace: "tx",
    heliusSdkPath: "helius.tx.sendTransactionWithSender",
    compat: "SKIPPED",
    sinceVersion: null,
    sourceDoc: "https://www.helius.dev/docs/sending-transactions/sender",
    notes:
      "Shimmed as regular sendTransaction. Parallel Jito routing requires infrastructure we can't reproduce locally.",
  },

  // ─── rpc (Helius-flavored V2 extensions of standard Solana RPC) ───
  {
    method: "getProgramAccountsV2",
    namespace: "rpc",
    heliusSdkPath: "helius.getProgramAccountsV2",
    compat: "PLANNED",
    sinceVersion: null,
    sourceDoc: "https://www.helius.dev/docs/api-reference/rpc/http/getprogramaccountsv2",
  },
  {
    method: "getAllProgramAccounts",
    namespace: "rpc",
    heliusSdkPath: "helius.getAllProgramAccounts",
    compat: "PLANNED",
    sinceVersion: null,
    sourceDoc: "https://www.helius.dev/docs/rpc/optimization-techniques",
  },
  {
    method: "getTokenAccountsByOwnerV2",
    namespace: "rpc",
    heliusSdkPath: "helius.getTokenAccountsByOwnerV2",
    compat: "PLANNED",
    sinceVersion: null,
    sourceDoc:
      "https://www.helius.dev/docs/api-reference/rpc/http/gettokenaccountsbyownerv2",
  },
  {
    method: "getAllTokenAccountsByOwner",
    namespace: "rpc",
    heliusSdkPath: "helius.getAllTokenAccountsByOwner",
    compat: "PLANNED",
    sinceVersion: null,
    sourceDoc: "https://www.helius.dev/docs/rpc/optimization-techniques",
  },
  {
    method: "getTransactionsForAddress",
    namespace: "rpc",
    heliusSdkPath: "helius.tx.getTransactionsForAddress",
    compat: "PLANNED",
    sinceVersion: null,
    sourceDoc: "https://www.helius.dev/docs/rpc/gettransactionsforaddress",
  },

  // ─── staking ──────────────────────────────────────────────────────
  {
    method: "createStakeTransaction",
    namespace: "staking",
    heliusSdkPath: "helius.staking.createStakeTransaction",
    compat: "PLANNED",
    sinceVersion: null,
    sourceDoc: "https://www.helius.dev/docs/staking/how-to-stake-with-helius-programmatically",
    notes:
      "Pure local instruction builder. No Helius backend required beyond knowing the Helius validator pubkey.",
  },
  {
    method: "createUnstakeTransaction",
    namespace: "staking",
    heliusSdkPath: "helius.staking.createUnstakeTransaction",
    compat: "PLANNED",
    sinceVersion: null,
    sourceDoc: "https://www.helius.dev/docs/staking/how-to-stake-with-helius-programmatically",
  },
  {
    method: "createWithdrawTransaction",
    namespace: "staking",
    heliusSdkPath: "helius.staking.createWithdrawTransaction",
    compat: "PLANNED",
    sinceVersion: null,
    sourceDoc: "https://www.helius.dev/docs/staking/how-to-stake-with-helius-programmatically",
  },
  {
    method: "getStakeInstructions",
    namespace: "staking",
    heliusSdkPath: "helius.staking.getStakeInstructions",
    compat: "PLANNED",
    sinceVersion: null,
    sourceDoc: "https://www.helius.dev/docs/staking/how-to-stake-with-helius-programmatically",
  },
  {
    method: "getUnstakeInstruction",
    namespace: "staking",
    heliusSdkPath: "helius.staking.getUnstakeInstruction",
    compat: "PLANNED",
    sinceVersion: null,
    sourceDoc: "https://www.helius.dev/docs/staking/how-to-stake-with-helius-programmatically",
  },
  {
    method: "getWithdrawInstruction",
    namespace: "staking",
    heliusSdkPath: "helius.staking.getWithdrawInstruction",
    compat: "PLANNED",
    sinceVersion: null,
    sourceDoc: "https://www.helius.dev/docs/staking/how-to-stake-with-helius-programmatically",
  },
  {
    method: "getWithdrawableAmount",
    namespace: "staking",
    heliusSdkPath: "helius.staking.getWithdrawableAmount",
    compat: "PLANNED",
    sinceVersion: null,
    sourceDoc: "https://www.helius.dev/docs/staking/how-to-stake-with-helius-programmatically",
  },
  {
    method: "getHeliusStakeAccounts",
    namespace: "staking",
    heliusSdkPath: "helius.staking.getHeliusStakeAccounts",
    compat: "PLANNED",
    sinceVersion: null,
    sourceDoc: "https://www.helius.dev/docs/staking/how-to-stake-with-helius-programmatically",
  },

  // ─── webhooks ─────────────────────────────────────────────────────
  {
    method: "createWebhook",
    namespace: "webhooks",
    heliusSdkPath: "helius.webhooks.createWebhook",
    compat: "PLANNED",
    sinceVersion: null,
    sourceDoc: "https://www.helius.dev/docs/api-reference/webhooks",
    notes:
      "Will be shimmed as a local polling simulator (v0.5). POSTs to the webhook URL when Surfpool transactions match the filter. Not push delivery.",
  },
  {
    method: "getWebhookByID",
    namespace: "webhooks",
    heliusSdkPath: "helius.webhooks.getWebhookByID",
    compat: "PLANNED",
    sinceVersion: null,
    sourceDoc: "https://www.helius.dev/docs/api-reference/webhooks",
  },
  {
    method: "getAllWebhooks",
    namespace: "webhooks",
    heliusSdkPath: "helius.webhooks.getAllWebhooks",
    compat: "PLANNED",
    sinceVersion: null,
    sourceDoc: "https://www.helius.dev/docs/api-reference/webhooks",
  },
  {
    method: "updateWebhook",
    namespace: "webhooks",
    heliusSdkPath: "helius.webhooks.updateWebhook",
    compat: "PLANNED",
    sinceVersion: null,
    sourceDoc: "https://www.helius.dev/docs/api-reference/webhooks",
  },
  {
    method: "deleteWebhook",
    namespace: "webhooks",
    heliusSdkPath: "helius.webhooks.deleteWebhook",
    compat: "PLANNED",
    sinceVersion: null,
    sourceDoc: "https://www.helius.dev/docs/api-reference/webhooks",
  },

  // ─── ws ───────────────────────────────────────────────────────────
  {
    method: "signatureSubscribe",
    namespace: "ws",
    heliusSdkPath: "helius.ws.signatureNotifications",
    compat: "SHIM",
    sinceVersion: "0.1.0",
    sourceDoc:
      "https://www.helius.dev/docs/api-reference/rpc/websocket-methods",
    notes:
      "Polyfilled via HTTP polling of getSignatureStatuses. Surfpool's native WS doesn't support subscription methods; this polyfill makes confirmTransaction() and sendAndConfirm() work.",
  },
  {
    method: "signatureUnsubscribe",
    namespace: "ws",
    heliusSdkPath: "helius.ws.close",
    compat: "SHIM",
    sinceVersion: "0.1.0",
    sourceDoc:
      "https://www.helius.dev/docs/api-reference/rpc/websocket-methods",
  },
  {
    method: "accountSubscribe",
    namespace: "ws",
    heliusSdkPath: "helius.ws.accountNotifications",
    compat: "PLANNED",
    sinceVersion: null,
    sourceDoc: "https://www.helius.dev/docs/api-reference/rpc/websocket-methods",
    notes: "Forwarded to Surfpool WS when available.",
  },
  {
    method: "logsSubscribe",
    namespace: "ws",
    heliusSdkPath: "helius.ws.logsNotifications",
    compat: "PLANNED",
    sinceVersion: null,
    sourceDoc: "https://www.helius.dev/docs/api-reference/rpc/websocket-methods",
  },
  {
    method: "programSubscribe",
    namespace: "ws",
    heliusSdkPath: "helius.ws.programNotifications",
    compat: "PLANNED",
    sinceVersion: null,
    sourceDoc: "https://www.helius.dev/docs/api-reference/rpc/websocket-methods",
  },
  {
    method: "slotSubscribe",
    namespace: "ws",
    heliusSdkPath: "helius.ws.slotNotifications",
    compat: "PLANNED",
    sinceVersion: null,
    sourceDoc: "https://www.helius.dev/docs/api-reference/rpc/websocket-methods",
  },
  {
    method: "rootSubscribe",
    namespace: "ws",
    heliusSdkPath: "helius.ws.rootNotifications",
    compat: "PLANNED",
    sinceVersion: null,
    sourceDoc: "https://www.helius.dev/docs/api-reference/rpc/websocket-methods",
  },

  // ─── wallet (beta) ────────────────────────────────────────────────
  {
    method: "getBalances",
    namespace: "wallet",
    heliusSdkPath: "helius.wallet.getBalances",
    compat: "PLANNED",
    sinceVersion: null,
    sourceDoc: "https://www.helius.dev/docs/api-reference/wallet-api",
    notes: "Beta upstream. Local reproduction will aggregate token accounts; USD pricing out of scope.",
  },
  {
    method: "getHistory",
    namespace: "wallet",
    heliusSdkPath: "helius.wallet.getHistory",
    compat: "PLANNED",
    sinceVersion: null,
    sourceDoc: "https://www.helius.dev/docs/api-reference/wallet-api",
  },
  {
    method: "getTransfers",
    namespace: "wallet",
    heliusSdkPath: "helius.wallet.getTransfers",
    compat: "PLANNED",
    sinceVersion: null,
    sourceDoc: "https://www.helius.dev/docs/api-reference/wallet-api",
  },
  {
    method: "getFundedBy",
    namespace: "wallet",
    heliusSdkPath: "helius.wallet.getFundedBy",
    compat: "PLANNED",
    sinceVersion: null,
    sourceDoc: "https://www.helius.dev/docs/api-reference/wallet-api",
  },
  {
    method: "getIdentity",
    namespace: "wallet",
    heliusSdkPath: "helius.wallet.getIdentity",
    compat: "PLANNED",
    sinceVersion: null,
    sourceDoc: "https://www.helius.dev/docs/api-reference/wallet-api",
    notes: "Requires a curated wallet-label database. Local implementation will ship with an empty labels set.",
  },
  {
    method: "getBatchIdentity",
    namespace: "wallet",
    heliusSdkPath: "helius.wallet.getBatchIdentity",
    compat: "PLANNED",
    sinceVersion: null,
    sourceDoc: "https://www.helius.dev/docs/api-reference/wallet-api",
  },

  // ─── compat (this proxy's own custom introspection method) ────────
  {
    method: "surfpoolHeliusInfo",
    namespace: "compat",
    heliusSdkPath: "(surfpool-helius extension)",
    compat: "EXACT",
    sinceVersion: "0.1.1",
    sourceDoc: "https://github.com/tylerthebuildor/surfpool-helius",
    notes:
      "Custom method — not part of Helius. Returns this manifest so callers can introspect which methods are supported at what fidelity before they run against the proxy.",
  },
];

export interface ManifestSummary {
  exact: number;
  localIndex: number;
  bestEffort: number;
  shim: number;
  planned: number;
  skipped: number;
  total: number;
}

export function summarize(entries: readonly MethodEntry[]): ManifestSummary {
  const s: ManifestSummary = {
    exact: 0,
    localIndex: 0,
    bestEffort: 0,
    shim: 0,
    planned: 0,
    skipped: 0,
    total: entries.length,
  };
  for (const e of entries) {
    switch (e.compat) {
      case "EXACT":
        s.exact++;
        break;
      case "LOCAL_INDEX":
        s.localIndex++;
        break;
      case "BEST_EFFORT":
        s.bestEffort++;
        break;
      case "SHIM":
        s.shim++;
        break;
      case "PLANNED":
        s.planned++;
        break;
      case "SKIPPED":
        s.skipped++;
        break;
    }
  }
  return s;
}
