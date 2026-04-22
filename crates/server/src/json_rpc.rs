//! JSON-RPC 2.0 envelope types. Kept minimal + lenient — Solana's
//! RPC ecosystem varies slightly between 1.0 / 2.0 implementations
//! and we want to pass through whatever clients send without over-
//! validating.

use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
pub struct JsonRpcRequest {
    /// Always "2.0" in practice — we don't check.
    #[serde(default)]
    pub jsonrpc: Option<String>,
    pub method: String,
    #[serde(default)]
    pub params: serde_json::Value,
    #[serde(default)]
    pub id: serde_json::Value,
}

#[derive(Debug, Serialize)]
pub struct JsonRpcSuccess<'a> {
    pub jsonrpc: &'static str,
    pub id: &'a serde_json::Value,
    pub result: serde_json::Value,
}

#[derive(Debug, Serialize)]
pub struct JsonRpcFailure<'a> {
    pub jsonrpc: &'static str,
    pub id: &'a serde_json::Value,
    pub error: JsonRpcError,
}

#[derive(Debug, Serialize)]
pub struct JsonRpcError {
    pub code: i32,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

#[must_use]
pub fn ok(id: &serde_json::Value, result: serde_json::Value) -> serde_json::Value {
    serde_json::to_value(JsonRpcSuccess {
        jsonrpc: "2.0",
        id,
        result,
    })
    .expect("JsonRpcSuccess is always serializable")
}

#[must_use]
pub fn fail(id: &serde_json::Value, code: i32, message: impl Into<String>) -> serde_json::Value {
    serde_json::to_value(JsonRpcFailure {
        jsonrpc: "2.0",
        id,
        error: JsonRpcError {
            code,
            message: message.into(),
            data: None,
        },
    })
    .expect("JsonRpcFailure is always serializable")
}

// Standard-ish error codes used throughout the server.
pub mod codes {
    pub const INVALID_PARAMS: i32 = -32602;
    pub const INTERNAL_ERROR: i32 = -32000;
    pub const METHOD_NOT_FOUND: i32 = -32601;
}
