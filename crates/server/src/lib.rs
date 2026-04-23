//! Tidepool HTTP + WebSocket JSON-RPC server. Wraps the service
//! layer's async functions behind an axum front-end.
//!
//! Public surface:
//!
//! - [`run`] starts the server and blocks until shutdown.
//! - [`ServerConfig`] holds port, upstream URL, and optional indexed
//!   trees.
//! - [`HttpUpstream`] is the production [`UpstreamClient`] impl;
//!   consumers building their own servers can import it directly.

#![forbid(unsafe_code)]

pub mod config;
pub mod dispatcher;
pub mod http;
pub mod json_rpc;
pub mod rest;
pub mod upstream_http;
pub mod webhook_runtime;
pub mod ws;

pub use config::ServerConfig;
pub use http::run;
pub use upstream_http::HttpUpstream;
pub use ws::run_ws;

// Re-export of the service layer's upstream trait so consumers don't
// have to reach into the service crate just to write their own impl.
pub use tidepool_rpc::upstream::UpstreamClient;
