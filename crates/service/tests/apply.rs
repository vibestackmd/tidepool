//! apply_event integration tests. Feed sequences of CnftEvents
//! through apply, then check the resulting store state + merkle math.

use std::collections::BTreeMap;

use tidepool_rpc::cnft::{
    apply::derive_asset_id, apply_event, CnftEvent, CnftStore, MemoryCnftStore, MintMetadata,
    NoopOverride,
};
use tidepool_rpc::{compute_proof, verify_proof, TreeState};
use tidepool_rpc_core::Creator;

const TREE: [u8; 32] = [0x11; 32];

fn stub_mint_metadata() -> MintMetadata {
    MintMetadata {
        name: "Asset".into(),
        symbol: "AST".into(),
        uri: "https://example.com/a.json".into(),
        seller_fee_basis_points: 500,
        primary_sale_happened: false,
        is_mutable: true,
        creators: vec![Creator {
            address: [0x44; 32],
            verified: false,
            share: 100,
        }],
        collection: None,
        data_hash_input: br#"{"name":"Asset"}"#.to_vec(),
    }
}

async fn build_tree_state(
    store: &MemoryCnftStore,
    tree: [u8; 32],
) -> TreeState {
    let info = store.get_tree(&tree).await.unwrap().expect("tree present");
    let mut leaves = BTreeMap::new();
    for rec in store.list_leaves(&tree).await.unwrap() {
        if !rec.burned {
            leaves.insert(rec.leaf_index, rec.leaf_hash);
        }
    }
    TreeState {
        depth: info.depth,
        leaves,
    }
}

// ─── base cases ─────────────────────────────────────────────────────

#[tokio::test]
async fn create_tree_populates_with_zero_mints() {
    let store = MemoryCnftStore::new();
    apply_event(
        &store,
        CnftEvent::CreateTree {
            tree: TREE,
            depth: 10,
            max_buffer_size: 16,
        },
    )
    .await
    .unwrap();
    let info = store.get_tree(&TREE).await.unwrap().unwrap();
    assert_eq!(info.depth, 10);
    assert_eq!(info.num_minted, 0);
}

#[tokio::test]
async fn mint_allocates_index_0_and_derives_asset_id() {
    let store = MemoryCnftStore::new();
    apply_event(
        &store,
        CnftEvent::CreateTree {
            tree: TREE,
            depth: 10,
            max_buffer_size: 16,
        },
    )
    .await
    .unwrap();

    apply_event(
        &store,
        CnftEvent::Mint {
            tree: TREE,
            owner: [7; 32],
            delegate: [8; 32],
            metadata: stub_mint_metadata(),
            verify_collection: None,
            noop: None,
        },
    )
    .await
    .unwrap();

    assert_eq!(store.get_tree(&TREE).await.unwrap().unwrap().num_minted, 1);
    let expected = derive_asset_id(&TREE, 0);
    let rec = store.get_leaf(&expected).await.unwrap().unwrap();
    assert_eq!(rec.leaf_index, 0);
    assert_eq!(rec.nonce, 0);
    assert!(!rec.burned);
    assert!(rec.leaf_hash.iter().any(|&b| b != 0));
}

#[tokio::test]
async fn mint_on_unknown_tree_errors() {
    let store = MemoryCnftStore::new();
    let err = apply_event(
        &store,
        CnftEvent::Mint {
            tree: TREE,
            owner: [0; 32],
            delegate: [0; 32],
            metadata: stub_mint_metadata(),
            verify_collection: None,
            noop: None,
        },
    )
    .await
    .unwrap_err();
    assert!(format!("{err}").contains("unknown tree"));
}

#[tokio::test]
async fn sequential_mints_get_monotonic_leaf_indices() {
    let store = MemoryCnftStore::new();
    apply_event(
        &store,
        CnftEvent::CreateTree {
            tree: TREE,
            depth: 10,
            max_buffer_size: 16,
        },
    )
    .await
    .unwrap();
    for i in 0..3 {
        apply_event(
            &store,
            CnftEvent::Mint {
                tree: TREE,
                owner: [10 + i; 32],
                delegate: [20 + i; 32],
                metadata: stub_mint_metadata(),
                verify_collection: None,
                noop: None,
            },
        )
        .await
        .unwrap();
    }
    let leaves = store.list_leaves(&TREE).await.unwrap();
    assert_eq!(
        leaves.iter().map(|l| l.leaf_index).collect::<Vec<_>>(),
        vec![0, 1, 2]
    );
}

// ─── transfer / delegate / burn ─────────────────────────────────────

#[tokio::test]
async fn transfer_updates_owner_and_delegate_and_rehashes_leaf() {
    let store = MemoryCnftStore::new();
    apply_event(
        &store,
        CnftEvent::CreateTree {
            tree: TREE,
            depth: 10,
            max_buffer_size: 16,
        },
    )
    .await
    .unwrap();
    apply_event(
        &store,
        CnftEvent::Mint {
            tree: TREE,
            owner: [1; 32],
            delegate: [2; 32],
            metadata: stub_mint_metadata(),
            verify_collection: None,
            noop: None,
        },
    )
    .await
    .unwrap();

    let mid = store.get_leaf_by_index(&TREE, 0).await.unwrap().unwrap();
    let original_hash = mid.leaf_hash;

    apply_event(
        &store,
        CnftEvent::Transfer {
            tree: TREE,
            leaf_index: 0,
            nonce: 0,
            new_owner: [9; 32],
            new_delegate: [9; 32],
            data_hash: mid.data_hash,
            creator_hash: mid.creator_hash,
            noop: None,
        },
    )
    .await
    .unwrap();

    let after = store.get_leaf_by_index(&TREE, 0).await.unwrap().unwrap();
    assert_eq!(after.owner, [9; 32]);
    assert_eq!(after.delegate, [9; 32]);
    assert_ne!(after.leaf_hash, original_hash);
}

#[tokio::test]
async fn transfer_with_wrong_data_hash_and_no_noop_is_silent_noop() {
    let store = MemoryCnftStore::new();
    apply_event(
        &store,
        CnftEvent::CreateTree {
            tree: TREE,
            depth: 10,
            max_buffer_size: 16,
        },
    )
    .await
    .unwrap();
    apply_event(
        &store,
        CnftEvent::Mint {
            tree: TREE,
            owner: [0; 32],
            delegate: [0; 32],
            metadata: stub_mint_metadata(),
            verify_collection: None,
            noop: None,
        },
    )
    .await
    .unwrap();
    let before = store.get_leaf_by_index(&TREE, 0).await.unwrap().unwrap();

    apply_event(
        &store,
        CnftEvent::Transfer {
            tree: TREE,
            leaf_index: 0,
            nonce: 0,
            new_owner: [1; 32],
            new_delegate: [1; 32],
            // Wrong dataHash + no noop override → divergence, skip.
            data_hash: [0xff; 32],
            creator_hash: before.creator_hash,
            noop: None,
        },
    )
    .await
    .unwrap();

    let after = store.get_leaf_by_index(&TREE, 0).await.unwrap().unwrap();
    assert_eq!(after.leaf_hash, before.leaf_hash, "mismatch should not mutate");
}

#[tokio::test]
async fn delegate_updates_just_the_delegate() {
    let store = MemoryCnftStore::new();
    apply_event(
        &store,
        CnftEvent::CreateTree {
            tree: TREE,
            depth: 10,
            max_buffer_size: 16,
        },
    )
    .await
    .unwrap();
    apply_event(
        &store,
        CnftEvent::Mint {
            tree: TREE,
            owner: [1; 32],
            delegate: [2; 32],
            metadata: stub_mint_metadata(),
            verify_collection: None,
            noop: None,
        },
    )
    .await
    .unwrap();
    let mid = store.get_leaf_by_index(&TREE, 0).await.unwrap().unwrap();

    apply_event(
        &store,
        CnftEvent::Delegate {
            tree: TREE,
            leaf_index: 0,
            nonce: 0,
            new_delegate: [5; 32],
            data_hash: mid.data_hash,
            creator_hash: mid.creator_hash,
            noop: None,
        },
    )
    .await
    .unwrap();

    let after = store.get_leaf_by_index(&TREE, 0).await.unwrap().unwrap();
    assert_eq!(after.owner, [1; 32]);
    assert_eq!(after.delegate, [5; 32]);
}

#[tokio::test]
async fn burn_marks_leaf_with_zeroed_hash() {
    let store = MemoryCnftStore::new();
    apply_event(
        &store,
        CnftEvent::CreateTree {
            tree: TREE,
            depth: 10,
            max_buffer_size: 16,
        },
    )
    .await
    .unwrap();
    apply_event(
        &store,
        CnftEvent::Mint {
            tree: TREE,
            owner: [0; 32],
            delegate: [0; 32],
            metadata: stub_mint_metadata(),
            verify_collection: None,
            noop: None,
        },
    )
    .await
    .unwrap();
    apply_event(
        &store,
        CnftEvent::Burn {
            tree: TREE,
            leaf_index: 0,
            nonce: 0,
            noop: None,
        },
    )
    .await
    .unwrap();

    let rec = store.get_leaf_by_index(&TREE, 0).await.unwrap().unwrap();
    assert!(rec.burned);
    assert_eq!(rec.leaf_hash, [0u8; 32]);
}

// ─── noop-required family ───────────────────────────────────────────

#[tokio::test]
async fn verify_creator_flips_the_matching_creator_and_adopts_noop_hashes() {
    let store = MemoryCnftStore::new();
    apply_event(
        &store,
        CnftEvent::CreateTree {
            tree: TREE,
            depth: 8,
            max_buffer_size: 16,
        },
    )
    .await
    .unwrap();
    apply_event(
        &store,
        CnftEvent::Mint {
            tree: TREE,
            owner: [1; 32],
            delegate: [2; 32],
            metadata: stub_mint_metadata(),
            verify_collection: None,
            noop: None,
        },
    )
    .await
    .unwrap();

    apply_event(
        &store,
        CnftEvent::VerifyCreator {
            tree: TREE,
            creator: [0x44; 32],
            noop: NoopOverride {
                leaf_index: 0,
                nonce: 0,
                owner: [1; 32],
                delegate: [2; 32],
                data_hash: [0xaa; 32],
                creator_hash: [0xbb; 32],
            },
        },
    )
    .await
    .unwrap();

    let asset_id = derive_asset_id(&TREE, 0);
    let rec = store.get_leaf(&asset_id).await.unwrap().unwrap();
    assert_eq!(rec.data_hash, [0xaa; 32]);
    assert_eq!(rec.creator_hash, [0xbb; 32]);
    assert!(rec.mint_metadata.creators[0].verified);
}

#[tokio::test]
async fn set_and_verify_collection_marks_collection_and_updates_hashes() {
    let store = MemoryCnftStore::new();
    apply_event(
        &store,
        CnftEvent::CreateTree {
            tree: TREE,
            depth: 8,
            max_buffer_size: 16,
        },
    )
    .await
    .unwrap();
    apply_event(
        &store,
        CnftEvent::Mint {
            tree: TREE,
            owner: [1; 32],
            delegate: [2; 32],
            metadata: stub_mint_metadata(),
            verify_collection: None,
            noop: None,
        },
    )
    .await
    .unwrap();

    apply_event(
        &store,
        CnftEvent::SetAndVerifyCollection {
            tree: TREE,
            collection: [0x77; 32],
            noop: NoopOverride {
                leaf_index: 0,
                nonce: 0,
                owner: [1; 32],
                delegate: [2; 32],
                data_hash: [0xfe; 32],
                creator_hash: [0xfd; 32],
            },
        },
    )
    .await
    .unwrap();

    let asset_id = derive_asset_id(&TREE, 0);
    let rec = store.get_leaf(&asset_id).await.unwrap().unwrap();
    assert_eq!(rec.mint_metadata.collection, Some(([0x77; 32], true)));
    assert_eq!(rec.data_hash, [0xfe; 32]);
}

// ─── full-flow + merkle cross-check ─────────────────────────────────

#[tokio::test]
async fn full_flow_mint_transfer_burn_produces_verifiable_proofs() {
    let store = MemoryCnftStore::new();
    apply_event(
        &store,
        CnftEvent::CreateTree {
            tree: TREE,
            depth: 8,
            max_buffer_size: 16,
        },
    )
    .await
    .unwrap();

    for b in 1u8..=2 {
        apply_event(
            &store,
            CnftEvent::Mint {
                tree: TREE,
                owner: [b; 32],
                delegate: [b; 32],
                metadata: stub_mint_metadata(),
                verify_collection: None,
                noop: None,
            },
        )
        .await
        .unwrap();
    }

    let leaf0 = store.get_leaf_by_index(&TREE, 0).await.unwrap().unwrap();
    apply_event(
        &store,
        CnftEvent::Transfer {
            tree: TREE,
            leaf_index: 0,
            nonce: 0,
            new_owner: [9; 32],
            new_delegate: [9; 32],
            data_hash: leaf0.data_hash,
            creator_hash: leaf0.creator_hash,
            noop: None,
        },
    )
    .await
    .unwrap();

    let state = build_tree_state(&store, TREE).await;
    for idx in [0u64, 1] {
        let p = compute_proof(&state, idx).unwrap();
        assert!(
            verify_proof(&p.leaf, &p.proof, idx, &p.root),
            "proof failed at {idx}"
        );
    }

    // Burn leaf 1, re-verify — burned slot should be the empty leaf.
    apply_event(
        &store,
        CnftEvent::Burn {
            tree: TREE,
            leaf_index: 1,
            nonce: 1,
            noop: None,
        },
    )
    .await
    .unwrap();
    let state2 = build_tree_state(&store, TREE).await;
    let p1 = compute_proof(&state2, 1).unwrap();
    assert_eq!(p1.leaf, [0u8; 32]);
    assert!(verify_proof(&p1.leaf, &p1.proof, 1, &p1.root));
}
