//! SQLite-backed `WebhookRegistry`. Shares a `Connection` with the
//! other persistent stores via `SqliteBackend`. Tables prefixed
//! `webhooks_*`; schema in the central migration.
//!
//! JSON-in-a-BLOB pattern keeps the schema stable as `Webhook`
//! fields evolve. The id counter seeds past the prior max on
//! open so fresh ids don't collide after a restart.

use std::sync::Arc;

use async_trait::async_trait;
use rusqlite::{params, Connection, OptionalExtension};
use tokio::sync::Mutex;

use super::registry::{WebhookError, WebhookRegistry, WebhookResult};
use super::types::{Webhook, WebhookInput};
use crate::sqlite_backend::SqliteBackend;

pub struct SqliteWebhookRegistry {
    conn: Arc<Mutex<Connection>>,
    counter: Arc<Mutex<u64>>,
}

impl SqliteWebhookRegistry {
    /// Build from a shared backend. Seeds the in-process id counter
    /// by reading the max existing `webhook_id` and parsing it as hex
    /// (our standard `wh_<hex>` format). Anything we can't parse —
    /// including a future id format change — resets to 0.
    pub async fn new(backend: &SqliteBackend) -> Result<Self, WebhookError> {
        let conn = Arc::clone(&backend.conn);
        let max_id: u64 = {
            let guard = conn.lock().await;
            guard
                .query_row(
                    "SELECT webhook_id FROM webhooks_webhooks ORDER BY rowid DESC LIMIT 1",
                    [],
                    |r| r.get::<_, String>(0),
                )
                .optional()
                .map_err(map_err)?
                .and_then(|s| {
                    s.strip_prefix("wh_")
                        .and_then(|rest| u64::from_str_radix(rest, 16).ok())
                })
                .unwrap_or(0)
        };
        Ok(Self {
            conn,
            counter: Arc::new(Mutex::new(max_id)),
        })
    }

    async fn next_id(&self) -> String {
        let mut c = self.counter.lock().await;
        *c += 1;
        format!("wh_{:x}", *c)
    }
}

#[allow(clippy::needless_pass_by_value)]
fn map_err(e: rusqlite::Error) -> WebhookError {
    WebhookError::BadRequest(format!("sqlite: {e}"))
}

#[async_trait]
impl WebhookRegistry for SqliteWebhookRegistry {
    async fn create(&self, input: WebhookInput) -> WebhookResult<Webhook> {
        let url = input
            .webhook_url
            .ok_or_else(|| WebhookError::BadRequest("webhookURL is required".into()))?;
        let addresses = input.account_addresses.unwrap_or_default();
        if addresses.is_empty() {
            return Err(WebhookError::BadRequest(
                "accountAddresses must contain at least one address".into(),
            ));
        }
        let id = self.next_id().await;
        let webhook = Webhook {
            webhook_id: id.clone(),
            webhook_url: url,
            account_addresses: addresses,
            transaction_types: input.transaction_types,
            txn_status: input.txn_status,
            webhook_type: input.webhook_type,
            auth_header: input.auth_header,
        };
        let json = serde_json::to_vec(&webhook)
            .map_err(|e| WebhookError::BadRequest(format!("serialize: {e}")))?;
        let c = self.conn.lock().await;
        c.execute(
            "INSERT INTO webhooks_webhooks(webhook_id, json) VALUES (?1, ?2)",
            params![id, json],
        )
        .map_err(map_err)?;
        Ok(webhook)
    }

    async fn list(&self) -> WebhookResult<Vec<Webhook>> {
        let c = self.conn.lock().await;
        let mut stmt = c
            .prepare("SELECT json FROM webhooks_webhooks ORDER BY webhook_id ASC")
            .map_err(map_err)?;
        let rows = stmt
            .query_map([], |r| r.get::<_, Vec<u8>>(0))
            .map_err(map_err)?;
        let mut out = Vec::new();
        for r in rows {
            let bytes = r.map_err(map_err)?;
            if let Ok(w) = serde_json::from_slice::<Webhook>(&bytes) {
                out.push(w);
            }
        }
        Ok(out)
    }

    async fn get(&self, id: &str) -> WebhookResult<Option<Webhook>> {
        let c = self.conn.lock().await;
        let json: Option<Vec<u8>> = c
            .query_row(
                "SELECT json FROM webhooks_webhooks WHERE webhook_id = ?1",
                params![id],
                |r| r.get(0),
            )
            .optional()
            .map_err(map_err)?;
        Ok(json.and_then(|b| serde_json::from_slice(&b).ok()))
    }

    async fn edit(&self, id: &str, input: WebhookInput) -> WebhookResult<Webhook> {
        let c = self.conn.lock().await;
        let json: Option<Vec<u8>> = c
            .query_row(
                "SELECT json FROM webhooks_webhooks WHERE webhook_id = ?1",
                params![id],
                |r| r.get(0),
            )
            .optional()
            .map_err(map_err)?;
        let Some(bytes) = json else {
            return Err(WebhookError::NotFound { id: id.to_string() });
        };
        let existing: Webhook = serde_json::from_slice(&bytes)
            .map_err(|e| WebhookError::BadRequest(format!("deserialize: {e}")))?;
        let merged = Webhook {
            webhook_id: existing.webhook_id.clone(),
            webhook_url: input.webhook_url.unwrap_or(existing.webhook_url),
            account_addresses: input.account_addresses.unwrap_or(existing.account_addresses),
            transaction_types: if input.transaction_types.is_empty() {
                existing.transaction_types
            } else {
                input.transaction_types
            },
            txn_status: input.txn_status.or(existing.txn_status),
            webhook_type: input.webhook_type.or(existing.webhook_type),
            auth_header: input.auth_header.or(existing.auth_header),
        };
        let new_json = serde_json::to_vec(&merged)
            .map_err(|e| WebhookError::BadRequest(format!("serialize: {e}")))?;
        c.execute(
            "UPDATE webhooks_webhooks SET json = ?1 WHERE webhook_id = ?2",
            params![new_json, id],
        )
        .map_err(map_err)?;
        Ok(merged)
    }

    async fn delete(&self, id: &str) -> WebhookResult<bool> {
        let c = self.conn.lock().await;
        let n = c
            .execute(
                "DELETE FROM webhooks_webhooks WHERE webhook_id = ?1",
                params![id],
            )
            .map_err(map_err)?;
        Ok(n > 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn backend() -> SqliteBackend {
        SqliteBackend::open_in_memory().unwrap()
    }

    fn input(url: &str, addrs: &[&str]) -> WebhookInput {
        WebhookInput {
            webhook_url: Some(url.into()),
            account_addresses: Some(addrs.iter().map(|s| (*s).to_string()).collect()),
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn create_get_list_round_trip() {
        let r = SqliteWebhookRegistry::new(&backend()).await.unwrap();
        let wh = r.create(input("https://x", &["A"])).await.unwrap();
        assert_eq!(r.get(&wh.webhook_id).await.unwrap().unwrap(), wh);
        let all = r.list().await.unwrap();
        assert_eq!(all.len(), 1);
    }

    #[tokio::test]
    async fn edit_preserves_fields_not_set_in_input() {
        let r = SqliteWebhookRegistry::new(&backend()).await.unwrap();
        let wh = r
            .create(WebhookInput {
                webhook_url: Some("https://x".into()),
                account_addresses: Some(vec!["A".into()]),
                transaction_types: vec!["NFT_SALE".into()],
                ..Default::default()
            })
            .await
            .unwrap();
        let edited = r
            .edit(
                &wh.webhook_id,
                WebhookInput {
                    webhook_url: Some("https://y".into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(edited.webhook_url, "https://y");
        assert_eq!(edited.transaction_types, vec!["NFT_SALE".to_string()]);
    }

    #[tokio::test]
    async fn delete_removes_and_second_delete_returns_false() {
        let r = SqliteWebhookRegistry::new(&backend()).await.unwrap();
        let wh = r.create(input("https://x", &["A"])).await.unwrap();
        assert!(r.delete(&wh.webhook_id).await.unwrap());
        assert!(!r.delete(&wh.webhook_id).await.unwrap());
    }

    #[tokio::test]
    async fn persistence_across_opens_preserves_webhooks_and_id_counter() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let path = tmp.path().to_path_buf();
        drop(tmp);

        let first_id = {
            let backend = SqliteBackend::open(&path).unwrap();
            let r = SqliteWebhookRegistry::new(&backend).await.unwrap();
            let a = r.create(input("https://a", &["A"])).await.unwrap();
            a.webhook_id
        };

        let backend = SqliteBackend::open(&path).unwrap();
        let r = SqliteWebhookRegistry::new(&backend).await.unwrap();
        assert_eq!(r.list().await.unwrap().len(), 1);
        let next = r.create(input("https://b", &["B"])).await.unwrap();
        assert_ne!(next.webhook_id, first_id, "counter resumes past prior max");
        let _ = std::fs::remove_file(&path);
    }
}
