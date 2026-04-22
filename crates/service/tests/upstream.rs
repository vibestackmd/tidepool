//! FixtureUpstream integration tests. The network-backed impl gets
//! its own tests alongside the server crate where its plumbing lives.

use serde_json::json;
use tidepool_rpc::upstream::{AccountData, FixtureUpstream, UpstreamClient, UpstreamError};

#[tokio::test]
async fn get_account_returns_registered_accounts() {
    let upstream = FixtureUpstream::new().with_account(
        "AAA",
        AccountData {
            data: vec![1, 2, 3],
            owner: [9; 32],
            lamports: 1_000_000,
        },
    );
    let a = upstream.get_account("AAA").await.unwrap().unwrap();
    assert_eq!(a.lamports, 1_000_000);
    assert_eq!(a.owner[0], 9);
    assert_eq!(a.data, vec![1, 2, 3]);
}

#[tokio::test]
async fn get_account_returns_none_for_unknown() {
    let upstream = FixtureUpstream::new();
    assert!(upstream.get_account("nope").await.unwrap().is_none());
}

#[tokio::test]
async fn rpc_call_routes_registered_methods() {
    let upstream = FixtureUpstream::new().with_method("getSlot", |_params| Ok(json!(42)));
    let raw = upstream.rpc_call("getSlot", json!([])).await.unwrap();
    assert_eq!(raw, b"42");
}

#[tokio::test]
async fn rpc_call_receives_params() {
    let upstream = FixtureUpstream::new().with_method("echo", |params| Ok(params.clone()));
    let raw = upstream
        .rpc_call("echo", json!([1, 2, 3]))
        .await
        .unwrap();
    assert_eq!(raw, b"[1,2,3]");
}

#[tokio::test]
async fn rpc_call_errors_on_unstubbed_method() {
    let upstream = FixtureUpstream::new();
    let err = upstream
        .rpc_call("getBalance", json!(["addr"]))
        .await
        .unwrap_err();
    assert!(matches!(err, UpstreamError::MethodNotStubbed { .. }));
    assert!(format!("{err}").contains("getBalance"));
}
