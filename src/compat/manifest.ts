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
//   SDK_WRAPPER  — a helius-sdk client-side helper that composes standard
//                  wire-level RPC methods under the hood. The proxy does
//                  NOT intercept these — they run in the SDK in the
//                  user's app, make several wire-level RPC calls that
//                  pass through this proxy, and "just work" as long as
//                  the underlying wire methods are supported. The entry
//                  exists in the manifest so users know what works when
//                  they point helius-sdk at this proxy, even though no
//                  handler code lives here.
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
  | "SDK_WRAPPER"
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
    notes:
      "MplCore assets + collections via Codama-generated Kit client at src/generated/mpl-core (plugin walker added in v0.3). v0.5.0 adds legacy Metaplex Token Metadata NFTs via src/generated/token-metadata — `getAsset(mint)` routes through the Metadata PDA and resolves the token holder via a getProgramAccounts memcmp scan of the owning token program (Surfpool's getTokenLargestAccounts times out). v0.5.1 extends the mint-as-id path to Token-2022 mints (both programs share the Metadata PDA derivation). Both decoders regenerate from pinned IDLs (idls/*.source.json).",
  },
  {
    method: "searchAssets",
    namespace: "das",
    heliusSdkPath: "helius.das.searchAssets",
    compat: "LOCAL_INDEX",
    sinceVersion: "0.1.0",
    sourceDoc: "https://www.helius.dev/docs/api-reference/das/searchassets",
    notes:
      "Full filter surface as of v0.3: ownerAddress, authorityAddress, creatorAddress, interface, grouping, tokenType, compressed, sortBy. All filters backed by secondary indexes on the local cache.",
  },
  {
    method: "getAssetBatch",
    namespace: "das",
    heliusSdkPath: "helius.das.getAssetBatch",
    compat: "EXACT",
    sinceVersion: "0.2.0",
    sourceDoc: "https://www.helius.dev/docs/api-reference/das",
    notes: "Parallel upstream reads with cache population. Max 1000 ids per batch.",
  },
  {
    method: "getAssetProof",
    namespace: "das",
    heliusSdkPath: "helius.das.getAssetProof",
    compat: "PLANNED",
    sinceVersion: null,
    sourceDoc: "https://www.helius.dev/docs/api-reference/das/getassetproof",
    notes:
      "Deferred. Originally scoped as v0.5.0 but reclassified after research proved it can't be a thin wrapper. Concurrent Merkle Tree accounts only store the current root + a rolling changelog — they have no assetId→leafIndex map or historical leaves, so a correct proof requires a full local cNFT indexer replaying every Bubblegum ix. That violates the 'thin wrapper' invariant; see docs/proxy-strategy.md §6 'Research finding' and 'Deferred: cNFT proof support' for the full reasoning and the shape of a future dedicated milestone.",
  },
  {
    method: "getAssetProofBatch",
    namespace: "das",
    heliusSdkPath: "helius.das.getAssetProofBatch",
    compat: "PLANNED",
    sinceVersion: null,
    sourceDoc: "https://www.helius.dev/docs/api-reference/das",
    notes: "Deferred alongside getAssetProof — same research finding.",
  },
  {
    method: "getAssetsByOwner",
    namespace: "das",
    heliusSdkPath: "helius.das.getAssetsByOwner",
    compat: "LOCAL_INDEX",
    sinceVersion: "0.2.0",
    sourceDoc: "https://www.helius.dev/docs/api-reference/das/getassetsbyowner",
    notes: "Returns assets the proxy has indexed via prior getAsset/getAssetBatch calls. Owners never touched return empty.",
  },
  {
    method: "getAssetsByGroup",
    namespace: "das",
    heliusSdkPath: "helius.das.getAssetsByGroup",
    compat: "LOCAL_INDEX",
    sinceVersion: "0.2.0",
    sourceDoc: "https://www.helius.dev/docs/api-reference/das/getassetsbygroup",
    notes: "For MplCore, groupKey is 'collection' and groupValue is the collection pubkey.",
  },
  {
    method: "getAssetsByCreator",
    namespace: "das",
    heliusSdkPath: "helius.das.getAssetsByCreator",
    compat: "LOCAL_INDEX",
    sinceVersion: "0.3.0",
    sourceDoc: "https://www.helius.dev/docs/api-reference/das/getassetsbycreator",
    notes:
      "Creators are derived from the Royalties and VerifiedCreators MplCore plugins, merged into a single {address, share, verified} list. Supports `onlyVerified` filter.",
  },
  {
    method: "getAssetsByAuthority",
    namespace: "das",
    heliusSdkPath: "helius.das.getAssetsByAuthority",
    compat: "LOCAL_INDEX",
    sinceVersion: "0.2.0",
    sourceDoc: "https://www.helius.dev/docs/api-reference/das",
    notes: "Matches the MplCore update authority. For Collection-authority assets, querying by the collection pubkey returns the member assets.",
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
    compat: "LOCAL_INDEX",
    sinceVersion: "0.5.0",
    sourceDoc: "https://www.helius.dev/docs/api-reference/das",
    notes:
      "Master edition supply / max_supply are EXACT — read directly from the on-chain MasterEditionV1/V2 account via a Codama-generated decoder. The editions[] list is LOCAL_INDEX: as of v0.5.1 it reflects print editions fetch.ts has observed while routing mint-as-id requests. A print mint that has never been fetched through this proxy will not appear; fetch it once via getAsset(printMint) and it's indexed for the next call. Pagination is page/limit, applied in-memory.",
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
    compat: "BEST_EFFORT",
    sinceVersion: "0.4.0",
    sourceDoc: "https://www.helius.dev/docs/priority-fee-api",
    notes:
      "Percentiles (Min/Low/Medium/High/VeryHigh/UnsafeMax) are computed locally over getRecentPrioritizationFees samples from the upstream. Close to real Helius but not identical — they use their own fleet-wide fee aggregation. On Surfpool (local, no contention) all percentiles are typically 0.",
  },
  {
    method: "getComputeUnits",
    namespace: "tx",
    heliusSdkPath: "helius.tx.getComputeUnits",
    compat: "SDK_WRAPPER",
    sinceVersion: null,
    sourceDoc: "https://www.helius.dev/docs/sending-transactions/optimizing-transactions",
    notes:
      "helius-sdk client-side helper. Calls simulateTransaction on the provided transaction and extracts the compute units used. Works transparently against this proxy via the simulateTransaction passthrough.",
  },
  {
    method: "sendSmartTransaction",
    namespace: "tx",
    heliusSdkPath: "helius.tx.sendSmartTransaction",
    compat: "SDK_WRAPPER",
    sinceVersion: null,
    sourceDoc: "https://www.helius.dev/docs/sending-transactions/optimizing-transactions",
    notes:
      "helius-sdk composition: simulateTransaction → getPriorityFeeEstimate → getLatestBlockhash → sendTransaction → getSignatureStatuses polling. Every underlying call passes through this proxy and works without a dedicated handler.",
  },
  {
    method: "createSmartTransaction",
    namespace: "tx",
    heliusSdkPath: "helius.tx.createSmartTransaction",
    compat: "SDK_WRAPPER",
    sinceVersion: null,
    sourceDoc: "https://www.helius.dev/docs/sending-transactions/optimizing-transactions",
    notes: "Same underlying RPC calls as sendSmartTransaction, minus the final send. Works transparently.",
  },
  {
    method: "broadcastTransaction",
    namespace: "tx",
    heliusSdkPath: "helius.tx.broadcastTransaction",
    compat: "SDK_WRAPPER",
    sinceVersion: null,
    sourceDoc: "https://www.helius.dev/docs/sending-transactions",
    notes: "helius-sdk helper that wraps sendTransaction + getSignatureStatuses polling. Works transparently.",
  },
  {
    method: "pollTransactionConfirmation",
    namespace: "tx",
    heliusSdkPath: "helius.tx.pollTransactionConfirmation",
    compat: "SDK_WRAPPER",
    sinceVersion: null,
    sourceDoc: "https://www.helius.dev/docs/sending-transactions",
    notes: "helius-sdk helper that polls getSignatureStatuses until confirmed or timeout. Works transparently.",
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
    compat: "BEST_EFFORT",
    sinceVersion: "0.4.0",
    sourceDoc: "https://www.helius.dev/docs/api-reference/rpc/http/getprogramaccountsv2",
    notes:
      "Passthrough to standard getProgramAccounts with cursor wrapping for pagination. `changedSinceSlot` is not supported (no per-account slot tracking locally) — the response includes a note field when the param is passed.",
  },
  {
    method: "getAllProgramAccounts",
    namespace: "rpc",
    heliusSdkPath: "helius.getAllProgramAccounts",
    compat: "SDK_WRAPPER",
    sinceVersion: null,
    sourceDoc: "https://www.helius.dev/docs/rpc/optimization-techniques",
    notes:
      "helius-sdk auto-paginating wrapper around getProgramAccountsV2. Loops in the client until paginationKey is null. Works transparently against this proxy.",
  },
  {
    method: "getTokenAccountsByOwnerV2",
    namespace: "rpc",
    heliusSdkPath: "helius.getTokenAccountsByOwnerV2",
    compat: "BEST_EFFORT",
    sinceVersion: "0.4.0",
    sourceDoc:
      "https://www.helius.dev/docs/api-reference/rpc/http/gettokenaccountsbyownerv2",
    notes:
      "Passthrough to standard getTokenAccountsByOwner with cursor wrapping. Same changedSinceSlot limitation as getProgramAccountsV2.",
  },
  {
    method: "getAllTokenAccountsByOwner",
    namespace: "rpc",
    heliusSdkPath: "helius.getAllTokenAccountsByOwner",
    compat: "SDK_WRAPPER",
    sinceVersion: null,
    sourceDoc: "https://www.helius.dev/docs/rpc/optimization-techniques",
    notes:
      "helius-sdk auto-paginating wrapper around getTokenAccountsByOwnerV2.",
  },
  {
    method: "getTransactionsForAddress",
    namespace: "rpc",
    heliusSdkPath: "helius.tx.getTransactionsForAddress",
    compat: "PLANNED",
    sinceVersion: null,
    sourceDoc: "https://www.helius.dev/docs/rpc/gettransactionsforaddress",
    notes:
      "Helius built this on their proprietary archival storage backend (replaced BigTable). Meaningful local reproduction requires transaction-level indexing we don't have yet — deferring to the same release as Enhanced Transactions parsing.",
  },

  // ─── staking ──────────────────────────────────────────────────────
  // All helius.staking.* methods are SDK_WRAPPER: they run entirely
  // client-side in helius-sdk, building standard Solana stake program
  // instructions and returning serialized transactions or instruction
  // arrays. The only RPC calls they make (if any) are getLatestBlockhash
  // — which passes through to Surfpool. Nothing for us to intercept.
  {
    method: "createStakeTransaction",
    namespace: "staking",
    heliusSdkPath: "helius.staking.createStakeTransaction",
    compat: "SDK_WRAPPER",
    sinceVersion: null,
    sourceDoc: "https://www.helius.dev/docs/staking/how-to-stake-with-helius-programmatically",
    notes:
      "helius-sdk client-side instruction builder. Constructs a standard Solana stake program transaction delegating to the Helius validator. Only RPC touch is getLatestBlockhash, which passes through.",
  },
  {
    method: "createUnstakeTransaction",
    namespace: "staking",
    heliusSdkPath: "helius.staking.createUnstakeTransaction",
    compat: "SDK_WRAPPER",
    sinceVersion: null,
    sourceDoc: "https://www.helius.dev/docs/staking/how-to-stake-with-helius-programmatically",
    notes: "Client-side stake-deactivation instruction builder. Works transparently.",
  },
  {
    method: "createWithdrawTransaction",
    namespace: "staking",
    heliusSdkPath: "helius.staking.createWithdrawTransaction",
    compat: "SDK_WRAPPER",
    sinceVersion: null,
    sourceDoc: "https://www.helius.dev/docs/staking/how-to-stake-with-helius-programmatically",
    notes: "Client-side withdraw instruction builder. Works transparently.",
  },
  {
    method: "getStakeInstructions",
    namespace: "staking",
    heliusSdkPath: "helius.staking.getStakeInstructions",
    compat: "SDK_WRAPPER",
    sinceVersion: null,
    sourceDoc: "https://www.helius.dev/docs/staking/how-to-stake-with-helius-programmatically",
    notes: "Returns the raw stake instructions without wrapping them in a transaction. Pure local.",
  },
  {
    method: "getUnstakeInstruction",
    namespace: "staking",
    heliusSdkPath: "helius.staking.getUnstakeInstruction",
    compat: "SDK_WRAPPER",
    sinceVersion: null,
    sourceDoc: "https://www.helius.dev/docs/staking/how-to-stake-with-helius-programmatically",
    notes: "Pure local.",
  },
  {
    method: "getWithdrawInstruction",
    namespace: "staking",
    heliusSdkPath: "helius.staking.getWithdrawInstruction",
    compat: "SDK_WRAPPER",
    sinceVersion: null,
    sourceDoc: "https://www.helius.dev/docs/staking/how-to-stake-with-helius-programmatically",
    notes: "Pure local.",
  },
  {
    method: "getWithdrawableAmount",
    namespace: "staking",
    heliusSdkPath: "helius.staking.getWithdrawableAmount",
    compat: "SDK_WRAPPER",
    sinceVersion: null,
    sourceDoc: "https://www.helius.dev/docs/staking/how-to-stake-with-helius-programmatically",
    notes:
      "helius-sdk reads the stake account via getAccountInfo and inspects its state. The getAccountInfo call passes through; the rest is client-side parsing.",
  },
  {
    method: "getHeliusStakeAccounts",
    namespace: "staking",
    heliusSdkPath: "helius.staking.getHeliusStakeAccounts",
    compat: "SDK_WRAPPER",
    sinceVersion: null,
    sourceDoc: "https://www.helius.dev/docs/staking/how-to-stake-with-helius-programmatically",
    notes:
      "helius-sdk calls getProgramAccounts on the Stake program filtered by owner = caller's wallet, then filters for Helius's validator. getProgramAccounts passes through.",
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
  sdkWrapper: number;
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
    sdkWrapper: 0,
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
      case "SDK_WRAPPER":
        s.sdkWrapper++;
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
