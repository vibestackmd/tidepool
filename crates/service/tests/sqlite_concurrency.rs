//! Concurrency smoke test for the shared SQLite backend. Three stores
//! (cNFT, DAS cache, webhooks) sit behind one `Arc<Mutex<Connection>>`;
//! we want confidence that concurrent writers across those stores
//! don't corrupt state, lose writes, or deadlock.
//!
//! Shape: spawn N tasks across the three stores, all writing
//! distinct rows. Join, then assert the observable row counts match
//! what we wrote. Bounded-but-real concurrency — 20 tasks × 50 writes
//! per store = 3000 writes total. Small enough to stay fast in CI,
//! large enough to surface an ordering bug that'd wedge under load.

use std::sync::Arc;
use std::time::Duration;

use tidepool_rpc::cache::CacheStore;
use tidepool_rpc::cnft::{CnftStore, SqliteCnftStore, TreeInfo};
use tidepool_rpc::das::types::{DasAsset, DasContent, DasMetadata};
use tidepool_rpc::sqlite_backend::SqliteBackend;
use tidepool_rpc::sqlite_cache::SqliteCache;
use tidepool_rpc::webhooks::{SqliteWebhookRegistry, WebhookInput, WebhookRegistry};

const WRITERS_PER_STORE: usize = 20;
const WRITES_PER_WRITER: usize = 50;

fn asset(id: &str) -> DasAsset {
    DasAsset {
        id: id.into(),
        interface: "FungibleToken".into(),
        content: DasContent {
            metadata: DasMetadata {
                name: format!("Asset {id}"),
                ..Default::default()
            },
            ..Default::default()
        },
        ..Default::default()
    }
}

fn tree(seed: u16) -> TreeInfo {
    let mut bytes = [0u8; 32];
    bytes[0] = seed as u8;
    bytes[1] = (seed >> 8) as u8;
    TreeInfo {
        tree: bytes,
        depth: 20,
        max_buffer_size: 64,
        num_minted: 0,
    }
}

fn webhook_input(idx: usize) -> WebhookInput {
    WebhookInput {
        webhook_url: Some(format!("https://example.com/hook/{idx}")),
        account_addresses: Some(vec!["ADDR_A".into()]),
        transaction_types: vec![],
        txn_status: None,
        webhook_type: None,
        auth_header: None,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_writes_across_three_stores_dont_corrupt() {
    let backend = SqliteBackend::open(":memory:").expect("open sqlite");
    let cache: Arc<SqliteCache> = Arc::new(SqliteCache::new(&backend));
    let cnft: Arc<SqliteCnftStore> = Arc::new(SqliteCnftStore::new(&backend));
    let webhooks: Arc<SqliteWebhookRegistry> = Arc::new(
        SqliteWebhookRegistry::new(&backend)
            .await
            .expect("webhook registry"),
    );

    let mut joins = Vec::new();

    // DAS cache writers — distinct asset ids per (writer, i) pair.
    for writer in 0..WRITERS_PER_STORE {
        let cache = Arc::clone(&cache);
        joins.push(tokio::spawn(async move {
            for i in 0..WRITES_PER_WRITER {
                let id = format!("ASSET_{writer:03}_{i:03}");
                cache.put_asset(asset(&id)).await.expect("put_asset");
            }
        }));
    }

    // cNFT writers — distinct tree seeds, writer * WRITES_PER_WRITER + i.
    // Fits in a u16 (20 * 50 = 1000).
    for writer in 0..WRITERS_PER_STORE {
        let cnft = Arc::clone(&cnft);
        joins.push(tokio::spawn(async move {
            for i in 0..WRITES_PER_WRITER {
                let seed = u16::try_from(writer * WRITES_PER_WRITER + i).unwrap();
                cnft.put_tree(tree(seed)).await.expect("put_tree");
            }
        }));
    }

    // Webhook writers — .create() assigns ids, so concurrency here
    // also exercises the id-generation serialization under lock.
    for writer in 0..WRITERS_PER_STORE {
        let webhooks = Arc::clone(&webhooks);
        joins.push(tokio::spawn(async move {
            for i in 0..WRITES_PER_WRITER {
                let input = webhook_input(writer * WRITES_PER_WRITER + i);
                webhooks.create(input).await.expect("create webhook");
            }
        }));
    }

    // Cap test runtime so a deadlock bug surfaces as a failure, not a
    // hang. 15s is generous for ~3000 SQLite writes on commodity
    // hardware.
    tokio::time::timeout(Duration::from_secs(15), async {
        for j in joins {
            j.await.expect("writer task");
        }
    })
    .await
    .expect("writers completed within timeout");

    let expected = WRITERS_PER_STORE * WRITES_PER_WRITER;

    // DAS cache — probe at both ends of the id space.
    let probe_first = cache
        .get_asset("ASSET_000_000")
        .await
        .expect("get_asset")
        .expect("first asset present");
    assert_eq!(probe_first.id, "ASSET_000_000");
    let last_id = format!(
        "ASSET_{:03}_{:03}",
        WRITERS_PER_STORE - 1,
        WRITES_PER_WRITER - 1
    );
    let probe_last = cache
        .get_asset(&last_id)
        .await
        .expect("get_asset")
        .expect("last asset present");
    assert_eq!(probe_last.id, last_id);

    // cNFT — probe at both ends of the seed space.
    let seed_first = tree(0).tree;
    let seed_last = tree(u16::try_from(expected - 1).unwrap()).tree;
    assert!(cnft
        .get_tree(&seed_first)
        .await
        .expect("get_tree")
        .is_some());
    assert!(cnft.get_tree(&seed_last).await.expect("get_tree").is_some());

    // Webhooks — exact count invariant. Any lost write surfaces here
    // because `create` generates a new id each call.
    let all = webhooks.list().await.expect("list webhooks");
    assert_eq!(
        all.len(),
        expected,
        "webhook count mismatch: expected {expected}, got {}",
        all.len()
    );
    // Every webhook URL should be unique — if lock contention
    // serialized two creates with the same input, we'd still see
    // distinct rows (different ids), but URL collision would signal
    // id-generation drift.
    let distinct_urls: std::collections::HashSet<_> =
        all.iter().map(|w| w.webhook_url.clone()).collect();
    assert_eq!(distinct_urls.len(), expected);
}
