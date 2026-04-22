//! axum HTTP server: JSON-RPC over POST, CORS, passthrough proxy for
//! anything the dispatcher doesn't claim.
//!
//! The shape mirrors the TS version: one POST route, one upstream
//! forward-path, full wildcard CORS. We intentionally don't bother
//! with clever routing — the payload tells us which method we're
//! handling.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::post,
    Json, Router,
};
use reqwest::Client;
use serde_json::{json, Value};
use tokio::net::TcpListener;
use tower_http::cors::{Any, CorsLayer};
use tracing::{error, info, warn};

use tidepool_rpc::cache::{CacheStore, MemoryCache};
use tidepool_rpc::cnft::{CnftStore, MemoryCnftStore};
use tidepool_rpc::das::{AccountDecoder, MplCoreDecoder, TokenMetadataDecoder};
use tidepool_rpc::upstream::UpstreamClient;

use crate::config::ServerConfig;
use crate::dispatcher::{dispatch, Ctx};
use crate::json_rpc::{fail, JsonRpcRequest};
use crate::upstream_http::HttpUpstream;

/// Serve the tidepool JSON-RPC API according to `config`. Blocks
/// until the runtime shuts down.
pub async fn run(config: ServerConfig) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Wire the service layer. Default in-memory impls; real impls can
    // be swapped in via `run_with_deps` (not shipped yet — slot in
    // when we need SQLite or a remote cache).
    let cnft: Arc<dyn CnftStore> = Arc::new(MemoryCnftStore::new());
    let cache: Arc<dyn CacheStore> = Arc::new(MemoryCache::new());
    let upstream: Arc<dyn UpstreamClient> = Arc::new(HttpUpstream::new(
        config.upstream_url.clone(),
        config.rpc_timeout,
    )?);
    let decoders: Arc<[Arc<dyn AccountDecoder>]> = Arc::from(vec![
        Arc::new(MplCoreDecoder) as Arc<dyn AccountDecoder>,
        Arc::new(TokenMetadataDecoder) as Arc<dyn AccountDecoder>,
    ]);

    let ctx = Ctx {
        cnft,
        cache,
        upstream,
        decoders,
    };

    // Background tree backfill (non-blocking). Failures are logged
    // and don't prevent the server from starting.
    for tree in &config.index_trees {
        let tree = tree.clone();
        let ctx_clone = ctx.clone();
        tokio::spawn(async move {
            match bs58::decode(&tree).into_vec() {
                Ok(v) if v.len() == 32 => {
                    let mut bytes = [0u8; 32];
                    bytes.copy_from_slice(&v);
                    let opts = tidepool_rpc::cnft::IndexTreeOptions::default();
                    match tidepool_rpc::cnft::index_tree(
                        &*ctx_clone.upstream,
                        &*ctx_clone.cnft,
                        bytes,
                        &opts,
                    )
                    .await
                    {
                        Ok(r) => info!(
                            tree = %tree,
                            processed = r.processed,
                            applied = r.applied,
                            "indexed tree"
                        ),
                        Err(e) => warn!(tree = %tree, err = %e, "failed to index tree"),
                    }
                }
                _ => warn!(tree = %tree, "invalid tree pubkey; skipping indexing"),
            }
        });
    }

    let upstream_url = config.upstream_url.clone();
    let passthrough_client = Client::builder()
        .timeout(config.rpc_timeout)
        .build()?;

    let state = AppState {
        ctx,
        passthrough_url: upstream_url,
        passthrough_client,
    };

    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    let app = Router::new()
        .route("/", post(handle_post))
        .layer(cors)
        .with_state(state);

    // WS polyfill server on port + 1 — runs concurrently with HTTP.
    let ws_port = config.port + 1;
    let ws_upstream_url = config.upstream_url.clone();
    let ws_timeout = config.rpc_timeout;
    tokio::spawn(async move {
        if let Err(e) = crate::ws::run_ws(ws_port, ws_upstream_url, ws_timeout).await {
            tracing::error!(err = %e, "ws server exited with error");
        }
    });

    let addr = SocketAddr::from(([0, 0, 0, 0], config.port));
    let listener = TcpListener::bind(&addr).await?;
    info!("tidepool listening on http://{addr} (ws on :{ws_port})");
    axum::serve(listener, app).await?;
    Ok(())
}

#[derive(Clone)]
struct AppState {
    ctx: Ctx<dyn CnftStore, dyn CacheStore, dyn UpstreamClient>,
    passthrough_url: String,
    passthrough_client: Client,
}

async fn handle_post(
    State(state): State<AppState>,
    body: axum::body::Bytes,
) -> Response {
    let Ok(req) = serde_json::from_slice::<JsonRpcRequest>(&body) else {
        // Forward malformed JSON to upstream unchanged — Surfpool's
        // own error becomes the user-visible error, matching TS
        // behavior.
        return passthrough(&state, &body).await;
    };

    match dispatch(&state.ctx, &req).await {
        Some(response_json) => Json(response_json).into_response(),
        None => passthrough(&state, &body).await,
    }
}

async fn passthrough(state: &AppState, body: &axum::body::Bytes) -> Response {
    match state
        .passthrough_client
        .post(&state.passthrough_url)
        .header("content-type", "application/json")
        .body(body.clone())
        .send()
        .await
    {
        Ok(upstream_resp) => {
            let status = upstream_resp.status();
            match upstream_resp.bytes().await {
                Ok(bytes) => {
                    let mut resp = Response::new(axum::body::Body::from(bytes));
                    *resp.status_mut() = status;
                    resp.headers_mut().insert(
                        axum::http::header::CONTENT_TYPE,
                        axum::http::HeaderValue::from_static("application/json"),
                    );
                    resp
                }
                Err(e) => {
                    error!(err = %e, "failed to read upstream body");
                    failure_response(502, "Upstream body read failed")
                }
            }
        }
        Err(e) => {
            error!(err = %e, "upstream unreachable");
            failure_response(502, &format!("Surfpool unreachable: {e}"))
        }
    }
}

fn failure_response(status: u16, message: &str) -> Response {
    let body = fail(&Value::Null, crate::json_rpc::codes::INTERNAL_ERROR, message);
    let mut resp = Json(body).into_response();
    *resp.status_mut() =
        StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    resp
}

// Silence the unused-import warning for Duration on Rust versions
// that don't eliminate it transitively. The real use is in
// `ServerConfig::rpc_timeout` via reqwest.
#[allow(dead_code)]
fn _duration_use(_: Duration) {}

// Silence unused-json! warning when dispatcher isn't compiled with
// certain handler variants (future-proofing).
#[allow(dead_code)]
fn _json_use() -> Value {
    json!(null)
}
