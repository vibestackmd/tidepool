//! WebSocket reverse proxy to Surfpool's native WS endpoint.
//!
//! As of Surfpool v1.1+ the upstream natively implements every
//! subscription method Tidepool used to polyfill (`signatureSubscribe`,
//! `accountSubscribe`, `logsSubscribe` with `mentions` filter,
//! `programSubscribe`, `slotSubscribe`, plus the corresponding
//! `*Unsubscribe`). We don't add value by re-implementing them via
//! HTTP polling — we just hide the fact that Surfpool's WS listens on
//! a different port (default 8900).
//!
//! Per-client behavior: accept the upgrade, dial upstream, run two
//! pumps (`client → upstream`, `upstream → client`), close cleanly
//! when either side closes.
//!
//! No reconnection — if upstream dies mid-session, the client sees
//! the close and reconnects itself. Solana RPC clients are built for
//! this.
//!
//! No interception or rewriting. Future per-method intercepts (e.g.
//! a Tidepool-specific `tidepool_*Subscribe`) would slot in here as a
//! peek-before-forward step.
//!
//! Replaces the polling polyfill that lived here through v0.1.x.

use std::time::Duration;

use axum::{
    extract::{
        ws::{Message, WebSocket},
        State, WebSocketUpgrade,
    },
    response::Response,
    routing::get,
    Router,
};
use futures_util::{SinkExt, StreamExt};
use tokio::net::TcpListener;
use tokio_tungstenite::{connect_async, tungstenite::Message as TgMessage};
use tracing::{info, warn};

#[derive(Clone)]
pub struct WsState {
    pub upstream_ws_url: String,
    /// Connect timeout for the initial upstream WS handshake. Not the
    /// session lifetime — once connected, the pumps run until either
    /// side closes.
    pub connect_timeout: Duration,
}

/// Spawn the WS server on `port`. Forwards every connection to
/// `upstream_ws_url`. Returns when the listener exits.
pub async fn run_ws(
    port: u16,
    upstream_ws_url: String,
    connect_timeout: Duration,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let state = WsState {
        upstream_ws_url,
        connect_timeout,
    };
    let app = Router::new().route("/", get(ws_upgrade)).with_state(state);
    let addr = std::net::SocketAddr::from(([0, 0, 0, 0], port));
    let listener = TcpListener::bind(&addr).await?;
    info!("tidepool WS proxy listening on ws://{addr}");
    axum::serve(listener, app).await?;
    Ok(())
}

async fn ws_upgrade(ws: WebSocketUpgrade, State(state): State<WsState>) -> Response {
    ws.on_upgrade(move |socket| proxy_connection(socket, state))
}

async fn proxy_connection(client_socket: WebSocket, state: WsState) {
    let upstream_url = state.upstream_ws_url.clone();

    // Dial upstream. If we can't reach it, close the client immediately.
    let upstream =
        match tokio::time::timeout(state.connect_timeout, connect_async(&upstream_url)).await {
            Ok(Ok((ws_stream, _resp))) => ws_stream,
            Ok(Err(e)) => {
                warn!(err = %e, upstream = %upstream_url, "upstream WS connect failed");
                let _ = close_client(client_socket).await;
                return;
            }
            Err(_) => {
                warn!(upstream = %upstream_url, "upstream WS connect timed out");
                let _ = close_client(client_socket).await;
                return;
            }
        };

    let (mut client_sink, mut client_stream) = client_socket.split();
    let (mut upstream_sink, mut upstream_stream) = upstream.split();

    // Pump A: client → upstream.
    let pump_a = async move {
        while let Some(Ok(msg)) = client_stream.next().await {
            let Some(out) = axum_to_tungstenite(msg) else {
                continue;
            };
            let was_close = matches!(out, TgMessage::Close(_));
            if upstream_sink.send(out).await.is_err() || was_close {
                break;
            }
        }
    };

    // Pump B: upstream → client.
    let pump_b = async move {
        while let Some(Ok(msg)) = upstream_stream.next().await {
            let Some(out) = tungstenite_to_axum(msg) else {
                continue;
            };
            let was_close = matches!(out, Message::Close(_));
            if client_sink.send(out).await.is_err() || was_close {
                break;
            }
        }
    };

    // Run both pumps; exit when either completes.
    tokio::select! {
        () = pump_a => {}
        () = pump_b => {}
    }
}

async fn close_client(mut socket: WebSocket) -> Result<(), axum::Error> {
    socket.send(Message::Close(None)).await
}

// axum's `Utf8Bytes` and tungstenite's `Utf8Bytes` are different
// concrete types (axum uses its own type alias; tungstenite re-exports
// from the `utf-8` crate). There's no zero-copy bridge between them,
// so each frame is reallocated through `&str` / `&[u8]`. `to_string()`
// / `to_vec()` on derefs is the readable way to do this; the
// `implicit_clone` lint is the cost of the type fork between the two
// WS libraries.
#[allow(clippy::implicit_clone)]
fn axum_to_tungstenite(msg: Message) -> Option<TgMessage> {
    match msg {
        Message::Text(s) => Some(TgMessage::Text(s.to_string())),
        Message::Binary(b) => Some(TgMessage::Binary(b.to_vec())),
        Message::Close(Some(c)) => Some(TgMessage::Close(Some(
            tokio_tungstenite::tungstenite::protocol::CloseFrame {
                code: c.code.into(),
                reason: c.reason.to_string().into(),
            },
        ))),
        Message::Close(None) => Some(TgMessage::Close(None)),
        // Ping / Pong: axum's WS layer handles ping-pong transparently;
        // we don't need to forward them. Forwarding ping would force
        // the upstream's pong back to us instead of axum's; cleaner to
        // let each hop manage liveness with its own peer.
        Message::Ping(_) | Message::Pong(_) => None,
    }
}

#[allow(clippy::implicit_clone)]
fn tungstenite_to_axum(msg: TgMessage) -> Option<Message> {
    match msg {
        TgMessage::Text(s) => Some(Message::Text(s.to_string().into())),
        TgMessage::Binary(b) => Some(Message::Binary(b.to_vec().into())),
        TgMessage::Close(Some(c)) => Some(Message::Close(Some(axum::extract::ws::CloseFrame {
            code: c.code.into(),
            reason: c.reason.to_string().into(),
        }))),
        TgMessage::Close(None) => Some(Message::Close(None)),
        TgMessage::Ping(_) | TgMessage::Pong(_) | TgMessage::Frame(_) => None,
    }
}
