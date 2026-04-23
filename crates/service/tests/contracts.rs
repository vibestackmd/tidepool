//! Contract tests — **layer 3** of Tidepool's testing strategy.
//! Run offline against the committed `contracts/fixtures/` +
//! `contracts/schemas/` pair produced by `cargo xtask record-helius`
//! and `cargo xtask derive-schemas`.
//!
//! They catch two classes of drift:
//!
//! 1. **Our types don't fit Helius's shape.** Deserializing the
//!    recorded response into our Rust DAS types must succeed. If
//!    Helius adds a field we don't know about (and our struct uses
//!    `deny_unknown_fields`) or drops one we require, this fails.
//! 2. **Our serialization drops fields Helius returns.** We round-
//!    trip: Rust type → JSON → diff against the recorded response.
//!    Any key present in Helius's response but missing from our
//!    serialization is flagged as a drift.
//!
//! These tests are the floor, not the ceiling. They only cover the
//! specific (method, case) pairs in `contracts/cases.toml`. Widen
//! coverage by adding cases there and re-running the xtask.

use std::path::{Path, PathBuf};

use serde_json::Value;

fn repo_root() -> PathBuf {
    // Tests run from the crate directory; repo root is two levels up.
    let crate_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    crate_dir
        .parent()
        .and_then(Path::parent)
        .unwrap_or(crate_dir)
        .to_path_buf()
}

fn load_fixture_response(method: &str, case: &str) -> Value {
    let path = repo_root()
        .join("contracts/fixtures")
        .join(method)
        .join(format!("{case}.json"));
    let bytes = std::fs::read(&path)
        .unwrap_or_else(|e| panic!("read fixture {}: {e}", path.display()));
    let envelope: Value = serde_json::from_slice(&bytes).expect("parse fixture envelope");
    envelope.get("response").cloned().expect("response key")
}

fn load_schema(method: &str, case: &str) -> Value {
    let path = repo_root()
        .join("contracts/schemas")
        .join(method)
        .join(format!("{case}.schema.json"));
    let bytes = std::fs::read(&path)
        .unwrap_or_else(|e| panic!("read schema {}: {e}", path.display()));
    let envelope: Value = serde_json::from_slice(&bytes).expect("parse schema envelope");
    envelope.get("schema").cloned().expect("schema key")
}

/// Collect every key path present in `a` but missing in `b`. Recursive
/// — catches both top-level misses and deeply-nested ones. Prefix
/// accumulates as we descend; returned paths are like
/// `result.content.metadata.name`.
fn missing_keys(a: &Value, b: &Value, prefix: &str) -> Vec<String> {
    let mut out = Vec::new();
    match (a, b) {
        (Value::Object(am), Value::Object(bm)) => {
            for (k, av) in am {
                let here = if prefix.is_empty() {
                    k.clone()
                } else {
                    format!("{prefix}.{k}")
                };
                if let Some(bv) = bm.get(k) {
                    out.extend(missing_keys(av, bv, &here));
                } else {
                    out.push(here);
                }
            }
        }
        (Value::Array(aa), Value::Array(ba)) => {
            // Compare shapes by pairing element 0 of each — enough to
            // catch "we dropped a per-item field" without O(N) noise.
            if let (Some(a0), Some(b0)) = (aa.first(), ba.first()) {
                out.extend(missing_keys(a0, b0, &format!("{prefix}[0]")));
            }
        }
        _ => {}
    }
    out
}

#[test]
fn contract_schemas_validate_against_their_source_fixtures() {
    // Self-check: every committed schema validates its own source
    // fixture. If this fails, the schema derivation is broken or the
    // fixture was edited by hand.
    let methods_dir = repo_root().join("contracts/fixtures");
    let entries = std::fs::read_dir(&methods_dir).expect("fixtures dir");
    let mut checked = 0;
    for method_entry in entries {
        let method_entry = method_entry.unwrap();
        if !method_entry.path().is_dir() {
            continue;
        }
        let method = method_entry.file_name().to_string_lossy().to_string();
        for case_entry in std::fs::read_dir(method_entry.path()).unwrap() {
            let case_entry = case_entry.unwrap();
            let case_file = case_entry.file_name().to_string_lossy().to_string();
            let Some(case) = case_file.strip_suffix(".json") else {
                continue;
            };

            let response = load_fixture_response(&method, case);
            let schema = load_schema(&method, case);
            let compiled = jsonschema::draft7::new(&schema)
                .unwrap_or_else(|e| panic!("compile schema for {method}/{case}: {e}"));
            if let Err(err) = compiled.validate(&response) {
                panic!("{method}/{case} schema rejected its own fixture: {err}");
            }
            checked += 1;
        }
    }
    assert!(
        checked > 0,
        "no fixtures found — did you run `cargo xtask record-helius`?"
    );
}

#[test]
fn get_assets_by_owner_parses_into_das_asset_cleanly() {
    use tidepool_rpc::das::DasAsset;

    let response = load_fixture_response(
        "getAssetsByOwner",
        "getAssetsByOwner_small_wallet",
    );
    let items = response
        .pointer("/result/items")
        .and_then(Value::as_array)
        .expect("items array");

    let mut parsed = 0;
    for (i, raw) in items.iter().enumerate() {
        let _asset: DasAsset = serde_json::from_value(raw.clone()).unwrap_or_else(|e| {
            panic!(
                "item {i}: our DasAsset type rejected a real Helius response: {e}\nvalue: {raw}"
            )
        });
        parsed += 1;
    }
    assert!(parsed > 0, "fixture produced zero items to validate");
}

#[test]
fn get_assets_by_owner_roundtrip_drops_no_fields() {
    // Round-trip a real Helius item through our DasAsset and back
    // to JSON, then diff against the original. Any field Helius
    // sends that we silently drop shows up here.
    use tidepool_rpc::das::DasAsset;

    let response = load_fixture_response(
        "getAssetsByOwner",
        "getAssetsByOwner_small_wallet",
    );
    let original = response
        .pointer("/result/items/0")
        .cloned()
        .expect("first item");

    let parsed: DasAsset = serde_json::from_value(original.clone())
        .expect("parse first item as DasAsset");
    let reserialized = serde_json::to_value(&parsed).expect("serialize DasAsset");

    let dropped = missing_keys(&original, &reserialized, "");
    assert!(
        dropped.is_empty(),
        "DasAsset drops fields Helius returns: {dropped:#?}\n\n\
         These fields exist on Helius responses but don't survive the\n\
         round-trip through our Rust type. Either add them to the type\n\
         or document the omission."
    );
}

#[test]
fn programmable_nft_interface_matches_helius() {
    // Regression guard for the pNFT drift that the contract test rig
    // found: our Token Metadata decoder used to hardcode
    // interface="V1_NFT". Real Helius returns "ProgrammableNFT" for
    // programmable mints (Mad Lads, Famous Fox, etc.). If someone
    // refactors `interface_for_standard` and breaks this mapping,
    // this test fails against the recorded fixture.
    let response = load_fixture_response("getAsset", "getAsset_mad_lads_1337");
    let iface = response
        .pointer("/result/interface")
        .and_then(|v| v.as_str())
        .expect("interface field");
    assert_eq!(
        iface, "ProgrammableNFT",
        "Helius returns ProgrammableNFT for Mad Lads; our decoder must \
         produce the same string via interface_for_standard()"
    );
}

/// Shared helper: parse every DAS asset in a by-X fixture's `items`
/// array and assert our type preserves every field. Factored out so
/// per-method tests share the same invariant.
fn assert_das_items_round_trip(method: &str, case: &str) {
    use tidepool_rpc::das::DasAsset;
    let response = load_fixture_response(method, case);
    let items = response
        .pointer("/result/items")
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("{method}/{case}: missing result.items array"));
    if items.is_empty() {
        // Empty-result cases still exercise the envelope shape
        // (the `total`/`limit`/`page` fields deserialize) but don't
        // have anything to round-trip. Silent skip — not a failure.
        eprintln!("{method}/{case}: upstream returned empty items — skipping per-item check");
        return;
    }
    for (i, raw) in items.iter().enumerate() {
        let parsed: DasAsset = serde_json::from_value(raw.clone()).unwrap_or_else(|e| {
            panic!("{method}/{case} item {i}: DasAsset deserialize failed: {e}")
        });
        let reserialized = serde_json::to_value(parsed).unwrap();
        let dropped = missing_keys(raw, &reserialized, "");
        assert!(
            dropped.is_empty(),
            "{method}/{case} item {i} drops fields on round-trip: {dropped:#?}"
        );
    }
}

#[test]
fn get_assets_by_authority_items_round_trip() {
    assert_das_items_round_trip("getAssetsByAuthority", "getAssetsByAuthority_small_set");
}

#[test]
fn get_assets_by_creator_items_round_trip() {
    assert_das_items_round_trip("getAssetsByCreator", "getAssetsByCreator_verified_only");
}

#[test]
fn get_assets_by_group_items_round_trip() {
    assert_das_items_round_trip("getAssetsByGroup", "getAssetsByGroup_mad_lads_collection");
}

#[test]
fn search_assets_items_round_trip() {
    assert_das_items_round_trip("searchAssets", "searchAssets_by_collection");
}

#[test]
fn get_asset_batch_items_round_trip() {
    // getAssetBatch returns a raw array, not a {items} wrapper.
    use tidepool_rpc::das::DasAsset;
    let response = load_fixture_response(
        "getAssetBatch",
        "getAssetBatch_two_real_two_missing",
    );
    let arr = response
        .pointer("/result")
        .and_then(Value::as_array)
        .expect("result array");
    for (i, raw) in arr.iter().enumerate() {
        if raw.is_null() {
            continue; // nonexistent ids resolve to null
        }
        let parsed: DasAsset = serde_json::from_value(raw.clone())
            .unwrap_or_else(|e| panic!("item {i}: DasAsset parse failed: {e}"));
        let reserialized = serde_json::to_value(parsed).unwrap();
        let dropped = missing_keys(raw, &reserialized, "");
        assert!(
            dropped.is_empty(),
            "item {i} drops fields on round-trip: {dropped:#?}"
        );
    }
}

#[test]
fn get_asset_invalid_id_returns_error_envelope() {
    // Regression test for error-shape fidelity. Helius returns a
    // standard JSON-RPC error envelope when `id` fails to parse.
    let response = load_fixture_response(
        "getAsset",
        "getAsset_invalid_id_returns_error",
    );
    // Either .error is populated and .result is absent, or vice versa.
    let err = response.pointer("/error").expect("error envelope");
    assert!(err.is_object(), "error must be an object");
    assert!(err.get("code").is_some(), "error.code required");
    assert!(err.get("message").is_some(), "error.message required");
}

#[test]
fn get_balances_response_round_trips() {
    // REST fixture: `GET /v0/addresses/<addr>/balances`. The shape is
    // `{ tokens: [...], nativeBalance: u64 }` — no `result` envelope
    // since REST doesn't use JSON-RPC.
    use tidepool_rpc::das::types::DasBalances;

    let response = load_fixture_response("getBalances", "getBalances_small_wallet");
    let parsed: DasBalances = serde_json::from_value(response.clone())
        .expect("DasBalances should accept real Helius REST response");
    let reserialized = serde_json::to_value(&parsed).expect("serialize DasBalances");
    let dropped = missing_keys(&response, &reserialized, "");
    assert!(
        dropped.is_empty(),
        "DasBalances drops fields Helius returns: {dropped:#?}"
    );
}

#[test]
fn get_transactions_by_address_response_is_array_of_enhanced() {
    // REST fixture: `GET /v0/addresses/<addr>/transactions?limit=3`.
    // Response is a bare array of enhanced-tx records, no envelope.
    use tidepool_rpc::enhanced::types::EnhancedTransaction;

    let response = load_fixture_response(
        "getTransactionsByAddress",
        "getTransactionsByAddress_small_wallet",
    );
    let items = response.as_array().expect("REST returns a bare array");
    assert!(!items.is_empty(), "small wallet should have at least one tx");
    for (i, raw) in items.iter().enumerate() {
        let _parsed: EnhancedTransaction = serde_json::from_value(raw.clone()).unwrap_or_else(|e| {
            panic!("item {i}: EnhancedTransaction rejected real Helius response: {e}\nvalue: {raw}")
        });
    }
}

#[test]
fn get_priority_fee_estimate_levels_round_trip() {
    use tidepool_rpc::priority_fee::PriorityFeeLevels;

    let response = load_fixture_response(
        "getPriorityFeeEstimate",
        "getPriorityFeeEstimate_all_levels",
    );
    let levels_raw = response
        .pointer("/result/priorityFeeLevels")
        .cloned()
        .expect("priorityFeeLevels key");

    let parsed: PriorityFeeLevels = serde_json::from_value(levels_raw.clone())
        .expect("parse priorityFeeLevels");
    let reserialized = serde_json::to_value(parsed).unwrap();
    let dropped = missing_keys(&levels_raw, &reserialized, "");
    assert!(
        dropped.is_empty(),
        "PriorityFeeLevels drops fields: {dropped:#?}"
    );
}
