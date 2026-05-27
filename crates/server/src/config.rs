//! Server configuration. Shape mirrors the TS `ProxyOptions` plus
//! server-specific knobs (port, WS URL override).

use std::path::PathBuf;
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct ServerConfig {
    /// HTTP port to bind.
    pub port: u16,
    /// Dedicated WebSocket port. When `None`, derives as `port + 1`.
    /// Tests set this explicitly to avoid parallel port-allocation
    /// races where two test runs pick adjacent HTTP ports and collide
    /// on each other's WS.
    pub ws_port: Option<u16>,
    /// Upstream Solana RPC endpoint for passthrough + DAS fetch flow.
    pub upstream_url: String,
    /// Upstream WebSocket URL — Tidepool's WS port is a reverse proxy
    /// onto this. Defaults to `ws://127.0.0.1:8900`, matching Surfpool's
    /// default WS port.
    pub upstream_ws_url: String,
    /// RPC call timeout applied to every upstream fetch.
    pub rpc_timeout: Duration,
    /// Bubblegum trees to backfill on startup. Empty = cNFT support
    /// disabled until a runtime `tidepool_indexTree` call.
    pub index_trees: Vec<String>,
    /// When set, persist cNFT/DAS/webhook state to a single SQLite
    /// file. Mirrors Surfpool's `--db` flag; accepts a filesystem
    /// path (typically ending in `.sqlite`) or the string `:memory:`
    /// for an explicit ephemeral run.
    ///
    /// When `None`, stores run in-memory and state is lost on
    /// restart. Default behavior.
    pub db: Option<PathBuf>,
    /// Snapshot files to load at boot, in order. Each file is a
    /// `SnapshotBlob` envelope returned by
    /// `tidepool_exportTreeSnapshot`. Applied before the HTTP server
    /// starts accepting requests. Mirrors Surfpool's `--snapshot`.
    pub snapshots: Vec<PathBuf>,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            port: 8897,
            ws_port: None,
            upstream_url: "http://127.0.0.1:8899".into(),
            upstream_ws_url: "ws://127.0.0.1:8900".into(),
            rpc_timeout: Duration::from_secs(10),
            index_trees: Vec::new(),
            db: None,
            snapshots: Vec::new(),
        }
    }
}
