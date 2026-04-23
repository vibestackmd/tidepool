//! JSON-RPC method dispatch. The core architectural win: every
//! method we serve maps to a [`Method`] enum variant, and the dispatch
//! function does an **exhaustive** match. Adding a new method = add
//! a variant + a match arm; compiler fails the build if you forget.
//!
//! Methods we don't recognize fall through to `Method::Passthrough`
//! which forwards the raw JSON-RPC envelope to the upstream.

use std::sync::Arc;

use serde_json::{json, Value};
use tracing::warn;

use tidepool_rpc::cache::{CacheStore, SearchFilter};
use tidepool_rpc::cnft::{index_tree, CnftStore, IndexTreeOptions};
use tidepool_rpc::compat::{manifest, summarize};
use tidepool_rpc::compatibility::compatibility;
use tidepool_rpc::das::{
    get_asset_full, get_asset_proof, get_asset_proof_batch, get_assets_by_authority,
    get_assets_by_creator, get_assets_by_group, get_assets_by_owner, get_balances,
    get_nft_editions, get_token_accounts, search_assets, AccountDecoder, TokenAccountsFilter,
};
use tidepool_rpc::enhanced::{
    enrich_token_standards, get_transactions, get_transactions_by_address,
    TransactionsByAddressOptions,
};
use tidepool_rpc::priority_fee::{compute_levels, percentile_at, PriorityLevel};
use tidepool_rpc::upstream::UpstreamClient;
use tidepool_rpc::webhooks::{PostClient, WebhookError, WebhookInput};

use crate::json_rpc::{codes, fail, ok, JsonRpcRequest};
use crate::webhook_runtime::WebhookRuntime;

/// Every method the server handles natively. Anything not listed here
/// is forwarded to the upstream unchanged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    // DAS
    GetAsset,
    GetAssetBatch,
    GetAssetProof,
    GetAssetProofBatch,
    GetAssetsByOwner,
    GetAssetsByAuthority,
    GetAssetsByCreator,
    GetAssetsByGroup,
    SearchAssets,
    GetNftEditions,
    GetTokenAccounts,
    // Helius v2 (cursor-paginated)
    GetProgramAccountsV2,
    GetTokenAccountsByOwnerV2,
    // NOTE: getBalances, createWebhook/getAllWebhooks/... ,
    // getTransactions, and getTransactionsByAddress are deliberately
    // absent from this enum. Helius serves them via REST
    // (`api.helius.xyz/v0/...`), not JSON-RPC. Serving them here
    // would let users write local code that'd fail against real
    // Helius. They live in `crate::rest` instead, routed to the
    // same handler functions (`pub(crate) handle_*`).
    // Tx (Helius-custom)
    GetPriorityFeeEstimate,
    SendTransactionWithSender,
    // Tidepool custom
    TidepoolInfo,
    TidepoolIndexTree,
    TidepoolExportTreeSnapshot,
    TidepoolLoadTreeSnapshot,
}

impl Method {
    /// Try to parse a wire method name. Returns `None` for
    /// methods the server doesn't own — caller falls through to the
    /// passthrough path.
    #[must_use]
    pub fn from_wire(name: &str) -> Option<Self> {
        Some(match name {
            "getAsset" => Self::GetAsset,
            "getAssetBatch" => Self::GetAssetBatch,
            "getAssetProof" => Self::GetAssetProof,
            "getAssetProofBatch" => Self::GetAssetProofBatch,
            "getAssetsByOwner" => Self::GetAssetsByOwner,
            "getAssetsByAuthority" => Self::GetAssetsByAuthority,
            "getAssetsByCreator" => Self::GetAssetsByCreator,
            "getAssetsByGroup" => Self::GetAssetsByGroup,
            "searchAssets" => Self::SearchAssets,
            "getNftEditions" => Self::GetNftEditions,
            "getTokenAccounts" => Self::GetTokenAccounts,
            "getProgramAccountsV2" => Self::GetProgramAccountsV2,
            "getTokenAccountsByOwnerV2" => Self::GetTokenAccountsByOwnerV2,
            "getPriorityFeeEstimate" => Self::GetPriorityFeeEstimate,
            "sendTransactionWithSender" => Self::SendTransactionWithSender,
            "tidepool_info" => Self::TidepoolInfo,
            "tidepool_indexTree" => Self::TidepoolIndexTree,
            "tidepool_exportTreeSnapshot" => Self::TidepoolExportTreeSnapshot,
            "tidepool_loadTreeSnapshot" => Self::TidepoolLoadTreeSnapshot,
            _ => return None,
        })
    }

    /// Reverse — useful for the compat manifest.
    #[must_use]
    pub fn to_wire(self) -> &'static str {
        match self {
            Self::GetAsset => "getAsset",
            Self::GetAssetBatch => "getAssetBatch",
            Self::GetAssetProof => "getAssetProof",
            Self::GetAssetProofBatch => "getAssetProofBatch",
            Self::GetAssetsByOwner => "getAssetsByOwner",
            Self::GetAssetsByAuthority => "getAssetsByAuthority",
            Self::GetAssetsByCreator => "getAssetsByCreator",
            Self::GetAssetsByGroup => "getAssetsByGroup",
            Self::SearchAssets => "searchAssets",
            Self::GetNftEditions => "getNftEditions",
            Self::GetTokenAccounts => "getTokenAccounts",
            Self::GetProgramAccountsV2 => "getProgramAccountsV2",
            Self::GetTokenAccountsByOwnerV2 => "getTokenAccountsByOwnerV2",
            Self::GetPriorityFeeEstimate => "getPriorityFeeEstimate",
            Self::SendTransactionWithSender => "sendTransactionWithSender",
            Self::TidepoolInfo => "tidepool_info",
            Self::TidepoolIndexTree => "tidepool_indexTree",
            Self::TidepoolExportTreeSnapshot => "tidepool_exportTreeSnapshot",
            Self::TidepoolLoadTreeSnapshot => "tidepool_loadTreeSnapshot",
        }
    }
}

/// Shared request-handling context. Wired once at server start and
/// passed to every dispatch call.
pub struct Ctx<S, C, U>
where
    S: CnftStore + ?Sized,
    C: CacheStore + ?Sized,
    U: UpstreamClient + ?Sized + 'static,
{
    pub cnft: Arc<S>,
    pub cache: Arc<C>,
    pub upstream: Arc<U>,
    pub decoders: Arc<[Arc<dyn AccountDecoder>]>,
    pub webhooks: Arc<WebhookRuntime<U, dyn PostClient>>,
}

impl<S, C, U> Clone for Ctx<S, C, U>
where
    S: CnftStore + ?Sized,
    C: CacheStore + ?Sized,
    U: UpstreamClient + ?Sized + 'static,
{
    fn clone(&self) -> Self {
        Self {
            cnft: Arc::clone(&self.cnft),
            cache: Arc::clone(&self.cache),
            upstream: Arc::clone(&self.upstream),
            decoders: Arc::clone(&self.decoders),
            webhooks: Arc::clone(&self.webhooks),
        }
    }
}

/// Dispatch one JSON-RPC request. Returns `Some(response)` when we
/// handled it natively, `None` when the caller should passthrough.
pub async fn dispatch<S, C, U>(
    ctx: &Ctx<S, C, U>,
    req: &JsonRpcRequest,
) -> Option<Value>
where
    S: CnftStore + ?Sized,
    C: CacheStore + ?Sized,
    U: UpstreamClient + ?Sized + 'static,
{
    let method = Method::from_wire(&req.method)?;
    Some(match method {
        Method::GetAsset => handle_get_asset(ctx, req).await,
        Method::GetAssetBatch => handle_get_asset_batch(ctx, req).await,
        Method::GetAssetProof => handle_get_asset_proof(ctx, req).await,
        Method::GetAssetProofBatch => handle_get_asset_proof_batch(ctx, req).await,
        Method::GetAssetsByOwner => handle_get_assets_by_owner(ctx, req).await,
        Method::GetAssetsByAuthority => handle_get_assets_by_authority(ctx, req).await,
        Method::GetAssetsByCreator => handle_get_assets_by_creator(ctx, req).await,
        Method::GetAssetsByGroup => handle_get_assets_by_group(ctx, req).await,
        Method::SearchAssets => handle_search_assets(ctx, req).await,
        Method::GetNftEditions => handle_get_nft_editions(ctx, req).await,
        Method::GetTokenAccounts => handle_get_token_accounts(ctx, req).await,
        // NOTE: getBalances, webhook CRUD, and Enhanced Transactions
        // are reachable only via the REST router (`crate::rest`) —
        // Helius serves them on `api.helius.xyz/v0/...`, not JSON-RPC.
        // The handler functions still live in this file but aren't
        // wired to any `Method` variant here.
        Method::GetProgramAccountsV2 => handle_get_program_accounts_v2(ctx, req).await,
        Method::GetTokenAccountsByOwnerV2 => handle_get_token_accounts_by_owner_v2(ctx, req).await,
        Method::GetPriorityFeeEstimate => handle_get_priority_fee_estimate(ctx, req).await,
        Method::SendTransactionWithSender => handle_send_transaction_with_sender(ctx, req).await,
        Method::TidepoolInfo => handle_tidepool_info(ctx, req).await,
        Method::TidepoolIndexTree => handle_tidepool_index_tree(ctx, req).await,
        Method::TidepoolExportTreeSnapshot => handle_tidepool_export_tree_snapshot(ctx, req).await,
        Method::TidepoolLoadTreeSnapshot => handle_tidepool_load_tree_snapshot(ctx, req).await,
    })
}

// ─── per-method handlers ──────────────────────────────────────────

async fn handle_get_asset<S, C, U>(ctx: &Ctx<S, C, U>, req: &JsonRpcRequest) -> Value
where
    S: CnftStore + ?Sized,
    C: CacheStore + ?Sized,
    U: UpstreamClient + ?Sized + 'static,
{
    let Some(asset_id) = extract_id_param(&req.params) else {
        return fail(&req.id, codes::INVALID_PARAMS, "missing `id` param");
    };
    match get_asset_full(&*ctx.cnft, &*ctx.cache, &*ctx.upstream, &ctx.decoders, &asset_id).await {
        Ok(Some(asset)) => ok(&req.id, serde_json::to_value(asset).unwrap_or(Value::Null)),
        Ok(None) => fail(&req.id, codes::INTERNAL_ERROR, "Asset not found"),
        Err(e) => {
            warn!(method = "getAsset", err = %e, "handler failed");
            fail(&req.id, codes::INTERNAL_ERROR, format!("{e}"))
        }
    }
}

async fn handle_get_asset_batch<S, C, U>(ctx: &Ctx<S, C, U>, req: &JsonRpcRequest) -> Value
where
    S: CnftStore + ?Sized,
    C: CacheStore + ?Sized,
    U: UpstreamClient + ?Sized + 'static,
{
    let Some(ids) = req.params.get("ids").and_then(Value::as_array) else {
        return fail(&req.id, codes::INVALID_PARAMS, "missing `ids` array");
    };
    let ids: Vec<String> = ids
        .iter()
        .filter_map(|v| v.as_str().map(String::from))
        .collect();
    match tidepool_rpc::das::get_asset_batch(
        &*ctx.cnft,
        &*ctx.cache,
        &*ctx.upstream,
        &ctx.decoders,
        &ids,
    )
    .await
    {
        Ok(results) => ok(&req.id, serde_json::to_value(results).unwrap_or(Value::Null)),
        Err(e) => fail(&req.id, codes::INTERNAL_ERROR, format!("{e}")),
    }
}

async fn handle_get_asset_proof<S, C, U>(ctx: &Ctx<S, C, U>, req: &JsonRpcRequest) -> Value
where
    S: CnftStore + ?Sized,
    C: CacheStore + ?Sized,
    U: UpstreamClient + ?Sized + 'static,
{
    let Some(asset_id) = extract_id_param(&req.params) else {
        return fail(&req.id, codes::INVALID_PARAMS, "missing `id` param");
    };
    let Some(id_bytes) = bs58_to_32(&asset_id) else {
        return fail(
            &req.id,
            codes::INVALID_PARAMS,
            "`id` is not a valid 32-byte base58 address",
        );
    };
    match get_asset_proof(&*ctx.cnft, &id_bytes).await {
        Ok(Some(p)) => ok(&req.id, serde_json::to_value(p).unwrap_or(Value::Null)),
        Ok(None) => fail(
            &req.id,
            codes::INTERNAL_ERROR,
            "Asset not found or tree not indexed",
        ),
        Err(e) => fail(&req.id, codes::INTERNAL_ERROR, format!("{e}")),
    }
}

async fn handle_get_asset_proof_batch<S, C, U>(ctx: &Ctx<S, C, U>, req: &JsonRpcRequest) -> Value
where
    S: CnftStore + ?Sized,
    C: CacheStore + ?Sized,
    U: UpstreamClient + ?Sized + 'static,
{
    let Some(ids) = req.params.get("ids").and_then(Value::as_array) else {
        return fail(&req.id, codes::INVALID_PARAMS, "missing `ids` array");
    };
    let id_bytes: Vec<[u8; 32]> = ids
        .iter()
        .filter_map(|v| v.as_str())
        .filter_map(bs58_to_32)
        .collect();
    match get_asset_proof_batch(&*ctx.cnft, &id_bytes).await {
        Ok(results) => {
            let map: serde_json::Map<String, Value> = ids
                .iter()
                .filter_map(|v| v.as_str())
                .zip(results.into_iter())
                .map(|(id, proof)| {
                    (
                        id.to_string(),
                        proof.map_or(Value::Null, |p| {
                            serde_json::to_value(p).unwrap_or(Value::Null)
                        }),
                    )
                })
                .collect();
            ok(&req.id, Value::Object(map))
        }
        Err(e) => fail(&req.id, codes::INTERNAL_ERROR, format!("{e}")),
    }
}

async fn handle_get_assets_by_owner<S, C, U>(ctx: &Ctx<S, C, U>, req: &JsonRpcRequest) -> Value
where
    S: CnftStore + ?Sized,
    C: CacheStore + ?Sized,
    U: UpstreamClient + ?Sized + 'static,
{
    let Some(owner) = req.params.get("ownerAddress").and_then(Value::as_str) else {
        return fail(&req.id, codes::INVALID_PARAMS, "missing `ownerAddress`");
    };
    match get_assets_by_owner(&*ctx.cache, owner).await {
        Ok(items) => ok(&req.id, serde_json::json!({ "items": items })),
        Err(e) => fail(&req.id, codes::INTERNAL_ERROR, format!("{e}")),
    }
}

async fn handle_get_assets_by_authority<S, C, U>(ctx: &Ctx<S, C, U>, req: &JsonRpcRequest) -> Value
where
    S: CnftStore + ?Sized,
    C: CacheStore + ?Sized,
    U: UpstreamClient + ?Sized + 'static,
{
    let Some(authority) = req.params.get("authorityAddress").and_then(Value::as_str) else {
        return fail(&req.id, codes::INVALID_PARAMS, "missing `authorityAddress`");
    };
    match get_assets_by_authority(&*ctx.cache, authority).await {
        Ok(items) => ok(&req.id, json!({ "items": items })),
        Err(e) => fail(&req.id, codes::INTERNAL_ERROR, format!("{e}")),
    }
}

async fn handle_get_assets_by_creator<S, C, U>(ctx: &Ctx<S, C, U>, req: &JsonRpcRequest) -> Value
where
    S: CnftStore + ?Sized,
    C: CacheStore + ?Sized,
    U: UpstreamClient + ?Sized + 'static,
{
    let Some(creator) = req.params.get("creatorAddress").and_then(Value::as_str) else {
        return fail(&req.id, codes::INVALID_PARAMS, "missing `creatorAddress`");
    };
    let only_verified = req
        .params
        .get("onlyVerified")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    match get_assets_by_creator(&*ctx.cache, creator, only_verified).await {
        Ok(items) => ok(&req.id, json!({ "items": items })),
        Err(e) => fail(&req.id, codes::INTERNAL_ERROR, format!("{e}")),
    }
}

async fn handle_get_assets_by_group<S, C, U>(ctx: &Ctx<S, C, U>, req: &JsonRpcRequest) -> Value
where
    S: CnftStore + ?Sized,
    C: CacheStore + ?Sized,
    U: UpstreamClient + ?Sized + 'static,
{
    let gk = req.params.get("groupKey").and_then(Value::as_str);
    let gv = req.params.get("groupValue").and_then(Value::as_str);
    let (Some(gk), Some(gv)) = (gk, gv) else {
        return fail(&req.id, codes::INVALID_PARAMS, "missing `groupKey` / `groupValue`");
    };
    match get_assets_by_group(&*ctx.cache, gk, gv).await {
        Ok(items) => ok(&req.id, json!({ "items": items })),
        Err(e) => fail(&req.id, codes::INTERNAL_ERROR, format!("{e}")),
    }
}

async fn handle_search_assets<S, C, U>(ctx: &Ctx<S, C, U>, req: &JsonRpcRequest) -> Value
where
    S: CnftStore + ?Sized,
    C: CacheStore + ?Sized,
    U: UpstreamClient + ?Sized + 'static,
{
    let filter = SearchFilter {
        owner_address: req.params.get("ownerAddress").and_then(Value::as_str).map(String::from),
        authority_address: req.params.get("authorityAddress").and_then(Value::as_str).map(String::from),
        creator_address: req.params.get("creatorAddress").and_then(Value::as_str).map(String::from),
        creator_verified: req.params.get("creatorVerified").and_then(Value::as_bool),
        grouping: req
            .params
            .get("grouping")
            .and_then(Value::as_array)
            .and_then(|arr| {
                let k = arr.first()?.as_str()?.to_string();
                let v = arr.get(1)?.as_str()?.to_string();
                Some((k, v))
            }),
        interface: req.params.get("interface").and_then(Value::as_str).map(String::from),
        burnt: req.params.get("burnt").and_then(Value::as_bool),
    };
    match search_assets(&*ctx.cache, &filter).await {
        Ok(items) => ok(&req.id, json!({ "items": items })),
        Err(e) => fail(&req.id, codes::INTERNAL_ERROR, format!("{e}")),
    }
}

/// `helius.das.getNftEditions(mint, page, limit)` — serves from the
/// local edition index populated on `getAsset` fetches. Cold-path
/// calls do one upstream fetch of the master mint to warm the index;
/// subsequent calls serve from cache.
async fn handle_get_nft_editions<S, C, U>(ctx: &Ctx<S, C, U>, req: &JsonRpcRequest) -> Value
where
    S: CnftStore + ?Sized,
    C: CacheStore + ?Sized,
    U: UpstreamClient + ?Sized + 'static,
{
    let mint = req
        .params
        .get("mint")
        .or_else(|| req.params.get("id"))
        .and_then(Value::as_str);
    let Some(mint) = mint else {
        return fail(
            &req.id,
            codes::INVALID_PARAMS,
            "getNftEditions requires `mint`",
        );
    };
    let page = req.params.get("page").and_then(Value::as_u64).unwrap_or(1);
    // Helius's default page size is 100.
    let limit = req
        .params
        .get("limit")
        .and_then(Value::as_u64)
        .unwrap_or(100);

    match get_nft_editions(
        &*ctx.cache,
        &*ctx.upstream,
        &ctx.decoders,
        mint,
        page,
        limit,
    )
    .await
    {
        Ok(Some(result)) => ok(&req.id, serde_json::to_value(result).unwrap_or(Value::Null)),
        Ok(None) => ok(&req.id, Value::Null),
        Err(e) => fail(&req.id, codes::INTERNAL_ERROR, format!("{e}")),
    }
}

/// `helius.das.getTokenAccounts(owner?, mint?, page?, limit?,
/// displayOptions.showZeroBalance?)`. Shim — forwards to the upstream
/// RPC (`getTokenAccountsByOwner` or `getProgramAccounts` memcmp),
/// reshapes the response, and paginates locally.
async fn handle_get_token_accounts<S, C, U>(ctx: &Ctx<S, C, U>, req: &JsonRpcRequest) -> Value
where
    S: CnftStore + ?Sized,
    C: CacheStore + ?Sized,
    U: UpstreamClient + ?Sized + 'static,
{
    let owner = req
        .params
        .get("owner")
        .and_then(Value::as_str)
        .map(String::from);
    let mint = req
        .params
        .get("mint")
        .and_then(Value::as_str)
        .map(String::from);
    let page = req.params.get("page").and_then(Value::as_u64).unwrap_or(1);
    let limit = req
        .params
        .get("limit")
        .and_then(Value::as_u64)
        .unwrap_or(100);
    let show_zero_balance = req
        .params
        .pointer("/displayOptions/showZeroBalance")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    let filter = TokenAccountsFilter {
        owner,
        mint,
        page,
        limit,
        show_zero_balance,
    };

    match get_token_accounts(&*ctx.upstream, &filter).await {
        Ok(result) => ok(&req.id, serde_json::to_value(result).unwrap_or(Value::Null)),
        Err(e) => {
            // BadRequest → invalid-params code; everything else is internal.
            let code = match &e {
                tidepool_rpc::das::DasError::BadRequest(_) => codes::INVALID_PARAMS,
                _ => codes::INTERNAL_ERROR,
            };
            fail(&req.id, code, format!("{e}"))
        }
    }
}

/// `helius.wallet.getBalances(owner)` — returns native SOL + all
/// SPL/Token-2022 positions the wallet holds. Shim — fans out to
/// `getBalance` + one `getTokenAccountsByOwner` per program.
pub(crate) async fn handle_get_balances<S, C, U>(ctx: &Ctx<S, C, U>, req: &JsonRpcRequest) -> Value
where
    S: CnftStore + ?Sized,
    C: CacheStore + ?Sized,
    U: UpstreamClient + ?Sized + 'static,
{
    // Helius accepts either `owner` or positional `[owner]`.
    let owner = req
        .params
        .get("owner")
        .and_then(Value::as_str)
        .or_else(|| {
            req.params
                .as_array()
                .and_then(|a| a.first())
                .and_then(Value::as_str)
        });
    let Some(owner) = owner else {
        return fail(
            &req.id,
            codes::INVALID_PARAMS,
            "getBalances requires `owner`",
        );
    };
    match get_balances(&*ctx.upstream, owner).await {
        Ok(result) => ok(&req.id, serde_json::to_value(result).unwrap_or(Value::Null)),
        Err(e) => fail(&req.id, codes::INTERNAL_ERROR, format!("{e}")),
    }
}

// ─── Webhook CRUD ──────────────────────────────────────────────────
// All five handlers front the `WebhookRuntime` on the Ctx — creation
// spawns a per-webhook polling task; deletion aborts it; edit
// restarts the task with the new config.

fn parse_webhook_input(params: &Value) -> WebhookInput {
    // Accept both Helius's camelCase (`webhookURL`, `accountAddresses`,
    // `transactionTypes`, `txnStatus`, `webhookType`, `authHeader`)
    // wire keys and our snake_case serde defaults.
    let url = params
        .get("webhookURL")
        .or_else(|| params.get("webhook_url"))
        .and_then(Value::as_str)
        .map(String::from);
    let addresses = params
        .get("accountAddresses")
        .or_else(|| params.get("account_addresses"))
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        });
    let transaction_types = params
        .get("transactionTypes")
        .or_else(|| params.get("transaction_types"))
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    let txn_status = params
        .get("txnStatus")
        .or_else(|| params.get("txn_status"))
        .and_then(Value::as_str)
        .map(String::from);
    let webhook_type = params
        .get("webhookType")
        .or_else(|| params.get("webhook_type"))
        .and_then(Value::as_str)
        .map(String::from);
    let auth_header = params
        .get("authHeader")
        .or_else(|| params.get("auth_header"))
        .and_then(Value::as_str)
        .map(String::from);
    WebhookInput {
        webhook_url: url,
        account_addresses: addresses,
        transaction_types,
        txn_status,
        webhook_type,
        auth_header,
    }
}

fn webhook_error_to_response(id: &Value, e: &WebhookError) -> Value {
    let code = match e {
        WebhookError::BadRequest(_) => codes::INVALID_PARAMS,
        WebhookError::NotFound { .. } => codes::INTERNAL_ERROR,
    };
    fail(id, code, format!("{e}"))
}

pub(crate) async fn handle_create_webhook<S, C, U>(ctx: &Ctx<S, C, U>, req: &JsonRpcRequest) -> Value
where
    S: CnftStore + ?Sized,
    C: CacheStore + ?Sized,
    U: UpstreamClient + ?Sized + 'static,
{
    let input = parse_webhook_input(&req.params);
    match ctx.webhooks.create(input).await {
        Ok(wh) => ok(&req.id, serde_json::to_value(wh).unwrap_or(Value::Null)),
        Err(e) => webhook_error_to_response(&req.id, &e),
    }
}

pub(crate) async fn handle_get_all_webhooks<S, C, U>(ctx: &Ctx<S, C, U>, req: &JsonRpcRequest) -> Value
where
    S: CnftStore + ?Sized,
    C: CacheStore + ?Sized,
    U: UpstreamClient + ?Sized + 'static,
{
    match ctx.webhooks.list().await {
        Ok(items) => ok(&req.id, serde_json::to_value(items).unwrap_or(Value::Null)),
        Err(e) => webhook_error_to_response(&req.id, &e),
    }
}

pub(crate) async fn handle_get_webhook_by_id<S, C, U>(ctx: &Ctx<S, C, U>, req: &JsonRpcRequest) -> Value
where
    S: CnftStore + ?Sized,
    C: CacheStore + ?Sized,
    U: UpstreamClient + ?Sized + 'static,
{
    let id = req
        .params
        .get("webhookID")
        .or_else(|| req.params.get("webhook_id"))
        .or_else(|| req.params.get("id"))
        .and_then(Value::as_str);
    let Some(id) = id else {
        return fail(&req.id, codes::INVALID_PARAMS, "missing `webhookID`");
    };
    match ctx.webhooks.get(id).await {
        Ok(Some(wh)) => ok(&req.id, serde_json::to_value(wh).unwrap_or(Value::Null)),
        Ok(None) => ok(&req.id, Value::Null),
        Err(e) => webhook_error_to_response(&req.id, &e),
    }
}

pub(crate) async fn handle_edit_webhook<S, C, U>(ctx: &Ctx<S, C, U>, req: &JsonRpcRequest) -> Value
where
    S: CnftStore + ?Sized,
    C: CacheStore + ?Sized,
    U: UpstreamClient + ?Sized + 'static,
{
    let id = req
        .params
        .get("webhookID")
        .or_else(|| req.params.get("webhook_id"))
        .or_else(|| req.params.get("id"))
        .and_then(Value::as_str);
    let Some(id) = id else {
        return fail(&req.id, codes::INVALID_PARAMS, "missing `webhookID`");
    };
    let input = parse_webhook_input(&req.params);
    match ctx.webhooks.edit(id, input).await {
        Ok(wh) => ok(&req.id, serde_json::to_value(wh).unwrap_or(Value::Null)),
        Err(e) => webhook_error_to_response(&req.id, &e),
    }
}

pub(crate) async fn handle_delete_webhook<S, C, U>(ctx: &Ctx<S, C, U>, req: &JsonRpcRequest) -> Value
where
    S: CnftStore + ?Sized,
    C: CacheStore + ?Sized,
    U: UpstreamClient + ?Sized + 'static,
{
    let id = req
        .params
        .get("webhookID")
        .or_else(|| req.params.get("webhook_id"))
        .or_else(|| req.params.get("id"))
        .and_then(Value::as_str);
    let Some(id) = id else {
        return fail(&req.id, codes::INVALID_PARAMS, "missing `webhookID`");
    };
    match ctx.webhooks.delete(id).await {
        Ok(removed) => ok(&req.id, json!({ "deleted": removed })),
        Err(e) => webhook_error_to_response(&req.id, &e),
    }
}

// ─── Enhanced Transactions ─────────────────────────────────────────

/// `helius.enhanced.getTransactions([signature, ...])`. Fans out one
/// `getTransaction` per signature and classifies each.
pub(crate) async fn handle_get_transactions<S, C, U>(ctx: &Ctx<S, C, U>, req: &JsonRpcRequest) -> Value
where
    S: CnftStore + ?Sized,
    C: CacheStore + ?Sized,
    U: UpstreamClient + ?Sized + 'static,
{
    let sigs: Vec<String> = req
        .params
        .get("signatures")
        .or_else(|| {
            // positional fallback: first param may be the array.
            req.params.as_array().and_then(|a| a.first())
        })
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    if sigs.is_empty() {
        return fail(
            &req.id,
            codes::INVALID_PARAMS,
            "getTransactions requires a non-empty `signatures` array",
        );
    }
    let mut out = get_transactions(&*ctx.upstream, &sigs).await;
    // Opportunistic enrichment: if a transfer's mint is already in
    // the DAS cache, we know its tokenStandard without another
    // upstream hop. Misses stay None (skip-on-serialize).
    enrich_token_standards(&*ctx.cache, &mut out).await;
    ok(&req.id, serde_json::to_value(out).unwrap_or(Value::Null))
}

/// `helius.enhanced.getTransactionsByAddress(address, options)`.
/// Resolves signatures via `getSignaturesForAddress` then fans out.
pub(crate) async fn handle_get_transactions_by_address<S, C, U>(
    ctx: &Ctx<S, C, U>,
    req: &JsonRpcRequest,
) -> Value
where
    S: CnftStore + ?Sized,
    C: CacheStore + ?Sized,
    U: UpstreamClient + ?Sized + 'static,
{
    let Some(address) = req
        .params
        .get("address")
        .and_then(Value::as_str)
        .map(String::from)
    else {
        return fail(
            &req.id,
            codes::INVALID_PARAMS,
            "getTransactionsByAddress requires `address`",
        );
    };
    let options = TransactionsByAddressOptions {
        before: req
            .params
            .get("before")
            .and_then(Value::as_str)
            .map(String::from),
        until: req
            .params
            .get("until")
            .and_then(Value::as_str)
            .map(String::from),
        limit: req.params.get("limit").and_then(Value::as_u64),
        types: req
            .params
            .get("type")
            .and_then(Value::as_str)
            .map(|s| vec![s.to_string()])
            .or_else(|| {
                req.params
                    .get("types")
                    .and_then(Value::as_array)
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|v| v.as_str().map(String::from))
                            .collect()
                    })
            })
            .unwrap_or_default(),
    };
    let mut out = get_transactions_by_address(&*ctx.upstream, &address, &options).await;
    enrich_token_standards(&*ctx.cache, &mut out).await;
    ok(&req.id, serde_json::to_value(out).unwrap_or(Value::Null))
}

/// `getProgramAccountsV2` — cursor-paginated passthrough over
/// `getProgramAccounts`. Forwards user-supplied filters / dataSlice /
/// encoding verbatim to the upstream, sorts by pubkey for stable
/// pagination, then slices by `cursor` + `limit`. Returns the next
/// cursor only when there's more data.
async fn handle_get_program_accounts_v2<S, C, U>(
    ctx: &Ctx<S, C, U>,
    req: &JsonRpcRequest,
) -> Value
where
    S: CnftStore + ?Sized,
    C: CacheStore + ?Sized,
    U: UpstreamClient + ?Sized + 'static,
{
    let Some(program_id) = req
        .params
        .get("programId")
        .and_then(Value::as_str)
        .map(String::from)
    else {
        return fail(
            &req.id,
            codes::INVALID_PARAMS,
            "getProgramAccountsV2 requires `programId`",
        );
    };

    let mut cfg = serde_json::Map::new();
    // Forward standard-RPC config fields verbatim when provided.
    for key in [
        "encoding",
        "commitment",
        "filters",
        "dataSlice",
        "minContextSlot",
    ] {
        if let Some(v) = req.params.get(key) {
            cfg.insert(key.to_string(), v.clone());
        }
    }
    let params = json!([program_id, Value::Object(cfg)]);

    let raw = match ctx.upstream.rpc_call("getProgramAccounts", params).await {
        Ok(r) => r,
        Err(e) => {
            return fail(
                &req.id,
                codes::INTERNAL_ERROR,
                format!("upstream getProgramAccounts failed: {e}"),
            );
        }
    };

    let cursor = req
        .params
        .get("cursor")
        .and_then(Value::as_str)
        .map(String::from);
    let limit = req
        .params
        .get("limit")
        .and_then(Value::as_u64)
        .unwrap_or(1000);

    let response = build_cursor_page(&raw, cursor.as_deref(), limit);
    ok(&req.id, response)
}

/// `getTokenAccountsByOwnerV2` — cursor-paginated passthrough over
/// `getTokenAccountsByOwner`. Same cursor semantics as V2 above.
async fn handle_get_token_accounts_by_owner_v2<S, C, U>(
    ctx: &Ctx<S, C, U>,
    req: &JsonRpcRequest,
) -> Value
where
    S: CnftStore + ?Sized,
    C: CacheStore + ?Sized,
    U: UpstreamClient + ?Sized + 'static,
{
    let Some(owner) = req
        .params
        .get("owner")
        .and_then(Value::as_str)
        .map(String::from)
    else {
        return fail(
            &req.id,
            codes::INVALID_PARAMS,
            "getTokenAccountsByOwnerV2 requires `owner`",
        );
    };

    // Underlying RPC wants one of { mint } or { programId } as its
    // filter object. Prefer mint when both are given (more specific).
    let filter_obj = if let Some(mint) = req.params.get("mint").and_then(Value::as_str) {
        json!({ "mint": mint })
    } else if let Some(program_id) = req.params.get("programId").and_then(Value::as_str) {
        json!({ "programId": program_id })
    } else {
        return fail(
            &req.id,
            codes::INVALID_PARAMS,
            "getTokenAccountsByOwnerV2 requires `mint` or `programId`",
        );
    };

    let mut cfg = serde_json::Map::new();
    for key in ["encoding", "commitment", "minContextSlot"] {
        if let Some(v) = req.params.get(key) {
            cfg.insert(key.to_string(), v.clone());
        }
    }
    let params = json!([owner, filter_obj, Value::Object(cfg)]);

    let raw = match ctx
        .upstream
        .rpc_call("getTokenAccountsByOwner", params)
        .await
    {
        Ok(r) => r,
        Err(e) => {
            return fail(
                &req.id,
                codes::INTERNAL_ERROR,
                format!("upstream getTokenAccountsByOwner failed: {e}"),
            );
        }
    };

    let cursor = req
        .params
        .get("cursor")
        .and_then(Value::as_str)
        .map(String::from);
    let limit = req
        .params
        .get("limit")
        .and_then(Value::as_u64)
        .unwrap_or(1000);

    let response = build_cursor_page(&raw, cursor.as_deref(), limit);
    ok(&req.id, response)
}

/// Slice the upstream's `[{pubkey, account}, ...]` (or
/// `{context, value: [...]}`) payload into a cursor-paginated page.
/// Cursor = last pubkey of the previous page; items are ordered
/// lexicographically by pubkey so a cursor-based walk is deterministic.
fn build_cursor_page(raw: &[u8], cursor: Option<&str>, limit: u64) -> Value {
    let parsed: Value = serde_json::from_slice(raw).unwrap_or(Value::Null);
    let array = if let Some(inner) = parsed.get("value") {
        inner.as_array().cloned().unwrap_or_default()
    } else {
        parsed.as_array().cloned().unwrap_or_default()
    };

    let mut sorted = array;
    sorted.sort_by(|a, b| {
        let ak = a.get("pubkey").and_then(Value::as_str).unwrap_or("");
        let bk = b.get("pubkey").and_then(Value::as_str).unwrap_or("");
        ak.cmp(bk)
    });

    // Apply cursor: drop everything at or before the given pubkey.
    let mut filtered: Vec<Value> = if let Some(c) = cursor {
        sorted
            .into_iter()
            .filter(|entry| {
                entry
                    .get("pubkey")
                    .and_then(Value::as_str)
                    .is_some_and(|pk| pk > c)
            })
            .collect()
    } else {
        sorted
    };

    // Limit.
    let limit_usize = usize::try_from(limit.max(1)).unwrap_or(1000);
    let has_more = filtered.len() > limit_usize;
    filtered.truncate(limit_usize);
    let next_cursor = if has_more {
        filtered
            .last()
            .and_then(|e| e.get("pubkey"))
            .and_then(Value::as_str)
            .map(String::from)
    } else {
        None
    };

    match next_cursor {
        Some(c) => json!({ "items": filtered, "cursor": c }),
        None => json!({ "items": filtered }),
    }
}

/// `helius.tx.sendTransactionWithSender`.
///
/// Real Helius routes the tx through its parallel Jito-relay fleet
/// for faster landing. Locally we can't reproduce the fleet, so we
/// shim by forwarding the tx to the upstream's plain
/// `sendTransaction`. Callers get a signature back; inclusion latency
/// is whatever the local validator produces.
///
/// Params mirror `sendTransaction` and forward untouched. Helius-
/// specific knobs like `skipPreflight` or Jito-tip addresses flow
/// through to Surfpool, which either supports or silently ignores them.
async fn handle_send_transaction_with_sender<S, C, U>(
    ctx: &Ctx<S, C, U>,
    req: &JsonRpcRequest,
) -> Value
where
    S: CnftStore + ?Sized,
    C: CacheStore + ?Sized,
    U: UpstreamClient + ?Sized + 'static,
{
    match ctx
        .upstream
        .rpc_call("sendTransaction", req.params.clone())
        .await
    {
        Ok(raw) => {
            let result: Value = serde_json::from_slice(&raw).unwrap_or(Value::Null);
            ok(&req.id, result)
        }
        Err(e) => fail(
            &req.id,
            codes::INTERNAL_ERROR,
            format!("upstream sendTransaction failed: {e}"),
        ),
    }
}

/// `helius.tx.getPriorityFeeEstimate` — computes percentiles locally
/// over `getRecentPrioritizationFees` samples. On Surfpool (local, no
/// contention) the upstream returns an empty array and every level is
/// 0, which is the correct answer for a no-contention environment.
///
/// Supports both response shapes:
/// - `includeAllPriorityFeeLevels: true` → `{ priorityFeeLevels: {...} }`
/// - otherwise → `{ priorityFeeEstimate: <single number> }` using the
///   requested `priorityLevel` (defaults to `medium`).
async fn handle_get_priority_fee_estimate<S, C, U>(
    ctx: &Ctx<S, C, U>,
    req: &JsonRpcRequest,
) -> Value
where
    S: CnftStore + ?Sized,
    C: CacheStore + ?Sized,
    U: UpstreamClient + ?Sized + 'static,
{
    // Helius accepts params as [{ accountKeys, options }]; also tolerant
    // of the bare object form { accountKeys, options }.
    let params_obj = match &req.params {
        Value::Array(a) => a.first().cloned().unwrap_or(Value::Null),
        other => other.clone(),
    };
    let account_keys: Vec<String> = params_obj
        .get("accountKeys")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    let options = params_obj.get("options").cloned().unwrap_or(Value::Null);
    let include_all = options
        .get("includeAllPriorityFeeLevels")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    // Fetch recent prioritization fees via the upstream. Surfpool (no
    // contention) returns []; real devnet/mainnet returns up to 150
    // samples.
    let upstream_params = if account_keys.is_empty() {
        json!([])
    } else {
        json!([account_keys])
    };
    let raw = match ctx
        .upstream
        .rpc_call("getRecentPrioritizationFees", upstream_params)
        .await
    {
        Ok(r) => r,
        Err(e) => {
            return fail(
                &req.id,
                codes::INTERNAL_ERROR,
                format!("upstream getRecentPrioritizationFees failed: {e}"),
            );
        }
    };
    // Result shape: [{ slot: u64, prioritizationFee: u64 }, ...]
    let fees: Vec<u64> = serde_json::from_slice::<Value>(&raw)
        .ok()
        .and_then(|v| v.as_array().cloned())
        .map(|arr| {
            arr.iter()
                .filter_map(|entry| entry.get("prioritizationFee").and_then(Value::as_u64))
                .collect()
        })
        .unwrap_or_default();

    let levels = compute_levels(&fees);

    if include_all {
        ok(
            &req.id,
            json!({
                "priorityFeeLevels": levels,
            }),
        )
    } else {
        // Single-level response path. Parse priorityLevel ("medium" by
        // default) and resolve against the sorted distribution.
        let level: PriorityLevel = options
            .get("priorityLevel")
            .and_then(Value::as_str)
            .and_then(|s| match s {
                "min" | "Min" => Some(PriorityLevel::Min),
                "low" | "Low" => Some(PriorityLevel::Low),
                "medium" | "Medium" => Some(PriorityLevel::Medium),
                "high" | "High" => Some(PriorityLevel::High),
                "veryHigh" | "VeryHigh" => Some(PriorityLevel::VeryHigh),
                "unsafeMax" | "UnsafeMax" => Some(PriorityLevel::UnsafeMax),
                _ => None,
            })
            .unwrap_or(PriorityLevel::Medium);
        let mut sorted = fees;
        sorted.sort_unstable();
        let estimate = percentile_at(&sorted, level);
        ok(
            &req.id,
            json!({
                "priorityFeeEstimate": estimate,
            }),
        )
    }
}

// `async` for symmetry with the other handler signatures — `dispatch`
// awaits every variant uniformly. Pure fn-equivalent handlers don't
// need to await anything.
#[allow(clippy::unused_async)]
async fn handle_tidepool_info<S, C, U>(_ctx: &Ctx<S, C, U>, req: &JsonRpcRequest) -> Value
where
    S: CnftStore + ?Sized,
    C: CacheStore + ?Sized,
    U: UpstreamClient + ?Sized + 'static,
{
    let methods = manifest();
    let summary = summarize(methods);
    ok(
        &req.id,
        json!({
            "name": "tidepool-rpc",
            "version": env!("CARGO_PKG_VERSION"),
            "methods": methods,
            "summary": summary,
            // Upstream pins this release was tested against. Parsed
            // at compile time from `compatibility.toml` — see
            // `crates/service/src/compatibility.rs`.
            "compatibility": compatibility(),
        }),
    )
}

async fn handle_tidepool_index_tree<S, C, U>(
    ctx: &Ctx<S, C, U>,
    req: &JsonRpcRequest,
) -> Value
where
    S: CnftStore + ?Sized,
    C: CacheStore + ?Sized,
    U: UpstreamClient + ?Sized + 'static,
{
    let Some(tree_b58) = req.params.get("tree").and_then(Value::as_str) else {
        return fail(&req.id, codes::INVALID_PARAMS, "missing `tree` param");
    };
    let Some(tree_bytes) = bs58_to_32(tree_b58) else {
        return fail(&req.id, codes::INVALID_PARAMS, "`tree` is not a valid 32-byte base58 address");
    };
    let opts = IndexTreeOptions::default();
    match index_tree(&*ctx.upstream, &*ctx.cnft, tree_bytes, &opts).await {
        Ok(result) => ok(
            &req.id,
            json!({
                "tree": tree_b58,
                "processed": result.processed,
                "applied": result.applied,
                "skipped": result.skipped,
            }),
        ),
        Err(e) => fail(&req.id, codes::INTERNAL_ERROR, format!("Index failed: {e}")),
    }
}

/// `tidepool_exportTreeSnapshot` — export one tree's indexed state as
/// a wire envelope the caller can save + later feed back via
/// `tidepool_loadTreeSnapshot` or the CLI's `--snapshot` flag.
/// Returns `null` when the tree isn't registered.
///
/// Shape mirrors Surfpool's `surfnet_exportSnapshot` but scoped to
/// cNFT tree state (our data model, not SVM accounts).
async fn handle_tidepool_export_tree_snapshot<S, C, U>(
    ctx: &Ctx<S, C, U>,
    req: &JsonRpcRequest,
) -> Value
where
    S: CnftStore + ?Sized,
    C: CacheStore + ?Sized,
    U: UpstreamClient + ?Sized + 'static,
{
    let Some(tree_b58) = req.params.get("tree").and_then(Value::as_str) else {
        return fail(&req.id, codes::INVALID_PARAMS, "missing `tree` param");
    };
    let Some(tree_bytes) = bs58_to_32(tree_b58) else {
        return fail(&req.id, codes::INVALID_PARAMS, "`tree` is not a valid 32-byte base58 address");
    };
    match tidepool_rpc::cnft::dump_tree(&*ctx.cnft, &tree_bytes).await {
        Ok(Some(snapshot)) => {
            let blob = tidepool_rpc::cnft::SnapshotBlob::from_tree(&snapshot);
            ok(
                &req.id,
                json!({
                    "tree": tree_b58,
                    "leafCount": snapshot.leaves.len(),
                    "lastSignature": snapshot.last_signature,
                    "snapshot": blob,
                }),
            )
        }
        Ok(None) => ok(&req.id, Value::Null),
        Err(e) => fail(&req.id, codes::INTERNAL_ERROR, format!("dump failed: {e}")),
    }
}

/// `tidepool_loadTreeSnapshot` — apply a previously-exported snapshot
/// to the local store. Overwrites any existing state for the tree.
async fn handle_tidepool_load_tree_snapshot<S, C, U>(
    ctx: &Ctx<S, C, U>,
    req: &JsonRpcRequest,
) -> Value
where
    S: CnftStore + ?Sized,
    C: CacheStore + ?Sized,
    U: UpstreamClient + ?Sized + 'static,
{
    let Some(snapshot_v) = req.params.get("snapshot") else {
        return fail(&req.id, codes::INVALID_PARAMS, "missing `snapshot` param");
    };
    let blob: tidepool_rpc::cnft::SnapshotBlob =
        match serde_json::from_value(snapshot_v.clone()) {
            Ok(b) => b,
            Err(e) => return fail(&req.id, codes::INVALID_PARAMS, format!("snapshot envelope: {e}")),
        };
    let snapshot = match blob.into_tree_snapshot() {
        Ok(s) => s,
        Err(e) => return fail(&req.id, codes::INVALID_PARAMS, e),
    };
    match tidepool_rpc::cnft::load_tree(&*ctx.cnft, snapshot).await {
        Ok(summary) => ok(
            &req.id,
            json!({
                "tree": bs58::encode(summary.tree).into_string(),
                "leafCount": summary.leaf_count,
            }),
        ),
        Err(e) => fail(&req.id, codes::INTERNAL_ERROR, format!("load failed: {e}")),
    }
}

// ─── param helpers ────────────────────────────────────────────────

fn extract_id_param(params: &Value) -> Option<String> {
    // Accept both `{ "id": "..." }` and `[...]` positional forms.
    if let Some(id) = params.get("id").and_then(Value::as_str) {
        return Some(id.to_string());
    }
    if let Some(arr) = params.as_array() {
        if let Some(id) = arr.first().and_then(Value::as_str) {
            return Some(id.to_string());
        }
    }
    None
}

fn bs58_to_32(s: &str) -> Option<[u8; 32]> {
    let v = bs58::decode(s).into_vec().ok()?;
    v.try_into().ok()
}
