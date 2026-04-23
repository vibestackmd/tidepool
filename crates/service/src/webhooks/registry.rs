//! Webhook storage + CRUD. Trait-first so tests can swap in a mock
//! impl; `MemoryWebhookRegistry` is the only shipping backend today.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use thiserror::Error;
use tokio::sync::Mutex;

use super::types::{Webhook, WebhookInput};

#[derive(Debug, Error)]
pub enum WebhookError {
    #[error("webhook not found: {id}")]
    NotFound { id: String },
    #[error("bad request: {0}")]
    BadRequest(String),
}

pub type WebhookResult<T> = Result<T, WebhookError>;

#[async_trait]
pub trait WebhookRegistry: Send + Sync {
    async fn create(&self, input: WebhookInput) -> WebhookResult<Webhook>;
    async fn list(&self) -> WebhookResult<Vec<Webhook>>;
    async fn get(&self, id: &str) -> WebhookResult<Option<Webhook>>;
    async fn edit(&self, id: &str, input: WebhookInput) -> WebhookResult<Webhook>;
    async fn delete(&self, id: &str) -> WebhookResult<bool>;
}

#[derive(Default)]
pub struct MemoryWebhookRegistry {
    inner: Arc<Mutex<HashMap<String, Webhook>>>,
    // Monotonic id generator. Base36-encoded counter — short, unique
    // within a process, readable in logs. A real deploy would swap in
    // UUIDs, but that's overkill for a single-node local simulator.
    counter: Arc<Mutex<u64>>,
}

impl MemoryWebhookRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    async fn next_id(&self) -> String {
        let mut c = self.counter.lock().await;
        *c += 1;
        format!("wh_{:x}", *c)
    }
}

#[async_trait]
impl WebhookRegistry for MemoryWebhookRegistry {
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
        self.inner.lock().await.insert(id, webhook.clone());
        Ok(webhook)
    }

    async fn list(&self) -> WebhookResult<Vec<Webhook>> {
        let g = self.inner.lock().await;
        let mut out: Vec<Webhook> = g.values().cloned().collect();
        // Deterministic order — by id — makes test assertions stable.
        out.sort_by(|a, b| a.webhook_id.cmp(&b.webhook_id));
        Ok(out)
    }

    async fn get(&self, id: &str) -> WebhookResult<Option<Webhook>> {
        Ok(self.inner.lock().await.get(id).cloned())
    }

    async fn edit(&self, id: &str, input: WebhookInput) -> WebhookResult<Webhook> {
        let mut g = self.inner.lock().await;
        let existing = g
            .get(id)
            .cloned()
            .ok_or_else(|| WebhookError::NotFound { id: id.to_string() })?;
        let merged = Webhook {
            webhook_id: existing.webhook_id.clone(),
            webhook_url: input.webhook_url.unwrap_or(existing.webhook_url),
            account_addresses: input.account_addresses.unwrap_or(existing.account_addresses),
            // Empty list from the user means "clear" — callers who
            // want "keep prior" should omit the field, which comes
            // through as an empty Vec after serde defaulting. We
            // can't disambiguate "omit" vs "set to empty" without a
            // dedicated Option wrapper; keep prior on empty since
            // clearing all filters is the less common case.
            transaction_types: if input.transaction_types.is_empty() {
                existing.transaction_types
            } else {
                input.transaction_types
            },
            txn_status: input.txn_status.or(existing.txn_status),
            webhook_type: input.webhook_type.or(existing.webhook_type),
            auth_header: input.auth_header.or(existing.auth_header),
        };
        g.insert(id.to_string(), merged.clone());
        Ok(merged)
    }

    async fn delete(&self, id: &str) -> WebhookResult<bool> {
        Ok(self.inner.lock().await.remove(id).is_some())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input_with_url(url: &str, addrs: &[&str]) -> WebhookInput {
        WebhookInput {
            webhook_url: Some(url.into()),
            account_addresses: Some(addrs.iter().map(|s| (*s).to_string()).collect()),
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn create_then_get_returns_webhook() {
        let r = MemoryWebhookRegistry::new();
        let wh = r
            .create(input_with_url("https://example.com/hook", &["ADDR1"]))
            .await
            .unwrap();
        assert!(wh.webhook_id.starts_with("wh_"));
        assert_eq!(wh.account_addresses, vec!["ADDR1"]);

        let got = r.get(&wh.webhook_id).await.unwrap().expect("present");
        assert_eq!(got, wh);
    }

    #[tokio::test]
    async fn create_rejects_missing_url_and_empty_addresses() {
        let r = MemoryWebhookRegistry::new();
        assert!(matches!(
            r.create(WebhookInput::default()).await,
            Err(WebhookError::BadRequest(_))
        ));
        assert!(matches!(
            r.create(WebhookInput {
                webhook_url: Some("https://x".into()),
                ..Default::default()
            })
            .await,
            Err(WebhookError::BadRequest(_))
        ));
    }

    #[tokio::test]
    async fn list_is_stable_ordered_and_includes_all_entries() {
        let r = MemoryWebhookRegistry::new();
        let a = r.create(input_with_url("https://a", &["A"])).await.unwrap();
        let b = r.create(input_with_url("https://b", &["B"])).await.unwrap();
        let all = r.list().await.unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].webhook_id, a.webhook_id);
        assert_eq!(all[1].webhook_id, b.webhook_id);
    }

    #[tokio::test]
    async fn edit_merges_and_preserves_prior_fields() {
        let r = MemoryWebhookRegistry::new();
        let orig = r
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
                &orig.webhook_id,
                WebhookInput {
                    account_addresses: Some(vec!["A".into(), "B".into()]),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(edited.webhook_url, "https://x", "url preserved");
        assert_eq!(edited.account_addresses, vec!["A", "B"]);
        assert_eq!(
            edited.transaction_types,
            vec!["NFT_SALE".to_string()],
            "filter list preserved when omitted"
        );
    }

    #[tokio::test]
    async fn delete_removes_webhook() {
        let r = MemoryWebhookRegistry::new();
        let wh = r.create(input_with_url("https://x", &["A"])).await.unwrap();
        assert!(r.delete(&wh.webhook_id).await.unwrap());
        // Second delete returns false (already gone).
        assert!(!r.delete(&wh.webhook_id).await.unwrap());
        assert!(r.get(&wh.webhook_id).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn edit_nonexistent_returns_not_found() {
        let r = MemoryWebhookRegistry::new();
        let err = r.edit("missing", WebhookInput::default()).await.unwrap_err();
        assert!(matches!(err, WebhookError::NotFound { .. }));
    }
}
