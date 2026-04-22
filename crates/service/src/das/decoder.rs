//! AccountDecoder trait — the pluggable seam for uncompressed-asset
//! decoding. The proxy picks a decoder by matching the account's
//! `owner` program ID; first match wins. Ship your own decoder to add
//! support for any NFT program.
//!
//! Decoders are synchronous + pure. Upstream reads, caching, and
//! cross-account resolution are the DAS-handler's job; decoders only
//! turn raw bytes into a DAS-shaped response (or `Ok(None)` when the
//! byte layout isn't a variant this decoder handles).

use thiserror::Error;

use super::types::DasAsset;

#[derive(Debug, Error)]
pub enum DecoderError {
    #[error("decoder `{decoder}` failed on {context}: {source}")]
    DecodeFailed {
        decoder: &'static str,
        context: String,
        #[source]
        source: std::io::Error,
    },
    #[error("decoder `{decoder}`: {reason}")]
    Invariant {
        decoder: &'static str,
        reason: &'static str,
    },
}

pub trait AccountDecoder: Send + Sync {
    /// Program ID (base58) this decoder handles. The proxy dispatches
    /// by matching the account's `owner` field against this string.
    fn program_id(&self) -> &str;

    /// Human-readable name — used in the `interface` field of the DAS
    /// asset response and in log output.
    fn name(&self) -> &str;

    /// Decode raw account bytes into a DAS asset. Return `Ok(None)`
    /// when the bytes are the right owner program but the wrong
    /// account variant (e.g. HashedAssetV1 when this decoder only
    /// handles AssetV1). Error out only on genuinely corrupt bytes.
    fn decode(&self, pubkey: &str, data: &[u8]) -> Result<Option<DasAsset>, DecoderError>;
}
