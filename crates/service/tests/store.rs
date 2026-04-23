//! CnftStore integration tests against the in-memory impl.

use tidepool_rpc::cnft::{
    types::{LeafRecord, MintMetadata, TreeInfo},
    CnftStore, MemoryCnftStore,
};

const TREE: [u8; 32] = [0x11; 32];
const OTHER_TREE: [u8; 32] = [0x22; 32];
const ASSET: [u8; 32] = [0x33; 32];

fn stub_mint_metadata() -> MintMetadata {
    MintMetadata {
        name: "n".into(),
        symbol: "s".into(),
        uri: "u".into(),
        seller_fee_basis_points: 0,
        primary_sale_happened: false,
        is_mutable: true,
        creators: vec![],
        collection: None,
        data_hash_input: vec![0; 16],
    }
}

fn stub_leaf(asset_id: [u8; 32], leaf_index: u64) -> LeafRecord {
    LeafRecord {
        asset_id,
        tree: TREE,
        nonce: leaf_index,
        leaf_index,
        mint_metadata: stub_mint_metadata(),
        owner: [1; 32],
        delegate: [1; 32],
        data_hash: [2; 32],
        creator_hash: [3; 32],
        leaf_hash: [4; 32],
        burned: false,
    }
}

#[tokio::test]
async fn put_get_tree_roundtrip() {
    let store = MemoryCnftStore::new();
    store
        .put_tree(TreeInfo {
            tree: TREE,
            depth: 20,
            max_buffer_size: 64,
            num_minted: 0,
        })
        .await
        .unwrap();
    let t = store.get_tree(&TREE).await.unwrap().unwrap();
    assert_eq!(t.depth, 20);
    assert_eq!(t.num_minted, 0);
}

#[tokio::test]
async fn get_tree_returns_none_for_unknown() {
    let store = MemoryCnftStore::new();
    assert!(store.get_tree(&OTHER_TREE).await.unwrap().is_none());
}

#[tokio::test]
async fn alloc_leaf_index_is_monotonic() {
    let store = MemoryCnftStore::new();
    store
        .put_tree(TreeInfo {
            tree: TREE,
            depth: 20,
            max_buffer_size: 64,
            num_minted: 0,
        })
        .await
        .unwrap();
    assert_eq!(store.alloc_leaf_index(&TREE).await.unwrap(), 0);
    assert_eq!(store.alloc_leaf_index(&TREE).await.unwrap(), 1);
    assert_eq!(store.alloc_leaf_index(&TREE).await.unwrap(), 2);
    let t = store.get_tree(&TREE).await.unwrap().unwrap();
    assert_eq!(t.num_minted, 3);
}

#[tokio::test]
async fn alloc_leaf_index_errors_on_unknown_tree() {
    let store = MemoryCnftStore::new();
    let err = store.alloc_leaf_index(&TREE).await.unwrap_err();
    assert!(format!("{err}").contains("unknown tree"));
}

#[tokio::test]
async fn ensure_num_minted_at_least_only_raises() {
    let store = MemoryCnftStore::new();
    store
        .put_tree(TreeInfo {
            tree: TREE,
            depth: 10,
            max_buffer_size: 8,
            num_minted: 5,
        })
        .await
        .unwrap();
    // Lower floor → no-op.
    store.ensure_num_minted_at_least(&TREE, 3).await.unwrap();
    assert_eq!(store.get_tree(&TREE).await.unwrap().unwrap().num_minted, 5);
    // Higher floor → raised.
    store.ensure_num_minted_at_least(&TREE, 10).await.unwrap();
    assert_eq!(store.get_tree(&TREE).await.unwrap().unwrap().num_minted, 10);
}

#[tokio::test]
async fn put_leaf_and_lookup_by_id_and_index() {
    let store = MemoryCnftStore::new();
    store
        .put_tree(TreeInfo {
            tree: TREE,
            depth: 10,
            max_buffer_size: 8,
            num_minted: 0,
        })
        .await
        .unwrap();
    store.put_leaf(stub_leaf(ASSET, 0)).await.unwrap();

    let by_id = store.get_leaf(&ASSET).await.unwrap().unwrap();
    assert_eq!(by_id.asset_id, ASSET);

    let by_idx = store.get_leaf_by_index(&TREE, 0).await.unwrap().unwrap();
    assert_eq!(by_idx.asset_id, ASSET);
}

#[tokio::test]
async fn put_leaf_replaces_at_same_position() {
    let store = MemoryCnftStore::new();
    let mut rec = stub_leaf(ASSET, 5);
    store.put_leaf(rec.clone()).await.unwrap();
    rec.owner = [0x9; 32];
    store.put_leaf(rec).await.unwrap();
    let got = store.get_leaf(&ASSET).await.unwrap().unwrap();
    assert_eq!(got.owner[0], 0x9);
}

#[tokio::test]
async fn list_leaves_is_insertion_ordered_and_tree_scoped() {
    let store = MemoryCnftStore::new();
    let a = stub_leaf([0xa1; 32], 0);
    let b = stub_leaf([0xb1; 32], 1);
    let mut other = stub_leaf([0xc1; 32], 0);
    other.tree = OTHER_TREE;

    store.put_leaf(a).await.unwrap();
    store.put_leaf(b).await.unwrap();
    store.put_leaf(other).await.unwrap();

    let for_tree = store.list_leaves(&TREE).await.unwrap();
    assert_eq!(for_tree.len(), 2);
    assert_eq!(for_tree[0].leaf_index, 0);
    assert_eq!(for_tree[1].leaf_index, 1);

    let for_other = store.list_leaves(&OTHER_TREE).await.unwrap();
    assert_eq!(for_other.len(), 1);
    assert_eq!(for_other[0].tree, OTHER_TREE);
}

#[tokio::test]
async fn last_signature_is_per_tree() {
    let store = MemoryCnftStore::new();
    assert!(store.get_last_signature(&TREE).await.unwrap().is_none());
    store
        .set_last_signature(&TREE, "sig-abc".into())
        .await
        .unwrap();
    assert_eq!(
        store.get_last_signature(&TREE).await.unwrap().as_deref(),
        Some("sig-abc")
    );
    assert!(store
        .get_last_signature(&OTHER_TREE)
        .await
        .unwrap()
        .is_none());
}
