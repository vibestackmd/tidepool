//! `record-helius` — pull real-Helius responses for every case in
//! `contracts/cases.toml` and persist them verbatim.
//!
//! **Wire-level** recording: we write exactly what Helius returned,
//! including `id`, `jsonrpc`, and error envelopes. That makes the
//! fixtures useful as both (a) replay inputs in tests and (b) raw
//! evidence of what the upstream was doing on the day we recorded.
//!
//! Two transports are supported, mirroring Helius's own split:
//!
//! ```toml
//! # JSON-RPC (default)
//! [[case]]
//! name = "getAsset_mad_lads_1337"
//! method = "getAsset"
//! params = { id = "J1S9H..." }
//!
//! # REST
//! [[case]]
//! name = "getBalances_small_wallet"
//! method = "getBalances"             # still used to group fixtures on disk
//! transport = "rest"
//! rest = { verb = "GET", path = "/v0/addresses/<addr>/balances" }
//!
//! # REST with JSON body (POST/PUT)
//! [[case]]
//! name = "getTransactions_single_sig"
//! method = "getTransactions"
//! transport = "rest"
//! rest = { verb = "POST", path = "/v0/transactions", body = { transactions = ["SIG1"] } }
//! ```
//!
//! `skip = "..."` on any case short-circuits it with a reason.
//! Runs sequentially with a small delay between calls so we don't
//! trip Helius's rate limits. Failures on individual cases warn and
//! continue — the goal is best-effort capture of everything we can.

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{bail, Context};
use clap::Parser;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tracing::{info, warn};

#[derive(Parser)]
pub struct Args {
    /// Path to the cases file. Defaults to `contracts/cases.toml`.
    #[arg(long, default_value = "contracts/cases.toml")]
    cases: PathBuf,
    /// Output directory for fixtures. Defaults to
    /// `contracts/fixtures/`.
    #[arg(long, default_value = "contracts/fixtures")]
    out: PathBuf,
    /// Helius JSON-RPC endpoint. Override to devnet or a self-hosted
    /// proxy when useful.
    #[arg(
        long,
        env = "HELIUS_RPC_URL",
        default_value = "https://mainnet.helius-rpc.com"
    )]
    endpoint: String,
    /// Helius REST base URL. REST lives on a different host than
    /// JSON-RPC — `api.helius.xyz` — so it needs its own flag.
    #[arg(
        long,
        env = "HELIUS_REST_URL",
        default_value = "https://api.helius.xyz"
    )]
    rest_endpoint: String,
    /// Helius API key. Appended as `?api-key=<key>` to both
    /// endpoints.
    #[arg(long, env = "HELIUS_API_KEY")]
    api_key: Option<String>,
    /// Milliseconds to sleep between calls. Low enough to stay snappy,
    /// high enough to not immediately trip Helius's free-tier limit.
    #[arg(long, default_value_t = 500)]
    delay_ms: u64,
    /// Only record cases whose name contains this substring. Useful
    /// when iterating on a single fixture.
    #[arg(long)]
    only: Option<String>,
    /// Only record cases for a specific transport. Accepts `json_rpc`
    /// or `rest`. Lets CI refresh REST fixtures without re-hitting
    /// the JSON-RPC cases (or vice versa) during targeted drift runs.
    #[arg(long)]
    transport: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CasesFile {
    #[serde(default, rename = "case")]
    cases: Vec<Case>,
}

#[derive(Debug, Deserialize)]
struct Case {
    name: String,
    method: String,
    #[serde(default)]
    params: Value,
    #[serde(default)]
    transport: Option<String>,
    #[serde(default)]
    rest: Option<RestCase>,
    #[serde(default)]
    skip: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RestCase {
    /// HTTP verb (GET/POST/PUT/DELETE). Case-insensitive.
    verb: String,
    /// Path relative to the REST base URL. `api-key` is appended as a
    /// query parameter automatically.
    path: String,
    /// Optional JSON body for POST/PUT.
    #[serde(default)]
    body: Option<Value>,
}

/// Wrapper we write to disk. `request` is the JSON-RPC body or
/// REST request descriptor we sent; `response` is Helius's raw reply.
/// Keeping both makes the fixture self-describing — future code
/// reviewers never have to go archaeology to figure out what
/// generated any given response.
#[derive(Debug, Serialize)]
struct Fixture<'a> {
    case: &'a str,
    method: &'a str,
    transport: &'a str,
    recorded_at: String,
    endpoint_host: String,
    request: Value,
    response: Value,
}

pub async fn run(args: Args) -> anyhow::Result<()> {
    let api_key = args
        .api_key
        .as_deref()
        .context("HELIUS_API_KEY env var not set — pass --api-key or export it")?;

    let cases_str = tokio::fs::read_to_string(&args.cases)
        .await
        .with_context(|| format!("reading {}", args.cases.display()))?;
    let cases: CasesFile = toml::from_str(&cases_str).context("parsing cases.toml")?;
    if cases.cases.is_empty() {
        bail!("no cases in {}", args.cases.display());
    }

    tokio::fs::create_dir_all(&args.out).await?;

    let rpc_url = format!("{}/?api-key={}", args.endpoint.trim_end_matches('/'), api_key);
    let rpc_host = extract_host(&args.endpoint);
    let rest_base = args.rest_endpoint.trim_end_matches('/').to_string();
    let rest_host = extract_host(&args.rest_endpoint);

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()?;

    let mut recorded = 0;
    let mut skipped = 0;
    let mut failed = 0;

    for case in &cases.cases {
        if let Some(reason) = &case.skip {
            info!(case = %case.name, reason, "skip");
            skipped += 1;
            continue;
        }
        if let Some(only) = &args.only {
            if !case.name.contains(only) {
                skipped += 1;
                continue;
            }
        }

        let transport = case.transport.as_deref().unwrap_or("json_rpc");
        if let Some(wanted) = &args.transport {
            if wanted != transport {
                skipped += 1;
                continue;
            }
        }
        let result = match transport {
            "json_rpc" => record_json_rpc(&client, &rpc_url, &rpc_host, case).await,
            "rest" => record_rest(&client, &rest_base, &rest_host, api_key, case).await,
            other => Err(anyhow::anyhow!("unknown transport `{other}`")),
        };

        info!(case = %case.name, method = %case.method, transport, "recording");
        match result {
            Ok(fixture) => {
                write_fixture(&args.out, &case.method, &case.name, &fixture).await?;
                recorded += 1;
            }
            Err(e) => {
                warn!(case = %case.name, err = %e, "record failed");
                failed += 1;
            }
        }

        if args.delay_ms > 0 {
            tokio::time::sleep(Duration::from_millis(args.delay_ms)).await;
        }
    }

    info!(
        recorded,
        skipped,
        failed,
        out = %args.out.display(),
        "done"
    );
    Ok(())
}

async fn record_json_rpc<'a>(
    client: &reqwest::Client,
    url: &str,
    host: &str,
    case: &'a Case,
) -> anyhow::Result<Fixture<'a>> {
    let request_body = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": case.method,
        "params": case.params,
    });
    let response = post_once(client, url, &request_body).await?;
    Ok(Fixture {
        case: &case.name,
        method: &case.method,
        transport: "json_rpc",
        recorded_at: chrono_like_utc(),
        endpoint_host: host.to_string(),
        request: request_body,
        response,
    })
}

async fn record_rest<'a>(
    client: &reqwest::Client,
    rest_base: &str,
    host: &str,
    api_key: &str,
    case: &'a Case,
) -> anyhow::Result<Fixture<'a>> {
    let rest = case
        .rest
        .as_ref()
        .context("transport=rest requires a [rest] table with verb + path")?;

    // Paths may already contain a query string (e.g. `?limit=3`), so
    // use `&` when joining the api-key to avoid producing `?...?...`.
    let joiner = if rest.path.contains('?') { '&' } else { '?' };
    let url = format!("{}{}{joiner}api-key={}", rest_base, rest.path, api_key);
    let verb = rest.verb.to_uppercase();
    let mut req = match verb.as_str() {
        "GET" => client.get(&url),
        "POST" => client.post(&url),
        "PUT" => client.put(&url),
        "DELETE" => client.delete(&url),
        other => bail!("unsupported REST verb `{other}`"),
    };
    if let Some(body) = &rest.body {
        req = req.json(body);
    }
    let http = req.send().await?;
    let status = http.status();
    let text = http.text().await?;
    if !status.is_success() {
        bail!("upstream {status}: {}", truncate(&text, 400));
    }
    let response: Value = serde_json::from_str(&text)
        .with_context(|| format!("parse JSON: {}", truncate(&text, 200)))?;

    // Redact the api-key from the request descriptor so it never
    // lands in a committed fixture.
    let request_desc = json!({
        "verb": verb,
        "path": rest.path,
        "body": rest.body.clone().unwrap_or(Value::Null),
    });

    Ok(Fixture {
        case: &case.name,
        method: &case.method,
        transport: "rest",
        recorded_at: chrono_like_utc(),
        endpoint_host: host.to_string(),
        request: request_desc,
        response,
    })
}

async fn post_once(
    client: &reqwest::Client,
    url: &str,
    body: &Value,
) -> anyhow::Result<Value> {
    let resp = client.post(url).json(body).send().await?;
    let status = resp.status();
    let text = resp.text().await?;
    if !status.is_success() {
        bail!("upstream {status}: {}", truncate(&text, 400));
    }
    let parsed: Value =
        serde_json::from_str(&text).with_context(|| format!("parse JSON: {}", truncate(&text, 200)))?;
    Ok(parsed)
}

async fn write_fixture(
    out_dir: &Path,
    method: &str,
    case_name: &str,
    fixture: &Fixture<'_>,
) -> anyhow::Result<()> {
    let method_dir = out_dir.join(method);
    tokio::fs::create_dir_all(&method_dir).await?;
    let path = method_dir.join(format!("{case_name}.json"));
    let body = serde_json::to_vec_pretty(fixture)?;
    tokio::fs::write(&path, body).await?;
    info!(path = %path.display(), "wrote fixture");
    Ok(())
}

/// Best-effort host extract for the `endpoint_host` field — avoids
/// committing the API key into fixture metadata.
fn extract_host(endpoint: &str) -> String {
    endpoint
        .trim_start_matches("http://")
        .trim_start_matches("https://")
        .split('/')
        .next()
        .unwrap_or("")
        .to_string()
}

/// RFC-3339-ish UTC timestamp without pulling in `chrono`. Good
/// enough for "when was this recorded" provenance.
fn chrono_like_utc() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("epoch-{secs}")
}

fn truncate(s: &str, n: usize) -> String {
    if s.len() <= n {
        s.to_string()
    } else {
        format!("{}...", &s[..n])
    }
}
