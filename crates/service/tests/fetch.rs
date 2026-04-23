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
use tidepool_rpc::das::{
    fetch_and_cache_asset, resolve_owner_for_mint, AccountDecoder, MplCoreDecoder,
};
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

    let got = fetch_and_cache_asset(
        &upstream,
        &cache,
        &decoders,
        "11111111111111111111111111111111",
    )
    .await
    .unwrap();
    assert!(got.is_none());
}

/// Build a 165-byte SPL Token account: bytes 0..32 mint, 32..64 owner,
/// 64..72 amount LE. Zero-fill the rest (delegate, state, etc — not
/// read by our resolver).
fn spl_token_account_bytes(mint: [u8; 32], owner: [u8; 32], amount: u64) -> Vec<u8> {
    let mut v = vec![0u8; 165];
    v[0..32].copy_from_slice(&mint);
    v[32..64].copy_from_slice(&owner);
    v[64..72].copy_from_slice(&amount.to_le_bytes());
    v
}

#[tokio::test]
async fn resolve_owner_for_mint_returns_top_holder_wallet() {
    let mint = "MintResolveTest11111111111111111111111111111";
    let token_account_addr = "TokAcctResolveTest111111111111111111111111111";
    let owner_bytes = [0x44u8; 32];
    let owner_b58 = bs58::encode(owner_bytes).into_string();

    let token_account_data = spl_token_account_bytes([0x11; 32], owner_bytes, 1);

    let upstream = FixtureUpstream::new()
        .with_method("getTokenLargestAccounts", move |_params| {
            Ok(serde_json::json!({
                "context": { "slot": 1000 },
                "value": [
                    { "address": "TokAcctResolveTest111111111111111111111111111", "amount": "1", "decimals": 0, "uiAmount": 1.0, "uiAmountString": "1" }
                ]
            }))
        })
        .with_account(
            token_account_addr,
            AccountData {
                data: token_account_data,
                owner: [0x99; 32], // not read by the resolver
                lamports: 2_039_280,
            },
        );

    let got = resolve_owner_for_mint(&upstream, mint).await;
    assert_eq!(got.as_deref(), Some(owner_b58.as_str()));
}

#[tokio::test]
async fn resolve_owner_for_mint_skips_zero_amount_entries() {
    // Burned NFTs leave zero-balance ATAs behind. The resolver should
    // ignore those and pick the first non-zero entry.
    let mint = "MintZeroAmountTest111111111111111111111111111";
    let live_holder_addr = "LiveHolder111111111111111111111111111111111";
    let live_owner = [0x55u8; 32];
    let live_owner_b58 = bs58::encode(live_owner).into_string();

    let live_bytes = spl_token_account_bytes([0x11; 32], live_owner, 1);

    let upstream = FixtureUpstream::new()
        .with_method("getTokenLargestAccounts", |_params| {
            Ok(serde_json::json!({
                "context": { "slot": 1000 },
                "value": [
                    { "address": "DeadZeroAcct111111111111111111111111111111", "amount": "0", "decimals": 0, "uiAmount": 0.0, "uiAmountString": "0" },
                    { "address": "LiveHolder111111111111111111111111111111111", "amount": "1", "decimals": 0, "uiAmount": 1.0, "uiAmountString": "1" }
                ]
            }))
        })
        .with_account(
            live_holder_addr,
            AccountData {
                data: live_bytes,
                owner: [0x99; 32],
                lamports: 2_039_280,
            },
        );

    assert_eq!(
        resolve_owner_for_mint(&upstream, mint).await.as_deref(),
        Some(live_owner_b58.as_str())
    );
}

#[tokio::test]
async fn resolve_owner_for_mint_returns_none_when_no_holders() {
    let upstream = FixtureUpstream::new().with_method("getTokenLargestAccounts", |_| {
        Ok(serde_json::json!({ "context": { "slot": 1 }, "value": [] }))
    });
    assert!(resolve_owner_for_mint(&upstream, "anyMint").await.is_none());
}

#[tokio::test]
async fn resolve_owner_for_mint_returns_none_when_token_account_missing() {
    let upstream = FixtureUpstream::new().with_method("getTokenLargestAccounts", |_| {
        Ok(serde_json::json!({
            "context": { "slot": 1 },
            "value": [{ "address": "MissingTokAcct11111111111111111111111111111", "amount": "1" }]
        }))
    });
    assert!(resolve_owner_for_mint(&upstream, "anyMint").await.is_none());
}

#[tokio::test]
async fn resolve_owner_for_mint_returns_none_when_upstream_missing_method() {
    let upstream = FixtureUpstream::new();
    assert!(resolve_owner_for_mint(&upstream, "anyMint").await.is_none());
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
