//! The state shared by every request of one server cycle (rebuilt at each hot restart).

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use arc_swap::ArcSwap;
use serde::Serialize;
use tokio::sync::{broadcast, watch, Notify};

use crate::auth::{Auth, User};
use crate::config::ServerConfig;
use crate::guard::Logins;
use crate::sso::Sso;

pub type Shared = Arc<AppState>;

/// Who receives a server event.
#[derive(Clone, Debug, PartialEq)]
pub enum Audience {
    All,
    Admins,
    /// Users who can edit (admin, operator).
    Editors,
    User(String),
}

impl Audience {
    pub fn includes(&self, user: &User) -> bool {
        match self {
            Audience::All => true,
            Audience::Admins => user.is_admin(),
            Audience::Editors => user.can_edit(),
            Audience::User(id) => *id == user.id,
        }
    }
}

/// One event, serialized once for every socket.
#[derive(Clone, Debug)]
pub struct Event {
    pub to: Audience,
    pub text: Arc<str>,
}

/// The broadcast of server events to the WebSocket clients.
pub struct Events {
    tx: broadcast::Sender<Event>,
}

impl Events {
    pub fn new(capacity: usize) -> Events {
        Events { tx: broadcast::channel(capacity).0 }
    }
    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.tx.subscribe()
    }
    /// Sends any JSON message (with its own `type`) to an audience.
    pub fn send<T: Serialize>(&self, to: Audience, msg: &T) {
        match serde_json::to_string(msg) {
            Ok(text) => {
                let _ = self.tx.send(Event { to, text: text.into() });
            }
            Err(e) => tracing::error!("cannot serialize an event: {e}"),
        }
    }
}

pub struct AppState {
    pub data_dir: PathBuf,
    pub auth: Arc<Auth>,
    pub config: ArcSwap<ServerConfig>,
    pub events: Events,
    /// The address the plain HTTP listener got.
    pub bind: SocketAddr,
    /// The HTTPS address when HTTPS is on and listening.
    pub https_bind: Option<SocketAddr>,
    pub started_at: Instant,
    pub started_at_ms: i64,
    /// Notified to rebuild the state and the listeners without leaving the process.
    pub restart: Notify,
    /// Set to true when this cycle ends (sockets close).
    pub closing: watch::Sender<bool>,
    /// Set to true to stop the server for good.
    pub stop: watch::Sender<bool>,
    /// Long-lived parts that survive hot restarts.
    pub logins: Arc<Logins>,
    pub sso: Arc<Sso>,
    pub supervised: bool,
    // ---- State of the network features (inventory, scanner…) goes here. ----
}

impl AppState {
    /// Tells every client that a list changed (`{"type":"changed","what":…}`): they fetch it again.
    pub fn changed(&self, what: &str) {
        self.changed_for(Audience::All, what);
    }

    pub fn changed_for(&self, to: Audience, what: &str) {
        self.events.send(to, &serde_json::json!({ "type": "changed", "what": what }));
    }

    /// An error message shown as a toast.
    pub fn error_to(&self, to: Audience, message: &str) {
        self.events.send(to, &serde_json::json!({ "type": "error", "message": message }));
    }

    pub fn uptime_secs(&self) -> u64 {
        crate::health::uptime_secs()
    }

    /// A receiver that resolves when this cycle ends.
    pub fn closing(&self) -> watch::Receiver<bool> {
        self.closing.subscribe()
    }
}
