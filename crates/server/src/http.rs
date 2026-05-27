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
use tidepool_rpc::cnft::{CnftStore, MemoryCnftStore, SqliteCnftStore};
use tidepool_rpc::das::{AccountDecoder, MplCoreDecoder, TokenMetadataDecoder};
use tidepool_rpc::sqlite_backend::SqliteBackend;
use tidepool_rpc::sqlite_cache::SqliteCache;
use tidepool_rpc::upstream::UpstreamClient;
use tidepool_rpc::webhooks::{MemoryWebhookRegistry, SqliteWebhookRegistry, WebhookRegistry};

use crate::config::ServerConfig;
use crate::dispatcher::{dispatch, Ctx};
use crate::json_rpc::{fail, JsonRpcRequest};
use crate::upstream_http::HttpUpstream;

/// Serve the tidepool JSON-RPC API according to `config`. Blocks
/// until the runtime shuts down.
#[allow(clippy::too_many_lines)]
pub async fn run(config: ServerConfig) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Pick persistence: SQLite (single file, Surfpool-style) when
    // `db` is set to a path or `:memory:`-but-persistent isolate;
    // plain in-memory otherwise. All three stores share one
    // connection inside the backend.
    let (cnft, cache, webhook_registry): (
        Arc<dyn CnftStore>,
        Arc<dyn CacheStore>,
        Arc<dyn WebhookRegistry>,
    ) = if let Some(db) = &config.db {
        info!("tidepool persisting state at {}", db.display());
        let backend = SqliteBackend::open(db)?;
        (
            Arc::new(SqliteCnftStore::new(&backend)),
            Arc::new(SqliteCache::new(&backend)),
            Arc::new(SqliteWebhookRegistry::new(&backend).await?),
        )
    } else {
        (
            Arc::new(MemoryCnftStore::new()),
            Arc::new(MemoryCache::new()),
            Arc::new(MemoryWebhookRegistry::new()),
        )
    };

    let upstream: Arc<dyn UpstreamClient> = Arc::new(HttpUpstream::new(
        config.upstream_url.clone(),
        config.rpc_timeout,
    )?);
    let decoders: Arc<[Arc<dyn AccountDecoder>]> = Arc::from(vec![
        Arc::new(MplCoreDecoder) as Arc<dyn AccountDecoder>,
        Arc::new(TokenMetadataDecoder) as Arc<dyn AccountDecoder>,
    ]);

    let poster: Arc<dyn tidepool_rpc::webhooks::PostClient> = Arc::new(
        crate::webhook_runtime::ReqwestPostClient::new(config.rpc_timeout),
    );
    let webhooks = Arc::new(crate::webhook_runtime::WebhookRuntime::new(
        webhook_registry,
        Arc::clone(&upstream),
        poster,
    ));

    let ctx = Ctx {
        cnft,
        cache,
        upstream,
        decoders,
        webhooks,
    };

    // Snapshot preload (--snapshot). Runs synchronously before the
    // HTTP server starts binding so that by the time requests flow
    // in, `getAssetProof` etc. can already answer against the loaded
    // trees. Errors log + continue — a bad snapshot shouldn't wedge
    // the whole server.
    for snap_path in &config.snapshots {
        match std::fs::read(snap_path) {
            Ok(bytes) => match serde_json::from_slice::<tidepool_rpc::cnft::SnapshotBlob>(&bytes) {
                Ok(blob) => match blob.into_tree_snapshot() {
                    Ok(snapshot) => {
                        match tidepool_rpc::cnft::load_tree(&*ctx.cnft, snapshot).await {
                            Ok(summary) => info!(
                                path = %snap_path.display(),
                                tree = %bs58::encode(summary.tree).into_string(),
                                leaves = summary.leaf_count,
                                "loaded snapshot"
                            ),
                            Err(e) => {
                                warn!(path = %snap_path.display(), err = %e, "snapshot apply failed");
                            }
                        }
                    }
                    Err(e) => {
                        warn!(path = %snap_path.display(), err = %e, "snapshot decode failed");
                    }
                },
                Err(e) => warn!(path = %snap_path.display(), err = %e, "snapshot parse failed"),
            },
            Err(e) => warn!(path = %snap_path.display(), err = %e, "snapshot read failed"),
        }
    }

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
    let passthrough_client = Client::builder().timeout(config.rpc_timeout).build()?;

    let state = AppState {
        ctx,
        passthrough_url: upstream_url,
        passthrough_client,
    };

    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    // REST layer — mirrors the paths helius-sdk hits on
    // `api.helius.xyz/v0/...`. Mounted on the same axum server so a
    // user points their SDK at http://localhost:<port> and gets both
    // JSON-RPC (POST /) and REST (/v0/*) transports from one place.
    // Ctx is injected via Extension so the REST router stays state-
    // type-agnostic and composes cleanly with the typed-state parent.
    let rest_ctx = Arc::new(state.ctx.clone());

    let app: Router = Router::new()
        .route("/", post(handle_post))
        .merge(crate::rest::router::<AppState>())
        .layer(axum::Extension(rest_ctx))
        .layer(cors)
        .with_state(state);

    // WS reverse proxy. Defaults to `port + 1` when ws_port isn't
    // explicitly set — production CLI shape. Tests pre-bind both
    // ports and pass them explicitly to dodge parallel races.
    // Forwards every connection to `upstream_ws_url`; uses
    // `rpc_timeout` as the upstream-dial timeout.
    let ws_port = config.ws_port.unwrap_or(config.port + 1);
    let upstream_ws = config.upstream_ws_url.clone();
    let ws_timeout = config.rpc_timeout;
    tokio::spawn(async move {
        if let Err(e) = crate::ws::run_ws(ws_port, upstream_ws, ws_timeout).await {
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

async fn handle_post(State(state): State<AppState>, body: axum::body::Bytes) -> Response {
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
    let body = fail(
        &Value::Null,
        crate::json_rpc::codes::INTERNAL_ERROR,
        message,
    );
    let mut resp = Json(body).into_response();
    *resp.status_mut() = StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
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
