//! Rust library integration example.
//!
//! Shows the shape of composing Tidepool's service layer in a Rust
//! test or tool — no HTTP server, no CLI, just direct async fn calls.
//! Scenario: seed a Bubblegum tree in the in-memory store, mint two
//! cNFTs, then pull a DAS proof for one of them.
//!
//! Run with:
//!
//! ```text
//! cargo run -p tidepool-example-rust-integration
//! ```

use tidepool_core::Creator;
use tidepool_rpc::cnft::{
    apply::derive_asset_id, apply_event, CnftEvent, CnftStore, MemoryCnftStore, MintMetadata,
};
use tidepool_rpc::das::{get_asset, get_asset_proof};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 1. Stand up the cNFT store — the one piece of state cNFT
    //    handlers need. In a real app you'd share an `Arc<MemoryCnftStore>`
    //    (or a SQLite-backed impl) across your whole service.
    let cnft = MemoryCnftStore::new();

    // 2. Pretend-index a tree + mint two assets on it by applying the
    //    events directly. In production the indexer drives this from
    //    a live RPC; here we're exercising the pure state transition
    //    pipeline.
    let tree = [0x11u8; 32];
    apply_event(
        &cnft,
        CnftEvent::CreateTree {
            tree,
            depth: 8,
            max_buffer_size: 16,
        },
    )
    .await?;

    for seed in 1u8..=2 {
        apply_event(
            &cnft,
            CnftEvent::Mint {
                tree,
                owner: [seed; 32],
                delegate: [seed; 32],
                metadata: MintMetadata {
                    name: format!("Asset #{seed}"),
                    symbol: "DEMO".into(),
                    uri: "https://example.com/demo.json".into(),
                    seller_fee_basis_points: 500,
                    primary_sale_happened: false,
                    is_mutable: true,
                    creators: vec![Creator {
                        address: [seed; 32],
                        verified: true,
                        share: 100,
                    }],
                    collection: None,
                    data_hash_input: format!(r#"{{"name":"Asset #{seed}"}}"#).into_bytes(),
                },
                verify_collection: None,
                noop: None,
            },
        )
        .await?;
    }

    // 3. Derive the asset id of the first mint, then pull its DAS
    //    shape and a merkle proof. This is exactly what a Helius
    //    client calling `getAsset` + `getAssetProof` does over the
    //    wire — only here we're in-process, no round-trip.
    let id_bytes = derive_asset_id(&tree, 0);
    let asset_id_b58 = bs58::encode(id_bytes).into_string();

    let asset = get_asset(&cnft, &id_bytes).await?.expect("seeded asset");
    println!("== getAsset ==");
    println!("  id:        {asset_id_b58}");
    println!("  interface: {}", asset.interface);
    println!("  owner:     {}", asset.ownership.owner);
    println!("  name:      {}", asset.content.metadata.name);
    if let Some(c) = &asset.compression {
        println!("  tree:      {}", c.tree);
        println!("  leaf_id:   {}", c.leaf_id);
        println!("  asset_hash: {}", c.asset_hash);
    }

    let proof = get_asset_proof(&cnft, &id_bytes)
        .await?
        .expect("proof available");
    println!("\n== getAssetProof ==");
    println!("  root:       {}", proof.root);
    println!("  node_index: {}", proof.node_index);
    println!("  proof nodes: {} entries", proof.proof.len());

    // 4. Sanity: list everything on the tree via the store surface.
    let leaves = cnft.list_leaves(&tree).await?;
    println!("\n== tree inventory ==");
    println!("  {} leaves indexed", leaves.len());

    Ok(())
}
