//! Shared SQLite backend — one connection, one file, all of the
//! persistent stores (cNFT indexer, DAS cache, webhook registry)
//! mounted as logical sub-schemas inside the same database.
//!
//! Surfpool's `--db` convention is one SQLite file per process; we
//! match that. Table names are prefix-namespaced (`cnft_*`, `cache_*`,
//! `webhooks_*`) so a user inspecting the file with `sqlite3` sees a
//! coherent layout.
//!
//! Accepts either `:memory:` (ephemeral, test-friendly) or a
//! filesystem path (typically ending in `.sqlite`).

use std::path::Path;
use std::sync::Arc;

use rusqlite::Connection;
use thiserror::Error;
use tokio::sync::Mutex;

use crate::cache::CacheError;
use crate::cnft::store::StoreError;
use crate::webhooks::WebhookError;

/// Combined schema. Runs once on open. The order is cnft → cache →
/// webhooks; each block is independent (no FKs across blocks).
const SCHEMA: &str = include_str!("sqlite_schema.sql");

/// Shared connection handle. Holds an `Arc<Mutex<Connection>>` that
/// each store wrapper clones. SQLite serializes writers on its own;
/// we only need the mutex so async callers don't race on the Rust
/// borrow.
#[derive(Clone)]
pub struct SqliteBackend {
    pub(crate) conn: Arc<Mutex<Connection>>,
}

#[derive(Debug, Error)]
pub enum BackendError {
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

impl SqliteBackend {
    /// Open `:memory:` or a file at `path`. Creates the parent
    /// directory if missing (for file paths); runs migrations.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, BackendError> {
        let path_ref = path.as_ref();
        // Allow the special `:memory:` literal without trying to
        // create a parent dir for it.
        if path_ref.as_os_str() != ":memory:" {
            if let Some(parent) = path_ref.parent() {
                if !parent.as_os_str().is_empty() {
                    std::fs::create_dir_all(parent)?;
                }
            }
        }
        let conn = Connection::open(path_ref)?;
        Self::initialize(conn)
    }

    /// Shorthand for `open(":memory:")`. Used in tests.
    pub fn open_in_memory() -> Result<Self, BackendError> {
        let conn = Connection::open_in_memory()?;
        Self::initialize(conn)
    }

    fn initialize(conn: Connection) -> Result<Self, BackendError> {
        conn.execute_batch(SCHEMA)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }
}

// ─── Error adapters ──────────────────────────────────────────────
// Each store layer has its own error type; we centralize the
// conversion from backend errors so stores don't each re-implement it.

impl From<BackendError> for StoreError {
    fn from(e: BackendError) -> Self {
        Self::UnknownTree {
            tree: format!("sqlite backend: {e}"),
        }
    }
}

impl From<BackendError> for CacheError {
    fn from(e: BackendError) -> Self {
        Self::Backend(e.to_string())
    }
}

impl From<BackendError> for WebhookError {
    fn from(e: BackendError) -> Self {
        Self::BadRequest(format!("sqlite backend: {e}"))
    }
}
