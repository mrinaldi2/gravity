//! The owner without a token (H-044 §3, T4). `client.token` grants
//! everything, release rulings included, and every bot can read the file.
//! Instead:
//! - the desktop app asks the local endpoint for a one-time ticket
//!   (`hermes/owner_ticket`), granted when the process asking is the app
//!   compiled into hermesd (`app_identity`: its Developer ID signature on
//!   macOS, its Program Files folder on Windows);
//! - a CLI owner command asks for one too (`hermes/owner_request`), and the
//!   owner allows or denies it on a card in the app.
//!
//! A ticket is good for one WebSocket hello within a minute. Bots, being in
//! a session, get neither. `client.token` keeps working until phase 2 (T6).

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::app::AppState;

/// How long a ticket waits for its hello.
const TICKET_TTL: Duration = Duration::from_secs(60);
/// JSON-RPC error: the caller may not act as the owner.
pub const NOT_THE_OWNER: i64 = -32002;

/// The process on the other end of a local connection.
#[derive(Debug, Clone, Copy, Default)]
pub struct Peer {
    pub pid: u32,
    /// macOS's `audit_token_t` for the peer, which names the exact process.
    pub audit_token: Option<[u32; 8]>,
}

/// Whether a process is the owner's desktop app.
pub trait CodeCheck: Send + Sync {
    fn is_owner_app(&self, peer: &Peer) -> bool;
}

/// The app as `app_identity` describes it, checked by the OS.
struct CompiledApp;

impl CodeCheck for CompiledApp {
    fn is_owner_app(&self, peer: &Peer) -> bool {
        super::owner_os::is_owner_app(peer)
    }
}

/// T4 kept the app's identity in this file, which any bot could rewrite.
fn remove_old_pin(home: &Path) {
    let old = home.join("secrets").join("owner-app.json");
    if let Err(e) = std::fs::remove_file(&old) {
        if e.kind() != std::io::ErrorKind::NotFound {
            tracing::warn!("can't remove {}: {e}", old.display());
        }
    }
}

/// The owner's tickets and the check that grants them.
pub struct Owner {
    check: RwLock<Arc<dyn CodeCheck>>,
    tickets: Mutex<HashMap<String, Instant>>,
}

impl Owner {
    pub fn new(home: &Path) -> Self {
        remove_old_pin(home);
        Self {
            check: RwLock::new(Arc::new(CompiledApp)),
            tickets: Mutex::new(HashMap::new()),
        }
    }

    /// Replaces the code check; tests use a double (real signing can't run in CI).
    pub fn set_check(&self, check: Arc<dyn CodeCheck>) {
        *self.check.write().unwrap_or_else(|e| e.into_inner()) = check;
    }

    pub fn is_owner_app(&self, peer: &Peer) -> bool {
        let check = self.check.read().unwrap_or_else(|e| e.into_inner()).clone();
        check.is_owner_app(peer)
    }

    /// A ticket for the owner; only `handle` hands one out, after its check.
    /// Public for tests, which stand in for the app in phase 2.
    #[doc(hidden)]
    pub fn mint(&self) -> String {
        let ticket = hex::encode(rand::random::<[u8; 32]>());
        let mut tickets = self.tickets.lock().unwrap_or_else(|e| e.into_inner());
        tickets.retain(|_, issued| issued.elapsed() < TICKET_TTL);
        tickets.insert(ticket.clone(), Instant::now());
        ticket
    }

    /// Spends a ticket: true once, within its minute.
    pub fn redeem(&self, ticket: &str) -> bool {
        let mut tickets = self.tickets.lock().unwrap_or_else(|e| e.into_inner());
        tickets
            .remove(ticket)
            .is_some_and(|issued| issued.elapsed() < TICKET_TTL)
    }
}

fn refused(id: &Value, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": NOT_THE_OWNER, "message": message } })
}

fn ticket(id: &Value, owner: &Owner) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": { "ticket": owner.mint() } })
}

/// Answers the owner's methods on the local endpoint; `None` for any other
/// method. `in_session` is true when the caller runs in a bot's session.
pub async fn handle(
    app: &AppState,
    peer: Option<Peer>,
    in_session: bool,
    request: &Value,
) -> Option<Value> {
    let method = request.get("method").and_then(Value::as_str)?;
    if method != "hermes/owner_ticket" && method != "hermes/owner_request" {
        return None;
    }
    let id = request.get("id").cloned().unwrap_or(Value::Null);
    let Some(peer) = peer else {
        return Some(refused(&id, "can't tell which process is asking"));
    };
    if in_session {
        return Some(refused(&id, "a bot can't act as the owner"));
    }
    if method == "hermes/owner_ticket" {
        return Some(if app.owner.is_owner_app(&peer) {
            ticket(&id, &app.owner)
        } else {
            refused(&id, "not the owner's app")
        });
    }
    let command = request["params"]["command"].as_str().unwrap_or("a command");
    Some(
        match crate::approval::ask_owner(app, command, peer.pid).await {
            Some(true) => ticket(&id, &app.owner),
            Some(false) => refused(&id, "the owner didn't allow it"),
            None => refused(&id, "open The Hermes to allow this command"),
        },
    )
}
