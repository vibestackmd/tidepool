//! fetch_and_cache_asset integration test. We synthesize a real
//! MplCore AssetV1 account via mpl-core types, hand it through
//! `FixtureUpstream`, and verify the fetch pipeline:
//! upstream → decoder dispatch → cache populate.

use std::sync::Arc;

use borsh::BorshSerialize;
use mpl_core::accounts::BaseAssetV1;
use mpl_core::types::{Key, UpdateAuthority};
use mpl_core::ID as MPL_CORE_ID;
use solana_program::pubkey::Pubkey;

use tidepool_rpc::cache::{CacheStore, MemoryCache};
use tidepool_rpc::das::{fetch_and_cache_asset, AccountDecoder, MplCoreDecoder};
use tidepool_rpc::upstream::{AccountData, FixtureUpstream};

#[tokio::test]
async fn fetch_mpl_core_asset_decodes_and_caches() {
    let mint_b58 = "AssetTest11111111111111111111111111111111111";
    let owner = Pubkey::new_from_array([0x22; 32]);

    let asset = BaseAssetV1 {
        key: Key::AssetV1,
        owner,
        update_authority: UpdateAuthority::Address(Pubkey::new_from_array([0x33; 32])),
        name: "Core Test".into(),
        uri: "https://example.com/c.json".into(),
        seq: None,
    };
    let mut account_data = Vec::new();
    asset.serialize(&mut account_data).expect("serialize");

    // Stub upstream with the MplCore-owned account.
    let upstream = FixtureUpstream::new().with_account(
        mint_b58,
        AccountData {
            data: account_data,
            owner: MPL_CORE_ID.to_bytes(),
            lamports: 1_000_000,
        },
    );

    let cache = MemoryCache::new();
    let decoders: Vec<Arc<dyn AccountDecoder>> = vec![Arc::new(MplCoreDecoder)];

    let got = fetch_and_cache_asset(&upstream, &cache, &decoders, mint_b58)
        .await
        .unwrap()
        .expect("Some");
    assert_eq!(got.id, mint_b58);
    assert_eq!(got.interface, "MplCoreAsset");
    assert_eq!(got.ownership.owner, owner.to_string());

    // Cache was populated — second fetch hits it without running decoder again.
    let cached = cache.get_asset(mint_b58).await.unwrap();
    assert!(cached.is_some());
}

#[tokio::test]
async fn fetch_returns_none_for_missing_account() {
    let upstream = FixtureUpstream::new();
    let cache = MemoryCache::new();
    let decoders: Vec<Arc<dyn AccountDecoder>> = vec![Arc::new(MplCoreDecoder)];

    let got = fetch_and_cache_asset(&upstream, &cache, &decoders, "11111111111111111111111111111111")
        .await
        .unwrap();
    assert!(got.is_none());
}

#[tokio::test]
async fn fetch_returns_none_for_unknown_owner_program() {
    let upstream = FixtureUpstream::new().with_account(
        "randompk",
        AccountData {
            data: vec![0; 100],
            owner: [0x99; 32], // not a registered decoder's program
            lamports: 1_000,
        },
    );
    let cache = MemoryCache::new();
    let decoders: Vec<Arc<dyn AccountDecoder>> = vec![Arc::new(MplCoreDecoder)];

    let got = fetch_and_cache_asset(&upstream, &cache, &decoders, "randompk")
        .await
        .unwrap();
    assert!(got.is_none(), "no decoder match → None");
}
