//! Production `UpstreamClient` impl over `reqwest`. Plain JSON-RPC
//! POST to the upstream URL.
//!
//! Why not `solana-client`: its typed `RpcClient` doesn't surface
//! generic method dispatch, and we need to pass unknown methods
//! through unchanged. reqwest is the de facto Rust HTTP client and
//! what solana-client uses internally anyway.

use std::time::Duration;

use async_trait::async_trait;
use reqwest::Client;
use serde_json::{json, Value};

use tidepool_rpc::upstream::{AccountData, UpstreamClient, UpstreamError, UpstreamResult};

/// Max bytes we'll read from an off-chain metadata document. Metaplex
/// JSON is a few KB; 2 MiB is a generous ceiling that still caps a
/// hostile or runaway URI.
const OFFCHAIN_MAX_BYTES: usize = 2 * 1024 * 1024;

#[derive(Debug, Clone)]
pub struct HttpUpstream {
    client: Client,
    url: String,
    timeout: Duration,
    /// When false, `fetch_uri` always returns `None` — disables
    /// off-chain DAS metadata enrichment (the `--no-offchain-metadata`
    /// flag). Useful for hermetic / fully-offline CI.
    offchain_enabled: bool,
}

impl HttpUpstream {
    pub fn new(url: impl Into<String>, timeout: Duration) -> Result<Self, UpstreamError> {
        let client = Client::builder()
            .timeout(timeout)
            .build()
            .map_err(|e| UpstreamError::Transport(e.to_string()))?;
        Ok(Self {
            client,
            url: url.into(),
            timeout,
            offchain_enabled: true,
        })
    }

    /// Toggle off-chain metadata fetching. Defaults to enabled.
    #[must_use]
    pub fn with_offchain_metadata(mut self, enabled: bool) -> Self {
        self.offchain_enabled = enabled;
        self
    }

    async fn post_rpc(&self, method: &str, params: Value) -> UpstreamResult<Value> {
        let body = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": method,
            "params": params,
        });
        let resp = self
            .client
            .post(&self.url)
            .json(&body)
            .send()
            .await
            .map_err(|e| {
                if e.is_timeout() {
                    UpstreamError::Timeout {
                        millis: u64::try_from(self.timeout.as_millis()).unwrap_or(u64::MAX),
                    }
                } else {
                    UpstreamError::Transport(e.to_string())
                }
            })?;
        let json: Value = resp
            .json()
            .await
            .map_err(|e| UpstreamError::Transport(format!("decode upstream body: {e}")))?;
        if let Some(err) = json.get("error") {
            return Err(UpstreamError::Rpc(err.to_string()));
        }
        Ok(json.get("result").cloned().unwrap_or(Value::Null))
    }
}

#[async_trait]
impl UpstreamClient for HttpUpstream {
    async fn rpc_call(&self, method: &str, params: Value) -> UpstreamResult<Vec<u8>> {
        let result = self.post_rpc(method, params).await?;
        serde_json::to_vec(&result)
            .map_err(|e| UpstreamError::Transport(format!("serialize result: {e}")))
    }

    async fn get_account(&self, address: &str) -> UpstreamResult<Option<AccountData>> {
        let params = json!([address, { "encoding": "base64" }]);
        let result = self.post_rpc("getAccountInfo", params).await?;
        // Response shape: { context: { slot }, value: AccountInfo | null }
        let Some(value) = result.get("value") else {
            return Ok(None);
        };
        if value.is_null() {
            return Ok(None);
        }

        let owner = value
            .get("owner")
            .and_then(Value::as_str)
            .ok_or_else(|| UpstreamError::Rpc("missing owner in getAccountInfo response".into()))?;
        let lamports = value.get("lamports").and_then(Value::as_u64).unwrap_or(0);
        let data_array = value
            .get("data")
            .and_then(Value::as_array)
            .ok_or_else(|| UpstreamError::Rpc("missing data array in getAccountInfo".into()))?;
        // Shape: [base64_data, encoding].
        let b64 = data_array
            .first()
            .and_then(Value::as_str)
            .ok_or_else(|| UpstreamError::Rpc("malformed data tuple".into()))?;

        let data = base64_decode(b64)
            .ok_or_else(|| UpstreamError::Rpc("base64-decode failed for account data".into()))?;
        let owner_bytes = base58_decode_32(owner)
            .ok_or_else(|| UpstreamError::Rpc("base58-decode owner failed".into()))?;

        Ok(Some(AccountData {
            data,
            owner: owner_bytes,
            lamports,
        }))
    }

    /// Fetch off-chain metadata. Supports `http(s)://` (via reqwest,
    /// inheriting the client timeout, capped at `OFFCHAIN_MAX_BYTES`)
    /// and `file://` (local read, for dev-seeded metadata). Fail-soft:
    /// every error path returns `None` so a `getAsset` degrades to its
    /// on-chain fields rather than failing.
    async fn fetch_uri(&self, uri: &str) -> Option<Vec<u8>> {
        if !self.offchain_enabled {
            return None;
        }
        if let Some(path) = uri.strip_prefix("file://") {
            // file:///abs/path → "/abs/path"; file://host/path is rare
            // for metadata, so we treat everything after the scheme as
            // a filesystem path.
            let bytes = tokio::fs::read(path).await.ok()?;
            if bytes.len() > OFFCHAIN_MAX_BYTES {
                return None;
            }
            return Some(bytes);
        }
        if uri.starts_with("http://") || uri.starts_with("https://") {
            let resp = self.client.get(uri).send().await.ok()?;
            if !resp.status().is_success() {
                return None;
            }
            // Cap the body. content-length is advisory; enforce on the
            // actual bytes too.
            if let Some(len) = resp.content_length() {
                if len > OFFCHAIN_MAX_BYTES as u64 {
                    return None;
                }
            }
            let bytes = resp.bytes().await.ok()?;
            if bytes.len() > OFFCHAIN_MAX_BYTES {
                return None;
            }
            return Some(bytes.to_vec());
        }
        // Unknown scheme (ipfs://, ar://, data:, …) — not resolved
        // locally. Real Helius runs gateways for these; Tidepool
        // leaves them to the consumer. Fail-soft.
        None
    }
}

// ─── small codec helpers ───────────────────────────────────────────

fn base64_decode(s: &str) -> Option<Vec<u8>> {
    // Hand-rolled base64 decoder to avoid adding `base64` crate for
    // this single call site. Handles standard + URL-safe alphabets,
    // ignores padding strictness.
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    const ALPHABET_URL: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut table = [255u8; 256];
    for (i, &b) in ALPHABET.iter().enumerate() {
        table[b as usize] = i as u8;
    }
    for (i, &b) in ALPHABET_URL.iter().enumerate() {
        table[b as usize] = i as u8;
    }
    let mut out: Vec<u8> = Vec::with_capacity(s.len() * 3 / 4);
    let mut buf: u32 = 0;
    let mut bits: u32 = 0;
    for &b in s.as_bytes() {
        if b == b'=' || b == b'\r' || b == b'\n' {
            continue;
        }
        let v = table[b as usize];
        if v == 255 {
            return None;
        }
        buf = (buf << 6) | u32::from(v);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(((buf >> bits) & 0xff) as u8);
        }
    }
    Some(out)
}

fn base58_decode_32(s: &str) -> Option<[u8; 32]> {
    let bytes = bs58::decode(s).into_vec().ok()?;
    bytes.try_into().ok()
}
