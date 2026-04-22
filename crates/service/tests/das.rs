//! DAS handler integration tests. Seed a cNFT into a store via
//! apply_event, then hit `get_asset` / `get_asset_proof` /
//! `get_asset_proof_batch` directly (service-layer fns, no HTTP).

use tidepool_rpc::cnft::{
    apply::derive_asset_id, apply_event, CnftEvent, MemoryCnftStore, MintMetadata,
};
use tidepool_rpc::das::{get_asset, get_asset_proof, get_asset_proof_batch};
use tidepool_rpc::verify_proof;
use tidepool_rpc_core::Creator;

const TREE: [u8; 32] = [0x11; 32];

fn stub_mint_metadata() -> MintMetadata {
    MintMetadata {
        name: "Compressed".into(),
        symbol: "CMP".into(),
        uri: "https://example.com/cnft.json".into(),
        seller_fee_basis_points: 250,
        primary_sale_happened: false,
        is_mutable: true,
        creators: vec![Creator {
            address: [0x44; 32],
            verified: false,
            share: 100,
        }],
        collection: None,
        data_hash_input: br#"{"name":"Compressed"}"#.to_vec(),
    }
}

async fn seed(store: &MemoryCnftStore) -> [u8; 32] {
    apply_event(
        store,
        CnftEvent::CreateTree {
            tree: TREE,
            depth: 6,
            max_buffer_size: 8,
        },
    )
    .await
    .unwrap();
    apply_event(
        store,
        CnftEvent::Mint {
            tree: TREE,
            owner: [0x22; 32],
            delegate: [0x33; 32],
            metadata: stub_mint_metadata(),
            verify_collection: None,
            noop: None,
        },
    )
    .await
    .unwrap();
    derive_asset_id(&TREE, 0)
}

fn bs58_to_bytes(s: &str) -> Vec<u8> {
    bs58::decode(s).into_vec().unwrap()
}

#[tokio::test]
async fn get_asset_returns_cnft_shape_for_indexed_asset() {
    let store = MemoryCnftStore::new();
    let asset_id = seed(&store).await;

    let asset = get_asset(&store, &asset_id).await.unwrap().expect("Some");
    assert_eq!(asset.id, bs58::encode(asset_id).into_string());
    assert_eq!(asset.interface, "V1_NFT");

    let compression = asset.compression.as_ref().expect("compression present");
    assert!(compression.compressed);
    assert!(compression.eligible);
    assert_eq!(compression.tree, bs58::encode(TREE).into_string());
    assert_eq!(compression.leaf_id, 0);

    // owner (0x22) != delegate (0x33) → delegated=true
    assert!(asset.ownership.delegated);
}

#[tokio::test]
async fn get_asset_returns_none_for_unknown_id() {
    let store = MemoryCnftStore::new();
    let unknown = [0xee; 32];
    assert!(get_asset(&store, &unknown).await.unwrap().is_none());
}

#[tokio::test]
async fn get_asset_proof_round_trips_against_verify_proof() {
    let store = MemoryCnftStore::new();
    let asset_id = seed(&store).await;

    let proof = get_asset_proof(&store, &asset_id).await.unwrap().expect("Some");
    assert_eq!(proof.tree_id, bs58::encode(TREE).into_string());
    // depth 6 → node_index = 2^6 + leaf_index(0) = 64
    assert_eq!(proof.node_index, 64);
    assert_eq!(proof.proof.len(), 6);

    // Decode base58 and cross-verify.
    let leaf_bytes: [u8; 32] = bs58_to_bytes(&proof.leaf).try_into().unwrap();
    let root_bytes: [u8; 32] = bs58_to_bytes(&proof.root).try_into().unwrap();
    let proof_nodes: Vec<[u8; 32]> = proof
        .proof
        .iter()
        .map(|s| bs58_to_bytes(s).try_into().unwrap())
        .collect();
    assert!(verify_proof(&leaf_bytes, &proof_nodes, 0, &root_bytes));
}

#[tokio::test]
async fn get_asset_proof_returns_none_for_unknown_asset() {
    let store = MemoryCnftStore::new();
    let unknown = [0xcc; 32];
    assert!(get_asset_proof(&store, &unknown).await.unwrap().is_none());
}

#[tokio::test]
async fn get_asset_proof_batch_returns_ordered_nulls_for_misses() {
    let store = MemoryCnftStore::new();
    let known = seed(&store).await;
    let unknown = [0xaa; 32];

    let results = get_asset_proof_batch(&store, &[known, unknown, known])
        .await
        .unwrap();
    assert_eq!(results.len(), 3);
    assert!(results[0].is_some(), "known id → proof");
    assert!(results[1].is_none(), "unknown id → None");
    assert!(results[2].is_some(), "known id again → proof");
}

#[tokio::test]
async fn get_asset_proof_batch_shares_tree_state_across_asset_ids() {
    // Not asserting the sharing directly — there's no hook — but a
    // batch over multiple leaves in one tree should still produce
    // verifiable proofs for each. If the tree-state materialization
    // regresses we'll see it here.
    let store = MemoryCnftStore::new();
    apply_event(
        &store,
        CnftEvent::CreateTree {
            tree: TREE,
            depth: 5,
            max_buffer_size: 8,
        },
    )
    .await
    .unwrap();
    let mut asset_ids = Vec::new();
    for i in 0u8..4 {
        apply_event(
            &store,
            CnftEvent::Mint {
                tree: TREE,
                owner: [i; 32],
                delegate: [i; 32],
                metadata: stub_mint_metadata(),
                verify_collection: None,
                noop: None,
            },
        )
        .await
        .unwrap();
        asset_ids.push(derive_asset_id(&TREE, u64::from(i)));
    }

    let results = get_asset_proof_batch(&store, &asset_ids).await.unwrap();
    assert_eq!(results.len(), 4);
    for (i, proof) in results.iter().enumerate() {
        let proof = proof.as_ref().expect("Some");
        let leaf: [u8; 32] = bs58_to_bytes(&proof.leaf).try_into().unwrap();
        let root: [u8; 32] = bs58_to_bytes(&proof.root).try_into().unwrap();
        let nodes: Vec<[u8; 32]> = proof
            .proof
            .iter()
            .map(|s| bs58_to_bytes(s).try_into().unwrap())
            .collect();
        assert!(
            verify_proof(&leaf, &nodes, i as u64, &root),
            "proof {i} failed to verify"
        );
    }
}
