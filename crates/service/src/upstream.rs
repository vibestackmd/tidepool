//! Upstream-client trait and a deterministic fixture impl.
//!
//! The trait is minimal: one generic `rpc_call` and a typed
//! `get_account` convenience. Everything the indexer and DAS handlers
//! need goes through these two methods. A network-backed
//! implementation (lifted onto `solana-client` crate) lands alongside
//! the server crate — for the service layer we only depend on the
//! trait + the fixture impl for tests.

use std::collections::HashMap;

use async_trait::async_trait;
use thiserror::Error;

/// Raw on-chain account as the indexer and decoders consume it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountData {
    pub data: Vec<u8>,
    pub owner: [u8; 32],
    pub lamports: u64,
}

/// Errors surfaceable by any `UpstreamClient`. Network-backed impls
/// wrap transport errors; the fixture impl only produces
/// `MethodNotStubbed`.
#[derive(Debug, Error)]
pub enum UpstreamError {
    #[error("upstream transport error: {0}")]
    Transport(String),
    #[error("upstream returned an RPC error: {0}")]
    Rpc(String),
    #[error("no fixture stub registered for RPC method '{method}'")]
    MethodNotStubbed { method: String },
    #[error("upstream request timed out after {millis}ms")]
    Timeout { millis: u64 },
}

pub type UpstreamResult<T> = Result<T, UpstreamError>;

/// An abstract Solana RPC client. `rpc_call` is the catch-all; typed
/// conveniences sit on top of it where we care about ergonomics.
#[async_trait]
pub trait UpstreamClient: Send + Sync {
    /// Invoke an arbitrary JSON-RPC method and return the `result`
    /// field as raw JSON bytes. Higher layers deserialize per-method.
    async fn rpc_call(&self, method: &str, params: serde_json::Value) -> UpstreamResult<Vec<u8>>;

    /// Convenience: read an account by base58 pubkey. Network impl
    /// goes through `getAccountInfo`; fixture impl reads from its map.
    /// Returns `Ok(None)` when the account doesn't exist; `Err` for
    /// transport failure.
    async fn get_account(&self, address: &str) -> UpstreamResult<Option<AccountData>>;
}

/// Closure shape for a fixture RPC method producer. Named to keep the
/// struct definition readable and to satisfy the `type_complexity`
/// lint — `Box<dyn Fn(...) -> ...>` gets unwieldy quickly.
type FixtureRpcHandler =
    Box<dyn Fn(&serde_json::Value) -> UpstreamResult<serde_json::Value> + Send + Sync>;

/// In-process canned upstream for tests. `rpc_responses` is a
/// method-name → producer map; `accounts` is consulted by
/// `get_account`. Producers close over owned state so tests can drive
/// sequences deterministically without async plumbing.
pub struct FixtureUpstream {
    accounts: HashMap<String, AccountData>,
    rpc_responses: HashMap<String, FixtureRpcHandler>,
}

impl FixtureUpstream {
    #[must_use]
    pub fn new() -> Self {
        Self {
            accounts: HashMap::new(),
            rpc_responses: HashMap::new(),
        }
    }

    /// Register an account under its base58 address. The same address
    /// also satisfies a `getAccountInfo`-shaped `rpc_call` unless a
    /// producer is registered explicitly for that method.
    #[must_use]
    pub fn with_account(mut self, address: impl Into<String>, data: AccountData) -> Self {
        self.accounts.insert(address.into(), data);
        self
    }

    /// Stub a JSON-RPC method with a producer closure that receives
    /// the raw `params` value and returns the `result` value.
    #[must_use]
    pub fn with_method<F>(mut self, method: impl Into<String>, handler: F) -> Self
    where
        F: Fn(&serde_json::Value) -> UpstreamResult<serde_json::Value> + Send + Sync + 'static,
    {
        self.rpc_responses.insert(method.into(), Box::new(handler));
        self
    }
}

impl Default for FixtureUpstream {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl UpstreamClient for FixtureUpstream {
    async fn rpc_call(&self, method: &str, params: serde_json::Value) -> UpstreamResult<Vec<u8>> {
        if let Some(handler) = self.rpc_responses.get(method) {
            let value = handler(&params)?;
            return serde_json::to_vec(&value)
                .map_err(|e| UpstreamError::Transport(format!("serialize fixture result: {e}")));
        }
        Err(UpstreamError::MethodNotStubbed {
            method: method.to_string(),
        })
    }

    async fn get_account(&self, address: &str) -> UpstreamResult<Option<AccountData>> {
        Ok(self.accounts.get(address).cloned())
    }
}
