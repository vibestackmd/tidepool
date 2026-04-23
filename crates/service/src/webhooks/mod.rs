//! Webhooks simulator — local polling delivery for Helius's
//! `createWebhook` family.
//!
//! Real Helius runs a streaming backend that pushes parsed
//! transactions to user-supplied URLs. Locally we fake the same
//! surface via per-webhook polling tasks:
//!   1. For each registered `accountAddress`, run
//!      `getSignaturesForAddress` on a 500ms interval.
//!   2. For every signature we haven't seen before, fetch the tx and
//!      buffer it.
//!   3. POST the buffered envelope to the user's webhook URL.
//!
//! Scope trimmed for v1:
//! - **Payload shape** is a simplified array of
//!   `{signature, slot, timestamp, err, accountAddresses}` entries
//!   rather than Helius's full enhanced-transaction shape. Enhanced
//!   payloads land once the tx parsers do.
//! - **Webhook types** (RAW, DISCORD, etc) are ignored — we only send
//!   RAW-equivalent JSON bodies.
//! - **Event-type filters** (`NFT_SALE`, `SWAP`, etc) are stored on the
//!   webhook record but not applied — delivery fires on every signature.
//! - **Auth headers**, **retries**, and **delivery history** are
//!   deliberately absent. Failed deliveries are logged via `tracing`
//!   and dropped.
//!
//! Storage is an in-memory `MemoryWebhookRegistry` wrapped in a
//! tokio Mutex. Restarts lose state; that's acceptable for a local
//! dev tool. Persistence slots in later behind the same API.

pub mod delivery;
pub mod registry;
pub mod sqlite_registry;
pub mod types;

pub use delivery::{spawn_delivery_task, tick_once, PostClient, POLL_INTERVAL};
pub use registry::{MemoryWebhookRegistry, WebhookError, WebhookRegistry, WebhookResult};
pub use sqlite_registry::SqliteWebhookRegistry;
pub use types::{Webhook, WebhookEvent, WebhookInput};
