//! `tidepool-rpc` binary. Thin wrapper around `tidepool_rpc_server::run`.
//!
//! Usage:
//!
//! ```text
//! tidepool-rpc start \
//!   --port 8897 \
//!   --upstream http://127.0.0.1:8899 \
//!   --upstream-ws ws://127.0.0.1:8900 \
//!   --index-tree <merkle-tree-pubkey> \
//!   --index-tree <another-tree-pubkey>
//! ```
//!
//! Environment variables mirror every flag:
//! `TIDEPOOL_PORT`, `TIDEPOOL_UPSTREAM`, `TIDEPOOL_UPSTREAM_WS`,
//! `TIDEPOOL_INDEX_TREES` (comma-separated), `TIDEPOOL_RPC_TIMEOUT_MS`.
//! `RUST_LOG` controls tracing verbosity (e.g. `RUST_LOG=tidepool=debug`).

use std::sync::OnceLock;
use std::time::Duration;

use clap::{Parser, Subcommand};
use tidepool_rpc::compatibility::compatibility;
use tidepool_rpc_server::{run, ServerConfig};
use tracing_subscriber::EnvFilter;

/// Build the `--version` / `-V` output. Combines `CARGO_PKG_VERSION`
/// with the `tested-against` pins from `compatibility.toml` so users
/// see "this release vs. these upstream versions" without running
/// the server. Cached in a OnceLock — we only format it once even
/// though clap may ask repeatedly.
fn long_version() -> &'static str {
    static CACHE: OnceLock<String> = OnceLock::new();
    CACHE.get_or_init(|| {
        // clap prepends "{bin} " before `long_version`, so start
        // with the version number + the tested-against stanza.
        use std::fmt::Write as _;
        let base = env!("CARGO_PKG_VERSION");
        let c = compatibility();
        let mut out = format!("{base}\n\ntested against:");
        for (name, pin) in &c.tested_against {
            let _ = write!(out, "\n  {name:<14} {}", pin.version);
        }
        out
    })
}

#[derive(Parser, Debug)]
#[command(
    name = "tidepool-rpc",
    version = env!("CARGO_PKG_VERSION"),
    long_version = long_version(),
    about = "Tidepool — Helius-compatible local dev environment, built on Surfpool",
    long_about = None,
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Start the HTTP + WebSocket RPC server.
    Start(StartArgs),
}

#[derive(clap::Args, Debug)]
struct StartArgs {
    /// HTTP port. WS polyfill binds on `port + 1`.
    #[arg(short, long, env = "TIDEPOOL_PORT", default_value_t = 8897)]
    port: u16,

    /// Upstream Solana RPC URL.
    #[arg(
        long,
        env = "TIDEPOOL_UPSTREAM",
        default_value = "http://127.0.0.1:8899"
    )]
    upstream: String,

    /// Upstream Solana WebSocket URL. Defaults to `ws://<upstream host>:8900`
    /// (Surfpool's native WS port). Override for non-default setups or `wss://`.
    #[arg(long, env = "TIDEPOOL_UPSTREAM_WS")]
    upstream_ws: Option<String>,

    /// Bubblegum tree pubkey to backfill on startup. Repeatable.
    /// Via env: comma-separated `TIDEPOOL_INDEX_TREES=<pk1>,<pk2>`.
    #[arg(
        long = "index-tree",
        env = "TIDEPOOL_INDEX_TREES",
        value_delimiter = ','
    )]
    index_tree: Vec<String>,

    /// Upstream RPC call timeout in milliseconds.
    #[arg(long, env = "TIDEPOOL_RPC_TIMEOUT_MS", default_value_t = 10_000)]
    rpc_timeout_ms: u64,

    /// Persistent state path. Mirrors Surfpool's `--db`: accepts a
    /// filesystem path (typically `.sqlite`) for on-disk persistence
    /// or `:memory:` for an explicit ephemeral SQLite run. When
    /// omitted (default), all stores run in-memory and state is
    /// lost on restart.
    #[arg(long, env = "TIDEPOOL_DB")]
    db: Option<std::path::PathBuf>,

    /// Snapshot file(s) to preload at boot, repeatable. Format is the
    /// `SnapshotBlob` envelope returned by `tidepool_exportTreeSnapshot`.
    /// Mirrors Surfpool's `--snapshot` flag; loads happen before the
    /// server starts serving requests.
    #[arg(long = "snapshot", env = "TIDEPOOL_SNAPSHOTS", value_delimiter = ',')]
    snapshots: Vec<std::path::PathBuf>,
}

impl StartArgs {
    fn into_config(self) -> ServerConfig {
        let ws_url = self
            .upstream_ws
            .unwrap_or_else(|| derive_ws_url(&self.upstream));
        ServerConfig {
            port: self.port,
            ws_port: None,
            upstream_url: self.upstream,
            upstream_ws_url: ws_url,
            rpc_timeout: Duration::from_millis(self.rpc_timeout_ms),
            index_trees: self.index_tree,
            db: self.db,
            snapshots: self.snapshots,
        }
    }
}

/// Default WS URL is `ws://<upstream host>:8900`. If `upstream` is
/// unparseable, fall back to `ws://127.0.0.1:8900` so local dev stays
/// working even with a garbage URL.
fn derive_ws_url(upstream: &str) -> String {
    let host = url_host(upstream).unwrap_or_else(|| "127.0.0.1".into());
    format!("ws://{host}:8900")
}

fn url_host(url: &str) -> Option<String> {
    // Hand-rolled because we don't depend on `url` crate yet — this
    // runs once at startup, complexity doesn't matter.
    let after_scheme = url.split_once("://").map_or(url, |(_, rest)| rest);
    let host_with_port = after_scheme.split('/').next().unwrap_or(after_scheme);
    let host = host_with_port.split(':').next().unwrap_or(host_with_port);
    if host.is_empty() {
        None
    } else {
        Some(host.to_string())
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Tracing output to stderr. `RUST_LOG=info` for default verbosity;
    // users can tune per-module via env. We don't install a subscriber
    // when one is already set (allows library users with their own
    // subscriber to take over).
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .with_target(true)
        .try_init();

    let cli = Cli::parse();
    match cli.command {
        Command::Start(args) => {
            let config = args.into_config();
            run(config).await?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn cli_parses_start_with_defaults() {
        let cli = Cli::try_parse_from(["tidepool-rpc", "start"]).expect("parse");
        let Command::Start(args) = cli.command;
        assert_eq!(args.port, 8897);
        assert_eq!(args.upstream, "http://127.0.0.1:8899");
        assert!(args.upstream_ws.is_none());
        assert!(args.index_tree.is_empty());
        assert_eq!(args.rpc_timeout_ms, 10_000);
    }

    #[test]
    fn cli_parses_repeated_index_tree_flags() {
        let cli = Cli::try_parse_from([
            "tidepool-rpc",
            "start",
            "--index-tree",
            "AAA",
            "--index-tree",
            "BBB",
        ])
        .expect("parse");
        let Command::Start(args) = cli.command;
        assert_eq!(args.index_tree, vec!["AAA".to_string(), "BBB".to_string()]);
    }

    #[test]
    fn cli_parses_all_long_flags() {
        let cli = Cli::try_parse_from([
            "tidepool-rpc",
            "start",
            "--port",
            "9000",
            "--upstream",
            "http://example.com:8899",
            "--upstream-ws",
            "wss://example.com:9000",
            "--index-tree",
            "TREE1",
            "--rpc-timeout-ms",
            "5000",
        ])
        .expect("parse");
        let Command::Start(args) = cli.command;
        assert_eq!(args.port, 9000);
        assert_eq!(args.upstream, "http://example.com:8899");
        assert_eq!(args.upstream_ws.as_deref(), Some("wss://example.com:9000"));
        assert_eq!(args.index_tree, vec!["TREE1".to_string()]);
        assert_eq!(args.rpc_timeout_ms, 5000);
    }

    #[test]
    fn derive_ws_url_from_default_upstream() {
        assert_eq!(
            derive_ws_url("http://127.0.0.1:8899"),
            "ws://127.0.0.1:8900"
        );
    }

    #[test]
    fn derive_ws_url_from_remote_host() {
        assert_eq!(
            derive_ws_url("http://rpc.example.com:8899"),
            "ws://rpc.example.com:8900"
        );
    }

    #[test]
    fn derive_ws_url_handles_garbage_gracefully() {
        // Even if the input can't be parsed, we fall back to localhost
        // rather than panicking at startup.
        assert_eq!(derive_ws_url(""), "ws://127.0.0.1:8900");
    }

    #[test]
    fn into_config_populates_ws_default_when_missing() {
        let args = StartArgs {
            port: 8897,
            upstream: "http://127.0.0.1:8899".into(),
            upstream_ws: None,
            index_tree: vec![],
            rpc_timeout_ms: 10_000,
            db: None,
            snapshots: vec![],
        };
        let cfg = args.into_config();
        assert_eq!(cfg.upstream_ws_url, "ws://127.0.0.1:8900");
    }

    #[test]
    fn into_config_preserves_explicit_ws_url() {
        let args = StartArgs {
            port: 8897,
            upstream: "http://127.0.0.1:8899".into(),
            upstream_ws: Some("wss://custom:9999".into()),
            index_tree: vec![],
            rpc_timeout_ms: 10_000,
            db: None,
            snapshots: vec![],
        };
        let cfg = args.into_config();
        assert_eq!(cfg.upstream_ws_url, "wss://custom:9999");
    }

    #[test]
    fn cli_definition_is_well_formed() {
        // Smokes every clap attribute — Cargo builds catch syntax
        // errors, clap's debug_assert catches conflicts like duplicate
        // short flags.
        Cli::command().debug_assert();
    }
}
