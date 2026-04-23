//! Enhanced Transactions — Helius's opinionated per-tx classifier.
//!
//! Helius's enhanced-transactions product returns a structured wrapper
//! around each transaction: `type` (e.g. `NFT_MINT`, `TRANSFER`),
//! `source` (e.g. `METAPLEX`, `BUBBLEGUM`), `nativeTransfers`,
//! `tokenTransfers`, and a few other normalized fields. Real Helius
//! has dozens of program-specific parsers (Jupiter, Raydium, Meteora,
//! Candy Machine, Marinade, …). Recreating that catalog locally is a
//! multi-week engagement.
//!
//! Our v1 scope is deliberately narrow:
//!
//! - **Classification** covers the handful of program IDs we already
//!   know how to reason about: System Program, SPL Token, Token-2022,
//!   Bubblegum (V1 + V2), Metaplex Token Metadata, MplCore. Anything
//!   else yields `type: "UNKNOWN"` / `source: "UNKNOWN"` rather than
//!   fabricating a misleading classification.
//!
//! - **Transfer extraction** uses the tx meta's `preBalances` /
//!   `postBalances` + `preTokenBalances` / `postTokenBalances` to
//!   derive native-SOL and SPL-Token movements without running full
//!   instruction-level parsers.
//!
//! - The `events` sub-object, `description`, and SWAP/STAKE/DEFI
//!   specialized breakouts are **not** populated — those depend on
//!   program-specific parsers we don't ship yet.
//!
//! Consumers that need full parity should continue hitting real
//! Helius for Enhanced Transactions; Tidepool's value is local-first
//! DAS + cNFT state. The module exists primarily so
//! `helius.enhanced.getTransactions(...)` doesn't return
//! METHOD_NOT_FOUND — users get a structurally-valid response with
//! the fields we can reliably produce.

pub mod classify;
pub mod enrich;
pub mod events;
pub mod fetch;
pub mod parse;
pub mod transfers;
pub mod types;

pub use classify::{classify, EnhancedClass};
pub use enrich::enrich_token_standards;
pub use events::derive_nft_event;
pub use fetch::{get_transactions, get_transactions_by_address, TransactionsByAddressOptions};
pub use parse::{parse_enhanced_tx, signatures_matching};
pub use transfers::{extract_native_transfers, extract_token_transfers};
pub use types::{
    AccountData, EnhancedEvents, EnhancedInstruction, EnhancedNativeTransfer,
    EnhancedTokenTransfer, EnhancedTransaction, NftEvent, NftEventMint,
};
