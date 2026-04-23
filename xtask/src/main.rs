//! Tidepool repo-local dev tooling. Run via `cargo xtask <cmd>`.
//!
//! Subcommands land in their own modules so each is isolated from
//! the rest:
//!
//! - `record-helius` (phase 1): read `contracts/cases.toml`, hit real
//!   Helius, write raw responses to `contracts/fixtures/`.
//! - `derive-schemas` (phase 2): infer JSON Schema from each
//!   recorded fixture, commit alongside.
//! - `check-drift` (phase 3): compare working-tree fixtures + schemas
//!   against the committed versions and emit a structured summary.
//!   Used by the weekly workflow to decide whether to open a PR.
//!
//! All outputs are committed to the repo — the fixtures + schemas are
//! the source of truth for "what Helius returned last time we asked."

use clap::{Parser, Subcommand};

mod check_drift;
mod record;
mod schemas;

#[derive(Parser)]
#[command(
    name = "xtask",
    about = "Tidepool dev tooling — fixture recording, schema derivation, drift detection."
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Record Helius responses for every case in contracts/cases.toml
    /// and write them to contracts/fixtures/<method>/<case>.json.
    /// Requires HELIUS_API_KEY.
    RecordHelius(record::Args),
    /// Infer a JSON Schema from every committed fixture and write to
    /// contracts/schemas/<method>/<case>.schema.json. Offline — no
    /// network, no API key needed.
    DeriveSchemas(schemas::Args),
    /// Diff working-tree fixtures + schemas against the committed
    /// ones. Exits non-zero when drift exists so CI can gate on it;
    /// prints a structured summary of added/removed/changed files.
    CheckDrift(check_drift::Args),
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Load `.env` if present so the recorder picks up HELIUS_API_KEY
    // locally without users having to `export` on every shell. CI
    // injects the same var via GitHub Actions secrets, so the
    // fallback chain becomes: process env > .env > error.
    let _ = dotenvy::dotenv();

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let cli = Cli::parse();
    match cli.command {
        Command::RecordHelius(args) => record::run(args).await,
        Command::DeriveSchemas(args) => schemas::run(args).await,
        Command::CheckDrift(args) => check_drift::run(args).await,
    }
}
