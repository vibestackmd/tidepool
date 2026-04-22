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
use tidepool_rpc::das::{
    get_asset_full, get_asset_proof, get_asset_proof_batch, get_assets_by_authority,
    get_assets_by_creator, get_assets_by_group, get_assets_by_owner, search_assets,
    AccountDecoder,
};
use tidepool_rpc::upstream::UpstreamClient;

use crate::json_rpc::{codes, fail, ok, JsonRpcRequest};

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
    // Tidepool custom
    SurfpoolHeliusInfo,
    SurfpoolHeliusIndexTree,
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
            "surfpoolHeliusInfo" => Self::SurfpoolHeliusInfo,
            "surfpoolHeliusIndexTree" => Self::SurfpoolHeliusIndexTree,
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
            Self::SurfpoolHeliusInfo => "surfpoolHeliusInfo",
            Self::SurfpoolHeliusIndexTree => "surfpoolHeliusIndexTree",
        }
    }
}

/// Shared request-handling context. Wired once at server start and
/// passed to every dispatch call.
pub struct Ctx<S, C, U>
where
    S: CnftStore + ?Sized,
    C: CacheStore + ?Sized,
    U: UpstreamClient + ?Sized,
{
    pub cnft: Arc<S>,
    pub cache: Arc<C>,
    pub upstream: Arc<U>,
    pub decoders: Arc<[Arc<dyn AccountDecoder>]>,
}

impl<S, C, U> Clone for Ctx<S, C, U>
where
    S: CnftStore + ?Sized,
    C: CacheStore + ?Sized,
    U: UpstreamClient + ?Sized,
{
    fn clone(&self) -> Self {
        Self {
            cnft: Arc::clone(&self.cnft),
            cache: Arc::clone(&self.cache),
            upstream: Arc::clone(&self.upstream),
            decoders: Arc::clone(&self.decoders),
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
    U: UpstreamClient + ?Sized,
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
        Method::SurfpoolHeliusInfo => handle_surfpool_helius_info(ctx, req).await,
        Method::SurfpoolHeliusIndexTree => handle_surfpool_helius_index_tree(ctx, req).await,
    })
}

// ─── per-method handlers ──────────────────────────────────────────

async fn handle_get_asset<S, C, U>(ctx: &Ctx<S, C, U>, req: &JsonRpcRequest) -> Value
where
    S: CnftStore + ?Sized,
    C: CacheStore + ?Sized,
    U: UpstreamClient + ?Sized,
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
    U: UpstreamClient + ?Sized,
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
    U: UpstreamClient + ?Sized,
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
    U: UpstreamClient + ?Sized,
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
    U: UpstreamClient + ?Sized,
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
    U: UpstreamClient + ?Sized,
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
    U: UpstreamClient + ?Sized,
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
    U: UpstreamClient + ?Sized,
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
    U: UpstreamClient + ?Sized,
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

// `async` for symmetry with the other handler signatures — `dispatch`
// awaits every variant uniformly. Pure fn-equivalent handlers don't
// need to await anything.
#[allow(clippy::unused_async)]
async fn handle_surfpool_helius_info<S, C, U>(_ctx: &Ctx<S, C, U>, req: &JsonRpcRequest) -> Value
where
    S: CnftStore + ?Sized,
    C: CacheStore + ?Sized,
    U: UpstreamClient + ?Sized,
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
        }),
    )
}

async fn handle_surfpool_helius_index_tree<S, C, U>(
    ctx: &Ctx<S, C, U>,
    req: &JsonRpcRequest,
) -> Value
where
    S: CnftStore + ?Sized,
    C: CacheStore + ?Sized,
    U: UpstreamClient + ?Sized,
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
