//! The owner without a token (H-044 §3, T4). `client.token` grants
//! everything, release rulings included, and every bot can read the file.
//! Instead:
//! - the desktop app asks the local endpoint for a one-time ticket
//!   (`hermes/owner_ticket`), granted when the process asking is the app
//!   pinned at `service install` (its code signature on macOS, its install
//!   folder on Windows);
//! - a CLI owner command asks for one too (`hermes/owner_request`), and the
//!   owner allows or denies it on a card in the app.
//!
//! A ticket is good for one WebSocket hello within a minute. Bots, being in
//! a session, get neither. `client.token` keeps working until phase 2 (T6).

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
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

/// What `service install` recorded about the app it ran from.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Pin {
    /// macOS: the app bundle's designated requirement, e.g. its cdhash.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requirement: Option<String>,
    /// Windows: the folder the app is installed in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app_dir: Option<String>,
}

fn pin_path(home: &Path) -> std::path::PathBuf {
    home.join("secrets").join("owner-app.json")
}

/// Records the app the bundled daemon at `sidecar` ships in, at `service
/// install`. Nothing is pinned for a binary outside an app (a dev build).
pub fn pin_app(home: &Path, sidecar: &Path) -> anyhow::Result<Option<Pin>> {
    let Some(pin) = super::owner_os::pin_for(sidecar)? else {
        return Ok(None);
    };
    let path = pin_path(home);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(&path, serde_json::to_vec_pretty(&pin)?)?;
    Ok(Some(pin))
}

/// The app pinned in the home, checked against a peer by the OS. Read on
/// every check: `service install` pins after the new daemon has started.
struct PinnedApp(std::path::PathBuf);

impl CodeCheck for PinnedApp {
    fn is_owner_app(&self, peer: &Peer) -> bool {
        std::fs::read(pin_path(&self.0))
            .ok()
            .and_then(|raw| serde_json::from_slice::<Pin>(&raw).ok())
            .is_some_and(|pin| super::owner_os::is_pinned_app(&pin, peer))
    }
}

/// The owner's tickets and the check that grants them.
pub struct Owner {
    check: RwLock<Arc<dyn CodeCheck>>,
    tickets: Mutex<HashMap<String, Instant>>,
}

impl Owner {
    pub fn new(home: &Path) -> Self {
        Self {
            check: RwLock::new(Arc::new(PinnedApp(home.to_path_buf()))),
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

    fn mint(&self) -> String {
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
