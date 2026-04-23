//! Cache store integration tests. Build DasAsset values by hand, put
//! them in the cache, query by every indexed dimension.

use tidepool_rpc::cache::{CacheStore, MemoryCache, SearchFilter};
use tidepool_rpc::das::{
    DasAsset, DasAuthority, DasContent, DasCreator, DasFile, DasGrouping, DasLinks, DasMetadata,
    DasOwnership, MasterEditionRecord, PrintEditionRecord,
};

fn stub_asset(id: &str, owner: &str) -> DasAsset {
    DasAsset {
        id: id.into(),
        interface: "V1_NFT".into(),
        content: DasContent {
            metadata: DasMetadata::default(),
            links: DasLinks::default(),
            files: Vec::<DasFile>::new(),
            ..Default::default()
        },
        ownership: DasOwnership {
            ownership_model: "single".into(),
            owner: owner.into(),
            ..Default::default()
        },
        mutable: true,
        ..Default::default()
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

// ─── edition index (getNftEditions) ─────────────────────────────────

#[tokio::test]
async fn master_edition_round_trip() {
    let cache = MemoryCache::new();
    let record = MasterEditionRecord {
        master_mint: "MASTER_MINT".into(),
        master_edition_pda: "MASTER_EDITION_PDA".into(),
        supply: 3,
        max_supply: Some(10),
    };
    cache.put_master_edition(record.clone()).await.unwrap();
    let got = cache.get_master_edition("MASTER_MINT").await.unwrap();
    assert_eq!(got, Some(record));
    let miss = cache.get_master_edition("UNKNOWN").await.unwrap();
    assert!(miss.is_none());
}

#[tokio::test]
async fn print_editions_sort_by_edition_num() {
    let cache = MemoryCache::new();
    let parent = "MASTER_EDITION_PDA";
    // Insert out of order.
    for (mint, num) in [("MINT3", 3u64), ("MINT1", 1), ("MINT2", 2)] {
        cache
            .put_print_edition(PrintEditionRecord {
                print_mint: mint.into(),
                print_edition_pda: format!("{mint}_EDITION_PDA"),
                parent_master_edition_pda: parent.into(),
                edition_num: num,
            })
            .await
            .unwrap();
    }
    let listed = cache.list_print_editions(parent).await.unwrap();
    assert_eq!(listed.len(), 3);
    assert_eq!(listed[0].edition_num, 1);
    assert_eq!(listed[1].edition_num, 2);
    assert_eq!(listed[2].edition_num, 3);
}

#[tokio::test]
async fn print_edition_repeated_put_overwrites_same_mint() {
    let cache = MemoryCache::new();
    let parent = "MASTER_EDITION_PDA";
    cache
        .put_print_edition(PrintEditionRecord {
            print_mint: "MINT1".into(),
            print_edition_pda: "OLD_PDA".into(),
            parent_master_edition_pda: parent.into(),
            edition_num: 1,
        })
        .await
        .unwrap();
    cache
        .put_print_edition(PrintEditionRecord {
            print_mint: "MINT1".into(),
            print_edition_pda: "NEW_PDA".into(),
            parent_master_edition_pda: parent.into(),
            edition_num: 1,
        })
        .await
        .unwrap();
    let listed = cache.list_print_editions(parent).await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].print_edition_pda, "NEW_PDA");
}

#[tokio::test]
async fn print_editions_are_scoped_by_parent() {
    let cache = MemoryCache::new();
    cache
        .put_print_edition(PrintEditionRecord {
            print_mint: "A1".into(),
            print_edition_pda: "A1_PDA".into(),
            parent_master_edition_pda: "PARENT_A".into(),
            edition_num: 1,
        })
        .await
        .unwrap();
    cache
        .put_print_edition(PrintEditionRecord {
            print_mint: "B1".into(),
            print_edition_pda: "B1_PDA".into(),
            parent_master_edition_pda: "PARENT_B".into(),
            edition_num: 1,
        })
        .await
        .unwrap();
    let a = cache.list_print_editions("PARENT_A").await.unwrap();
    let b = cache.list_print_editions("PARENT_B").await.unwrap();
    assert_eq!(a.len(), 1);
    assert_eq!(b.len(), 1);
    assert_eq!(a[0].print_mint, "A1");
    assert_eq!(b[0].print_mint, "B1");
}
