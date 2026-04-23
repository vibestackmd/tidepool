//! REST transport. Mirrors the paths `helius-sdk` hits on
//! `api.helius.xyz/v0/...` — served from the same base URL as our
//! JSON-RPC endpoint so a user pointing `helius-sdk` at Tidepool
//! gets every method, on the transport the SDK expects.
//!
//! Parity rule: if Helius serves a method over REST, we serve it
//! over REST. No method lives on both transports — clients should be
//! unable to write local code that'd fail against real Helius.
//!
//! Implementation: each REST route synthesizes a `JsonRpcRequest`
//! internally, calls the shared handler function from `dispatcher.rs`,
//! and unwraps the result (or surfaces the JSON-RPC error as a REST
//! error body + appropriate HTTP status). Lets us keep all business
//! logic in one place.

use std::sync::Arc;

use axum::{
    extract::Path,
    Extension,
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use serde_json::{json, Value};

use tidepool_rpc::cache::CacheStore;
use tidepool_rpc::cnft::CnftStore;
use tidepool_rpc::upstream::UpstreamClient;

use crate::dispatcher::{
    handle_create_webhook, handle_delete_webhook, handle_edit_webhook, handle_get_all_webhooks,
    handle_get_balances, handle_get_transactions, handle_get_transactions_by_address,
    handle_get_webhook_by_id, Ctx,
};
use crate::json_rpc::{codes, JsonRpcRequest};

type RestCtx = Ctx<dyn CnftStore, dyn CacheStore, dyn UpstreamClient>;

/// Mount the REST routes. Paths mirror Helius's public REST API
/// exactly so a redirected base URL drops straight in.
///
/// The `Arc<RestCtx>` is injected via `axum::Extension` at the parent
/// router level, not baked in here — keeps the router state-type-
/// agnostic so it composes with any parent that can layer the
/// extension. Axum 0.8 idiomatic pattern for shared state.
pub fn router<S: Clone + Send + Sync + 'static>() -> Router<S> {
    Router::new()
        // Wallet API.
        .route("/v0/addresses/{address}/balances", get(get_balances_rest))
        // Enhanced Transactions.
        .route(
            "/v0/addresses/{address}/transactions",
            get(get_transactions_by_address_rest),
        )
        .route("/v0/transactions", post(get_transactions_rest))
        // Webhooks CRUD.
        .route(
            "/v0/webhooks",
            get(list_webhooks_rest).post(create_webhook_rest),
        )
        .route(
            "/v0/webhooks/{id}",
            get(get_webhook_rest)
                .put(edit_webhook_rest)
                .delete(delete_webhook_rest),
        )
}

// ─── per-route handlers ────────────────────────────────────────────
// Each synthesizes a JSON-RPC-shaped request, delegates to the shared
// handler from `dispatcher.rs`, and returns a REST-shape response
// body (plain result; no JSON-RPC envelope).

async fn get_balances_rest(
    Path(address): Path<String>,
    Extension(ctx): Extension<Arc<RestCtx>>,
) -> Response {
    let req = synth_request(
        "getBalances",
        json!([address]),
    );
    let resp = handle_get_balances(&*ctx, &req).await;
    rest_response_from_rpc(resp)
}

async fn get_transactions_by_address_rest(
    Path(address): Path<String>,
    Extension(ctx): Extension<Arc<RestCtx>>,
) -> Response {
    let req = synth_request(
        "getTransactionsByAddress",
        json!({ "address": address }),
    );
    let resp = handle_get_transactions_by_address(&*ctx, &req).await;
    rest_response_from_rpc(resp)
}

async fn get_transactions_rest(
    Extension(ctx): Extension<Arc<RestCtx>>,
    Json(body): Json<Value>,
) -> Response {
    // REST body is typically `{ "transactions": ["sig1", "sig2"] }`;
    // reshape to the JSON-RPC `signatures` key our handler expects.
    let sigs = body
        .get("transactions")
        .cloned()
        .or_else(|| body.get("signatures").cloned())
        .unwrap_or(Value::Array(Vec::new()));
    let req = synth_request("getTransactions", json!({ "signatures": sigs }));
    let resp = handle_get_transactions(&*ctx, &req).await;
    rest_response_from_rpc(resp)
}

async fn create_webhook_rest(
    Extension(ctx): Extension<Arc<RestCtx>>,
    Json(body): Json<Value>,
) -> Response {
    let req = synth_request("createWebhook", body);
    let resp = handle_create_webhook(&*ctx, &req).await;
    rest_response_from_rpc(resp)
}

async fn list_webhooks_rest(Extension(ctx): Extension<Arc<RestCtx>>) -> Response {
    let req = synth_request("getAllWebhooks", Value::Null);
    let resp = handle_get_all_webhooks(&*ctx, &req).await;
    rest_response_from_rpc(resp)
}

async fn get_webhook_rest(
    Path(id): Path<String>,
    Extension(ctx): Extension<Arc<RestCtx>>,
) -> Response {
    let req = synth_request("getWebhookByID", json!({ "webhookID": id }));
    let resp = handle_get_webhook_by_id(&*ctx, &req).await;
    rest_response_from_rpc(resp)
}

async fn edit_webhook_rest(
    Path(id): Path<String>,
    Extension(ctx): Extension<Arc<RestCtx>>,
    Json(mut body): Json<Value>,
) -> Response {
    // Fold the URL id into the body so the handler sees both.
    if let Value::Object(ref mut map) = body {
        map.insert("webhookID".into(), Value::String(id.clone()));
    } else {
        body = json!({ "webhookID": id });
    }
    let req = synth_request("editWebhook", body);
    let resp = handle_edit_webhook(&*ctx, &req).await;
    rest_response_from_rpc(resp)
}

async fn delete_webhook_rest(
    Path(id): Path<String>,
    Extension(ctx): Extension<Arc<RestCtx>>,
) -> Response {
    let req = synth_request("deleteWebhook", json!({ "webhookID": id }));
    let resp = handle_delete_webhook(&*ctx, &req).await;
    rest_response_from_rpc(resp)
}

// ─── helpers ───────────────────────────────────────────────────────

fn synth_request(method: &str, params: Value) -> JsonRpcRequest {
    JsonRpcRequest {
        jsonrpc: Some("2.0".into()),
        id: Value::from(0),
        method: method.into(),
        params,
    }
}

/// Unwrap the internal JSON-RPC response into REST shape. Success
/// returns the `result` field (or the whole thing if there's no
/// envelope); error surfaces as JSON body with a 4xx/5xx status.
fn rest_response_from_rpc(mut resp: Value) -> Response {
    if let Some(err) = resp.get_mut("error").map(Value::take) {
        let code = err
            .get("code")
            .and_then(Value::as_i64)
            .unwrap_or(i64::from(codes::INTERNAL_ERROR));
        // Map JSON-RPC error codes to HTTP status. -32602 (Invalid
        // params) → 400, everything else → 500.
        let status = match code {
            -32602 => StatusCode::BAD_REQUEST,
            -32601 => StatusCode::NOT_FOUND,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        };
        return (status, Json(err)).into_response();
    }
    let result = resp
        .get_mut("result")
        .map_or(Value::Null, Value::take);
    (StatusCode::OK, Json(result)).into_response()
}
