//! WebSocket reverse-proxy integration tests. Spin up a fake upstream
//! WS server, point Tidepool's WS proxy at it, connect a client, and
//! assert frames flow both directions.
//!
//! We don't test subscription semantics here — that's Surfpool's job
//! upstream. We test that the proxy is transparent.

#![allow(clippy::useless_conversion, clippy::implicit_clone)]

use std::time::Duration;

use axum::{
    extract::{
        ws::{Message as AxumMsg, WebSocket},
        WebSocketUpgrade,
    },
    response::Response,
    routing::get,
    Router,
};
use futures_util::{SinkExt, StreamExt};
use tokio::net::TcpListener;
use tokio_tungstenite::{connect_async, tungstenite::Message as TgMsg};

/// Spawn a fake upstream WS server. The behavior is supplied as a
/// closure that owns the connection — most tests just echo, but
/// some need to send unsolicited frames or close early.
async fn spawn_upstream<F, Fut>(behavior: F) -> String
where
    F: Fn(WebSocket) -> Fut + Send + Sync + Clone + 'static,
    Fut: std::future::Future<Output = ()> + Send + 'static,
{
    let behavior_state = behavior;
    let app: Router = Router::new().route(
        "/",
        get(move |ws: WebSocketUpgrade| {
            let b = behavior_state.clone();
            async move { upgrade(ws, b) }
        }),
    );

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("ws://{addr}/")
}

fn upgrade<F, Fut>(ws: WebSocketUpgrade, behavior: F) -> Response
where
    F: FnOnce(WebSocket) -> Fut + Send + 'static,
    Fut: std::future::Future<Output = ()> + Send + 'static,
{
    ws.on_upgrade(behavior)
}

/// Spawn Tidepool's WS proxy pointing at the given upstream URL.
/// Returns the local port to connect to.
async fn spawn_proxy(upstream_ws_url: String) -> u16 {
    let probe = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = probe.local_addr().unwrap().port();
    drop(probe);
    tokio::spawn(async move {
        tidepool_server::run_ws(port, upstream_ws_url, Duration::from_secs(2))
            .await
            .unwrap();
    });
    tokio::time::sleep(Duration::from_millis(120)).await;
    port
}

#[tokio::test]
async fn proxy_forwards_client_to_upstream_and_back() {
    // Echo server: every text frame received is sent back unchanged.
    let upstream = spawn_upstream(|mut socket: WebSocket| async move {
        while let Some(Ok(msg)) = socket.next().await {
            if let AxumMsg::Text(s) = msg {
                if socket.send(AxumMsg::Text(s)).await.is_err() {
                    break;
                }
            }
        }
    })
    .await;

    let port = spawn_proxy(upstream).await;
    let (mut client, _) = connect_async(format!("ws://127.0.0.1:{port}/"))
        .await
        .expect("client should connect");

    // Send a JSON-RPC subscribe-shaped frame.
    let frame =
        r#"{"jsonrpc":"2.0","id":1,"method":"signatureSubscribe","params":["abc","finalized"]}"#;
    client
        .send(TgMsg::Text(frame.to_string().into()))
        .await
        .unwrap();

    let echoed = tokio::time::timeout(Duration::from_secs(2), client.next())
        .await
        .expect("response within 2s")
        .expect("stream not closed")
        .expect("ok msg");
    let TgMsg::Text(echoed) = echoed else {
        panic!("expected text, got {echoed:?}");
    };
    assert_eq!(echoed.to_string(), frame);
}

#[tokio::test]
async fn proxy_delivers_unsolicited_upstream_frames_to_client() {
    // Upstream sends a notification without prompting — simulates
    // signatureNotification fired after a subscribe ack.
    let upstream = spawn_upstream(|mut socket: WebSocket| async move {
        // Wait for one client frame, then push two frames.
        let _ = socket.next().await;
        let _ = socket
            .send(AxumMsg::Text(
                r#"{"jsonrpc":"2.0","result":42,"id":1}"#.to_string().into(),
            ))
            .await;
        let _ = socket
            .send(AxumMsg::Text(
                r#"{"jsonrpc":"2.0","method":"signatureNotification","params":{"subscription":42,"result":{}}}"#.to_string().into(),
            ))
            .await;
        // Keep the connection open so the client can read both.
        tokio::time::sleep(Duration::from_secs(2)).await;
    })
    .await;

    let port = spawn_proxy(upstream).await;
    let (mut client, _) = connect_async(format!("ws://127.0.0.1:{port}/"))
        .await
        .expect("client should connect");

    client
        .send(TgMsg::Text("subscribe".to_string().into()))
        .await
        .unwrap();

    // Read both forwarded frames.
    for expected_id in ["42", "signatureNotification"] {
        let msg = tokio::time::timeout(Duration::from_secs(2), client.next())
            .await
            .expect("response within 2s")
            .expect("stream not closed")
            .expect("ok msg");
        let TgMsg::Text(text) = msg else {
            panic!("expected text, got {msg:?}");
        };
        assert!(
            text.to_string().contains(expected_id),
            "expected frame containing {expected_id:?}, got {text}"
        );
    }
}

#[tokio::test]
async fn proxy_closes_client_when_upstream_unreachable() {
    // Point at a port nobody's listening on.
    let port = spawn_proxy("ws://127.0.0.1:1/".to_string()).await;
    let result = connect_async(format!("ws://127.0.0.1:{port}/")).await;

    // Either the WS handshake itself fails (proxy refused the upgrade
    // path), or we connect and get an immediate close. Both are
    // acceptable signals that the upstream-unreachable path
    // gracefully degrades.
    match result {
        Err(_) => {} // handshake failed — fine
        Ok((mut client, _)) => {
            let next = tokio::time::timeout(Duration::from_secs(2), client.next()).await;
            match next {
                Ok(Some(Ok(TgMsg::Close(_))) | None) => {} // closed — fine
                other => panic!("expected close after bad upstream, got {other:?}"),
            }
        }
    }
}
