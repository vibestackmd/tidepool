//! Tidepool Node.js bindings via napi-rs.
//!
//! Scope (12a): the primitives a JS test suite actually needs — MSW /
//! Nock / vitest / surfpool-sdk-node users plug `handleJsonRpcBody`
//! into whatever mock layer they already use. Full HTTP server
//! management (`createProxy` that boots axum in-process) is separate
//! work for 12b; consumers that want the full server run the CLI
//! binary as a subprocess today, and MSW users want the in-process
//! dispatch anyway.
//!
//! Exported surface:
//!
//! - `version() -> string` — crate version, sanity check.
//! - `HeliusContext` — opaque class wrapping the Rust context; owns
//!   a MemoryCnftStore + MemoryCache + configured upstream URL +
//!   default decoders.
//! - `handleJsonRpcBody(ctx, body) -> string | null` — dispatch one
//!   JSON-RPC request body. `null` = method not handled (caller
//!   should passthrough via MSW / Nock / undici's own mechanism).
//!
//! This is deliberately minimal — enough to satisfy the MSW/Nock
//! story that motivated the napi bridge in the first place.

#![deny(clippy::unwrap_used)]

use std::sync::Arc;
use std::time::Duration;

use napi_derive::napi;

use tidepool_rpc::cache::{CacheStore, MemoryCache};
use tidepool_rpc::cnft::{CnftStore, MemoryCnftStore};
use tidepool_rpc::das::{AccountDecoder, MplCoreDecoder, TokenMetadataDecoder};
use tidepool_rpc::upstream::UpstreamClient;
use tidepool_rpc::webhooks::PostClient;
use tidepool_server::webhook_runtime::{ReqwestPostClient, WebhookRuntime};
use tidepool_server::HttpUpstream;

/// Crate version — handy for sanity checks in JS tests.
#[napi]
#[must_use]
pub fn version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

/// Options accepted by [`create_helius_context`]. All fields optional
/// with sensible defaults for the happy path (local Surfpool).
#[napi(object)]
pub struct HeliusContextOptions {
    /// Upstream Solana RPC URL. Default: `http://127.0.0.1:8899`.
    pub upstream_url: Option<String>,
    /// Upstream RPC timeout in milliseconds. Default: `10_000`.
    pub rpc_timeout_ms: Option<u32>,
}

/// Opaque JS handle around the Rust context. JS consumers pass this
/// to [`handle_json_rpc_body`] repeatedly; internally we keep the
/// cNFT store, cache, upstream, and decoders alive for the lifetime
/// of the JS object.
#[napi]
pub struct HeliusContext {
    cnft: Arc<dyn CnftStore>,
    cache: Arc<dyn CacheStore>,
    upstream: Arc<dyn UpstreamClient>,
    decoders: Arc<[Arc<dyn AccountDecoder>]>,
    webhooks: Arc<WebhookRuntime<dyn UpstreamClient, dyn PostClient>>,
}

#[napi]
impl HeliusContext {
    /// Create a new context. All options are optional — defaults
    /// target a local Surfpool install.
    #[napi(constructor)]
    pub fn new(options: Option<HeliusContextOptions>) -> napi::Result<Self> {
        let opts = options.unwrap_or(HeliusContextOptions {
            upstream_url: None,
            rpc_timeout_ms: None,
        });
        let upstream_url = opts
            .upstream_url
            .unwrap_or_else(|| "http://127.0.0.1:8899".to_string());
        let rpc_timeout_ms = opts.rpc_timeout_ms.unwrap_or(10_000);

        let upstream = HttpUpstream::new(
            upstream_url,
            Duration::from_millis(u64::from(rpc_timeout_ms)),
        )
        .map_err(|e| napi::Error::from_reason(format!("{e}")))?;

        let upstream_arc: Arc<dyn UpstreamClient> = Arc::new(upstream);
        let poster: Arc<dyn PostClient> = Arc::new(ReqwestPostClient::new(Duration::from_millis(
            u64::from(rpc_timeout_ms),
        )));
        let webhooks = Arc::new(WebhookRuntime::with_memory_registry(
            Arc::clone(&upstream_arc),
            poster,
        ));

        Ok(Self {
            cnft: Arc::new(MemoryCnftStore::new()) as Arc<dyn CnftStore>,
            cache: Arc::new(MemoryCache::new()) as Arc<dyn CacheStore>,
            upstream: upstream_arc,
            decoders: Arc::from(vec![
                Arc::new(MplCoreDecoder) as Arc<dyn AccountDecoder>,
                Arc::new(TokenMetadataDecoder) as Arc<dyn AccountDecoder>,
            ]),
            webhooks,
        })
    }
}

/// Dispatch one JSON-RPC request body against the given context.
///
/// - Returns `Some(responseJson)` when the method is one the service
///   layer handles natively.
/// - Returns `None` when the method is unknown / the body is malformed
///   — the caller is expected to defer to its own passthrough (MSW's
///   `passthrough()`, Nock's `.allowUnmocked()`, undici's net-connect).
///
/// Body may be a JSON string or a pre-parsed object serialized by the
/// caller. Response is a JSON-encoded string so it drops straight into
/// most mock-HTTP APIs (e.g. `HttpResponse.json(JSON.parse(r))`).
#[napi]
pub async fn handle_json_rpc_body(
    ctx: &HeliusContext,
    body: String,
) -> napi::Result<Option<String>> {
    let req: tidepool_server::json_rpc::JsonRpcRequest = match serde_json::from_str(&body) {
        Ok(r) => r,
        Err(_) => return Ok(None),
    };
    let server_ctx = tidepool_server::dispatcher::Ctx {
        cnft: Arc::clone(&ctx.cnft),
        cache: Arc::clone(&ctx.cache),
        upstream: Arc::clone(&ctx.upstream),
        decoders: Arc::clone(&ctx.decoders),
        webhooks: Arc::clone(&ctx.webhooks),
    };
    let Some(response) = tidepool_server::dispatcher::dispatch(&server_ctx, &req).await else {
        return Ok(None);
    };
    serde_json::to_string(&response)
        .map(Some)
        .map_err(|e| napi::Error::from_reason(format!("serialize response: {e}")))
}
