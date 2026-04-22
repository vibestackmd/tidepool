//! Cache store integration tests. Build DasAsset values by hand, put
//! them in the cache, query by every indexed dimension.

use tidepool_rpc::cache::{CacheStore, MemoryCache, SearchFilter};
use tidepool_rpc::das::{
    DasAsset, DasAuthority, DasContent, DasCreator, DasFile, DasGrouping, DasLinks, DasMetadata,
    DasOwnership,
};

fn stub_asset(id: &str, owner: &str) -> DasAsset {
    DasAsset {
        id: id.into(),
        interface: "V1_NFT".into(),
        content: DasContent {
            schema: String::new(),
            json_uri: String::new(),
            metadata: DasMetadata {
                name: String::new(),
                symbol: String::new(),
                description: String::new(),
            },
            links: DasLinks {
                image: None,
                animation_url: None,
            },
            files: Vec::<DasFile>::new(),
        },
        authorities: vec![],
        creators: vec![],
        ownership: DasOwnership {
            frozen: false,
            delegated: false,
            ownership_model: "single".into(),
            owner: owner.into(),
        },
        grouping: vec![],
        mutable: true,
        burnt: false,
        compression: None,
    }
}

#[tokio::test]
async fn put_then_get_asset_roundtrip() {
    let cache = MemoryCache::new();
    let asset = stub_asset("A1", "owner1");
    cache.put_asset(asset.clone()).await.unwrap();
    assert_eq!(cache.get_asset("A1").await.unwrap().unwrap().id, "A1");
    assert!(cache.get_asset("A2").await.unwrap().is_none());
}

#[tokio::test]
async fn get_assets_by_owner_returns_scoped_set() {
    let cache = MemoryCache::new();
    cache.put_asset(stub_asset("A1", "owner1")).await.unwrap();
    cache.put_asset(stub_asset("A2", "owner1")).await.unwrap();
    cache.put_asset(stub_asset("A3", "owner2")).await.unwrap();

    let for_1 = cache.get_assets_by_owner("owner1").await.unwrap();
    assert_eq!(for_1.len(), 2);
    let ids: Vec<_> = for_1.iter().map(|a| a.id.as_str()).collect();
    assert!(ids.contains(&"A1") && ids.contains(&"A2"));

    assert_eq!(cache.get_assets_by_owner("owner2").await.unwrap().len(), 1);
}

#[tokio::test]
async fn put_asset_with_empty_owner_skips_owner_index() {
    // Token Metadata decoder leaves owner blank until resolution;
    // we don't want those polluting owner-indexed queries.
    let cache = MemoryCache::new();
    cache.put_asset(stub_asset("A1", "")).await.unwrap();
    assert_eq!(cache.get_assets_by_owner("").await.unwrap().len(), 0);
}

#[tokio::test]
async fn get_assets_by_authority_and_creator_and_group() {
    let cache = MemoryCache::new();
    let mut asset = stub_asset("A1", "owner1");
    asset.authorities = vec![DasAuthority {
        address: "authA".into(),
        scopes: vec!["full".into()],
    }];
    asset.creators = vec![DasCreator {
        address: "creatorA".into(),
        share: 100,
        verified: true,
    }];
    asset.grouping = vec![DasGrouping {
        group_key: "collection".into(),
        group_value: "coll1".into(),
    }];
    cache.put_asset(asset).await.unwrap();

    assert_eq!(cache.get_assets_by_authority("authA").await.unwrap().len(), 1);
    assert_eq!(
        cache.get_assets_by_creator("creatorA", false).await.unwrap().len(),
        1
    );
    assert_eq!(
        cache.get_assets_by_creator("creatorA", true).await.unwrap().len(),
        1,
        "verified creator queries pass when the creator is verified"
    );
    assert_eq!(
        cache.get_assets_by_group("collection", "coll1").await.unwrap().len(),
        1
    );
}

#[tokio::test]
async fn get_assets_by_creator_only_verified_filters_unverified() {
    let cache = MemoryCache::new();
    let mut asset = stub_asset("A1", "owner1");
    asset.creators = vec![DasCreator {
        address: "creatorA".into(),
        share: 100,
        verified: false, // unverified
    }];
    cache.put_asset(asset).await.unwrap();

    assert_eq!(
        cache.get_assets_by_creator("creatorA", false).await.unwrap().len(),
        1
    );
    assert_eq!(
        cache.get_assets_by_creator("creatorA", true).await.unwrap().len(),
        0,
        "only_verified filters unverified creators out"
    );
}

#[tokio::test]
async fn re_put_asset_removes_stale_index_entries() {
    let cache = MemoryCache::new();
    let mut asset = stub_asset("A1", "owner1");
    asset.authorities = vec![DasAuthority {
        address: "authA".into(),
        scopes: vec!["full".into()],
    }];
    cache.put_asset(asset.clone()).await.unwrap();

    // Change authority and re-put.
    asset.authorities = vec![DasAuthority {
        address: "authB".into(),
        scopes: vec!["full".into()],
    }];
    cache.put_asset(asset).await.unwrap();

    assert_eq!(
        cache.get_assets_by_authority("authA").await.unwrap().len(),
        0,
        "old authority index should be cleared"
    );
    assert_eq!(
        cache.get_assets_by_authority("authB").await.unwrap().len(),
        1
    );
}

#[tokio::test]
async fn search_assets_ands_multiple_filters() {
    let cache = MemoryCache::new();
    let mut a1 = stub_asset("A1", "owner1");
    a1.authorities = vec![DasAuthority {
        address: "authX".into(),
        scopes: vec!["full".into()],
    }];
    let mut a2 = stub_asset("A2", "owner1");
    a2.authorities = vec![DasAuthority {
        address: "authY".into(),
        scopes: vec!["full".into()],
    }];
    let mut a3 = stub_asset("A3", "owner2");
    a3.authorities = vec![DasAuthority {
        address: "authX".into(),
        scopes: vec!["full".into()],
    }];
    cache.put_asset(a1).await.unwrap();
    cache.put_asset(a2).await.unwrap();
    cache.put_asset(a3).await.unwrap();

    // owner1 AND authX → only A1.
    let filter = SearchFilter {
        owner_address: Some("owner1".into()),
        authority_address: Some("authX".into()),
        ..Default::default()
    };
    let results = cache.search_assets(&filter).await.unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].id, "A1");
}

#[tokio::test]
async fn search_assets_filters_by_interface_and_burnt() {
    let cache = MemoryCache::new();
    let mut a1 = stub_asset("A1", "owner1");
    a1.interface = "MplCoreAsset".into();
    a1.burnt = false;
    let mut a2 = stub_asset("A2", "owner1");
    a2.interface = "V1_NFT".into();
    a2.burnt = true;
    cache.put_asset(a1).await.unwrap();
    cache.put_asset(a2).await.unwrap();

    let filter = SearchFilter {
        interface: Some("V1_NFT".into()),
        burnt: Some(true),
        ..Default::default()
    };
    let results = cache.search_assets(&filter).await.unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].id, "A2");
}
