//! Server configuration. Shape mirrors the TS `ProxyOptions` plus
//! server-specific knobs (port, WS URL override).

use std::time::Duration;

#[derive(Debug, Clone)]
pub struct ServerConfig {
    /// HTTP port to bind. WS (when enabled) lands on port + 1.
    pub port: u16,
    /// Upstream Solana RPC endpoint for passthrough + DAS fetch flow.
    pub upstream_url: String,
    /// Upstream WebSocket URL — used by the (future) signatureSubscribe
    /// polyfill to forward non-polyfilled subscriptions. Defaults to
    /// `ws://<upstream host>:8900`.
    pub upstream_ws_url: String,
    /// RPC call timeout applied to every upstream fetch.
    pub rpc_timeout: Duration,
    /// Bubblegum trees to backfill on startup. Empty = cNFT support
    /// disabled until a runtime `surfpoolHeliusIndexTree` call.
    pub index_trees: Vec<String>,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            port: 8897,
            upstream_url: "http://127.0.0.1:8899".into(),
            upstream_ws_url: "ws://127.0.0.1:8900".into(),
            rpc_timeout: Duration::from_secs(10),
            index_trees: Vec::new(),
        }
    }
}
