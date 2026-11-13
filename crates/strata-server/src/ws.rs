//! Real time: the WebSocket `/ws`.

use std::time::Duration;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::sync::broadcast::error::RecvError;

use crate::api::token_from;
use crate::auth::User;
use crate::state::{AppState, Shared};

/// The first message of a socket: everything the client needs to draw.
pub fn hello(s: &AppState, user: &User) -> Value {
    let cfg = s.config.load();
    json!({
        "type": "hello",
        "user": user.public(),
        "serverName": cfg.name,
        "serverTime": crate::paths::now_ms(),
        "timezone": cfg.timezone_name(),
        "scans": s.net.snapshot_scans(),
    })
}

/// `GET /ws` (session required).
pub async fn handler(State(s): State<Shared>, headers: HeaderMap, ws: WebSocketUpgrade) -> Response {
    let token = token_from(&headers).unwrap_or_default();
    let Some(user) = s.auth.session_user(&token) else {
        return (StatusCode::UNAUTHORIZED, "login required").into_response();
    };
    ws.on_upgrade(move |socket| run(s, socket, user, token))
}

async fn run(s: Shared, socket: WebSocket, user: User, token: String) {
    let (mut tx, mut rx) = socket.split();
    // Subscribe before the hello so nothing is missed in between.
    let mut events = s.events.subscribe();
    let mut ended = s.auth.ended.subscribe();
    let mut changed = s.auth.changed.subscribe();
    let mut closing = s.closing();
    let mut check = tokio::time::interval(Duration::from_secs(30));
    check.tick().await;

    if tx.send(Message::Text(hello(&s, &user).to_string().into())).await.is_err() {
        return;
    }
    if *closing.borrow() {
        let _ = tx.send(Message::Close(None)).await;
        return;
    }
    loop {
        tokio::select! {
            ev = events.recv() => match ev {
                Ok(e) => {
                    if e.to.includes(&user) && tx.send(Message::Text(e.text.to_string().into())).await.is_err() {
                        break;
                    }
                }
                // Too slow: close, the client reconnects and gets everything anew.
                Err(RecvError::Lagged(_)) | Err(RecvError::Closed) => break,
            },
            t = ended.recv() => match t {
                Ok(t) if t == token => {
                    let _ = tx.send(Message::Text(json!({ "type": "sessionEnded" }).to_string().into())).await;
                    break;
                }
                Ok(_) | Err(RecvError::Lagged(_)) => {}
                Err(RecvError::Closed) => break,
            },
            id = changed.recv() => match id {
                Ok(id) if id == user.id => break,
                Ok(_) => {}
                Err(RecvError::Lagged(_)) => {
                    // Some change may have been ours: check the session again.
                    if s.auth.session_user(&token).map(|u| u.role) != Some(user.role) {
                        break;
                    }
                }
                Err(RecvError::Closed) => break,
            },
            _ = closing.changed() => break,
            _ = check.tick() => {
                if s.auth.session_user(&token).is_none() {
                    break;
                }
            }
            m = rx.next() => match m {
                Some(Ok(Message::Close(_))) | Some(Err(_)) | None => break,
                Some(Ok(_)) => {}
            },
        }
    }
    let _ = tx.send(Message::Close(None)).await;
}
