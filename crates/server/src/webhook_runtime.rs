//! Per-process webhook runtime: registry + background delivery tasks.
//!
//! Wraps the service-layer `WebhookRegistry` trait with a
//! JoinHandle-tracking layer so CRUD operations can also manage the
//! lifecycle of the matching polling task. The server ctx holds one of
//! these.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::Value;
use tokio::sync::Mutex;

use tidepool_rpc::upstream::UpstreamClient;
use tidepool_rpc::webhooks::{
    spawn_delivery_task, MemoryWebhookRegistry, PostClient, Webhook, WebhookInput,
    WebhookRegistry, WebhookResult,
};

/// Default-backed `PostClient` — reqwest under the hood. Placed in
/// the server crate so the service layer stays reqwest-free.
pub struct ReqwestPostClient {
    client: reqwest::Client,
}

impl ReqwestPostClient {
    #[must_use]
    pub fn new(timeout: Duration) -> Self {
        let client = reqwest::Client::builder()
            .timeout(timeout)
            .build()
            .expect("build reqwest client");
        Self { client }
    }
}

#[async_trait]
impl PostClient for ReqwestPostClient {
    async fn post_json(&self, url: &str, auth: Option<&str>, body: &Value) -> Result<(), String> {
        let mut req = self.client.post(url).json(body);
        if let Some(h) = auth {
            req = req.header("Authorization", h);
        }
        let resp = req.send().await.map_err(|e| e.to_string())?;
        if !resp.status().is_success() {
            return Err(format!("delivery non-2xx: {}", resp.status()));
        }
        Ok(())
    }
}

/// Lifecycle owner for webhooks. CRUD goes through the underlying
/// `WebhookRegistry`; task lifecycle (spawn on create, abort on delete)
/// is mirrored in the parallel `handles` map.
pub struct WebhookRuntime<U: UpstreamClient + ?Sized + 'static, P: PostClient + ?Sized + 'static> {
    registry: Arc<dyn WebhookRegistry>,
    upstream: Arc<U>,
    poster: Arc<P>,
    handles: Arc<Mutex<HashMap<String, tokio::task::JoinHandle<()>>>>,
}

impl<U: UpstreamClient + ?Sized + 'static, P: PostClient + ?Sized + 'static> WebhookRuntime<U, P> {
    pub fn new(registry: Arc<dyn WebhookRegistry>, upstream: Arc<U>, poster: Arc<P>) -> Self {
        Self {
            registry,
            upstream,
            poster,
            handles: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Convenience: default-backed in-memory registry.
    pub fn with_memory_registry(upstream: Arc<U>, poster: Arc<P>) -> Self {
        Self::new(Arc::new(MemoryWebhookRegistry::new()), upstream, poster)
    }

    pub async fn create(&self, input: WebhookInput) -> WebhookResult<Webhook> {
        let wh = self.registry.create(input).await?;
        let handle = spawn_delivery_task(
            wh.clone(),
            Arc::clone(&self.upstream),
            Arc::clone(&self.poster),
        );
        self.handles
            .lock()
            .await
            .insert(wh.webhook_id.clone(), handle);
        Ok(wh)
    }

    pub async fn list(&self) -> WebhookResult<Vec<Webhook>> {
        self.registry.list().await
    }

    pub async fn get(&self, id: &str) -> WebhookResult<Option<Webhook>> {
        self.registry.get(id).await
    }

    pub async fn edit(&self, id: &str, input: WebhookInput) -> WebhookResult<Webhook> {
        let wh = self.registry.edit(id, input).await?;
        // Restart the delivery task with the new config — existing
        // cursor state is dropped, same as Helius's behavior when a
        // webhook is edited.
        let mut handles = self.handles.lock().await;
        if let Some(prior) = handles.remove(id) {
            prior.abort();
        }
        let handle = spawn_delivery_task(
            wh.clone(),
            Arc::clone(&self.upstream),
            Arc::clone(&self.poster),
        );
        handles.insert(id.to_string(), handle);
        Ok(wh)
    }

    pub async fn delete(&self, id: &str) -> WebhookResult<bool> {
        let removed = self.registry.delete(id).await?;
        if let Some(handle) = self.handles.lock().await.remove(id) {
            handle.abort();
        }
        Ok(removed)
    }
}
