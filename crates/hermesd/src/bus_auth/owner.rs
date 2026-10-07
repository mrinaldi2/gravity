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
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::app::AppState;
use crate::approval::OwnerAnswer;

/// How long a ticket waits for its hello.
const TICKET_TTL: Duration = Duration::from_secs(60);
/// JSON-RPC error: the caller may not act as the owner.
pub const NOT_THE_OWNER: i64 = -32002;
/// What the CLI prints when the owner command doesn't run (UX-014).
pub const DENIED: &str = "You denied this command in The Hermes. It did not run.";
pub const EXPIRED: &str = "No answer in The Hermes, so the command did not run.";
pub const NO_APP: &str =
    "Open The Hermes on this computer to allow this command, then run it again.";

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

/// Who a ticket went to, as the logs name it (H-165): the process, never
/// the ticket.
#[derive(Debug, Clone)]
pub struct Grantee {
    /// `app` (the code check passed) or `cli` (the owner allowed it).
    pub via: &'static str,
    pub pid: u32,
    pub exe: Option<PathBuf>,
}

impl Grantee {
    fn of(via: &'static str, pid: u32) -> Self {
        Self {
            via,
            pid,
            exe: super::origin::exe_path(pid),
        }
    }

    fn exe(&self) -> String {
        self.exe
            .as_ref()
            .map_or_else(|| "unknown".to_string(), |exe| exe.display().to_string())
    }
}

/// How the owner has connected since the daemon started, for `/health`
/// (H-165): proof the app took the ticket path, without reading secrets.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize)]
pub struct OwnerCounts {
    pub owner_tickets_granted: u64,
    pub owner_tickets_redeemed: u64,
    pub client_token_fallbacks: u64,
}

/// The owner's tickets and the check that grants them.
pub struct Owner {
    check: RwLock<Arc<dyn CodeCheck>>,
    tickets: Mutex<HashMap<String, (Instant, Grantee)>>,
    granted: AtomicU64,
    redeemed: AtomicU64,
    client_token: AtomicU64,
}

impl Owner {
    pub fn new(home: &Path) -> Self {
        remove_old_pin(home);
        Self {
            check: RwLock::new(Arc::new(CompiledApp)),
            tickets: Mutex::new(HashMap::new()),
            granted: AtomicU64::new(0),
            redeemed: AtomicU64::new(0),
            client_token: AtomicU64::new(0),
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

    /// A ticket for the owner, as the test process; only `handle` hands one
    /// out, after its check. Public for tests, which stand in for the app in
    /// phase 2.
    #[doc(hidden)]
    pub fn mint(&self) -> String {
        self.grant(Grantee::of("test", std::process::id()))
    }

    fn grant(&self, to: Grantee) -> String {
        let ticket = hex::encode(rand::random::<[u8; 32]>());
        tracing::info!(
            via = to.via,
            pid = to.pid,
            exe = %to.exe(),
            team = super::app_identity::TEAM_ID,
            "owner ticket granted"
        );
        let mut tickets = self.tickets.lock().unwrap_or_else(|e| e.into_inner());
        tickets.retain(|_, (issued, _)| issued.elapsed() < TICKET_TTL);
        tickets.insert(ticket.clone(), (Instant::now(), to));
        self.granted.fetch_add(1, Ordering::Relaxed);
        ticket
    }

    /// Spends a ticket: true once, within its minute.
    pub fn redeem(&self, ticket: &str) -> bool {
        let spent = {
            let mut tickets = self.tickets.lock().unwrap_or_else(|e| e.into_inner());
            tickets.remove(ticket)
        };
        let Some((issued, to)) = spent.filter(|(issued, _)| issued.elapsed() < TICKET_TTL) else {
            return false;
        };
        tracing::info!(
            via = to.via,
            pid = to.pid,
            exe = %to.exe(),
            age_ms = issued.elapsed().as_millis() as u64,
            "owner ticket redeemed"
        );
        self.redeemed.fetch_add(1, Ordering::Relaxed);
        true
    }

    /// The owner connected with `client.token`, not a ticket: the app's
    /// fallback when the daemon refused it one, or an old CLI.
    pub fn note_client_token(&self) {
        tracing::info!("owner connected with client.token, not a ticket");
        self.client_token.fetch_add(1, Ordering::Relaxed);
    }

    pub fn counts(&self) -> OwnerCounts {
        OwnerCounts {
            owner_tickets_granted: self.granted.load(Ordering::Relaxed),
            owner_tickets_redeemed: self.redeemed.load(Ordering::Relaxed),
            client_token_fallbacks: self.client_token.load(Ordering::Relaxed),
        }
    }
}

fn refused(id: &Value, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": NOT_THE_OWNER, "message": message } })
}

fn ticket(id: &Value, owner: &Owner, to: Grantee) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": { "ticket": owner.grant(to) } })
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
        return Some(refused(
            &id,
            if method == "hermes/owner_request" {
                "Commands that act as the owner can't run from a bot's session."
            } else {
                "a bot can't act as the owner"
            },
        ));
    }
    if method == "hermes/owner_ticket" {
        return Some(if app.owner.is_owner_app(&peer) {
            ticket(&id, &app.owner, Grantee::of("app", peer.pid))
        } else {
            refused(&id, "not the owner's app")
        });
    }
    let params = &request["params"];
    let command = params["command"].as_str().unwrap_or("a command");
    let origin = super::origin::of(app, command, peer.pid, params["cwd"].as_str());
    Some(match crate::approval::ask_owner(app, &origin).await {
        OwnerAnswer::Allowed => ticket(&id, &app.owner, Grantee::of("cli", peer.pid)),
        OwnerAnswer::Denied => refused(&id, DENIED),
        OwnerAnswer::Expired => refused(&id, EXPIRED),
        OwnerAnswer::NoApp => refused(&id, NO_APP),
    })
}
