//! Versioned WebSocket control plane: handshake, request/response with
//! `req_id`, server pushes, terminal attach with replay cursors, and input
//! grant-gated typing. See `docs/protocol.md`.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::ws::WebSocketUpgrade;
use axum::extract::State;
use axum::http::HeaderMap;
use axum::response::IntoResponse;
use bus::Capability;
use serde_json::{json, Value};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::app::{AppState, DAEMON_VERSION, PROTOCOL_VERSION};
use crate::browser::view::Viewer;

mod admin;
mod binary;
mod board;
pub(crate) use board::{home_snapshot, peer_read};
mod board_import;
mod browser;
mod chat;
mod commands;
mod conversations;
mod dashboard;
mod metrics;
pub(crate) use metrics::home_metrics;
mod decisions;
mod decisions_publish;
mod dispatch;
mod entities;
mod home;
mod later;
mod links;
mod meetings;
mod messaging;
mod origin;
mod owner_actions;
mod owner_auth;
mod owner_card;
#[cfg(test)]
mod panic_tests;
mod peers;
mod permissions;
#[cfg(test)]
mod probe;
mod profiles;
mod project_repo;
mod quiesce;
mod release_install;
mod releases;
mod routines;
mod runtime;
mod session;
mod tasks;
mod terminal;
mod views;
mod workers;
mod writer;

pub(crate) use views::{bot_view, project_view};

const MAX_FRAME_BYTES: usize = 1024 * 1024;

/// How often the server pings a client, so a quiet link still carries traffic
/// both ways.
const PING_INTERVAL: Duration = Duration::from_secs(20);

/// A client that sends nothing at all for this long (no request, ping or
/// pong) is gone: a phone that drove into a tunnel, say. Its connection is
/// closed rather than left to the OS, which can take far longer to notice.
const IDLE_TIMEOUT: Duration = Duration::from_secs(60);

/// How long a closing connection's writer may take to send what is left. A
/// writer stuck on a dead link is stopped instead, so the socket closes.
const WRITER_GRACE: Duration = Duration::from_secs(5);

pub async fn ws_handler(
    State(app): State<Arc<AppState>>,
    headers: HeaderMap,
    peer: Option<axum::extract::ConnectInfo<std::net::SocketAddr>>,
    upgrade: WebSocketUpgrade,
) -> axum::response::Response {
    if !origin::origin_allowed(&app, &headers) {
        return (
            axum::http::StatusCode::FORBIDDEN,
            "origin not allowed".to_string(),
        )
            .into_response();
    }
    upgrade
        .max_frame_size(MAX_FRAME_BYTES)
        .on_upgrade(move |socket| session::handle_socket(app, socket, peer.map(|c| c.0)))
        .into_response()
}

struct Conn {
    app: Arc<AppState>,
    out: mpsc::UnboundedSender<Value>,
    /// bot_id -> forwarding task for live terminal frames.
    attachments: HashMap<String, JoinHandle<()>>,
    /// The bot browser this connection is watching, if any.
    browser_watch: Option<JoinHandle<()>>,
    /// Where that watch sends: `out`, and the writer's newest-wins frame slot.
    viewer: Viewer,
    caps: Vec<Capability>,
    /// None for the owner token; the issuing device otherwise. A ruling made
    /// from a device stays attributable after that device is revoked.
    device_id: Option<String>,
    /// Binary frames (protobuf envelopes) to this client.
    bin: mpsc::UnboundedSender<Vec<u8>>,
    /// The boards this connection watches.
    watch: board::Watch,
    /// The client renders terminal cards and may see and answer them.
    terminal_cards: bool,
    /// How this client may see and run owner actions (H-117 R1).
    owner: owner_actions::Client,
    /// The request being handled, named in the log if its handler panics.
    kind: String,
}

/// Attachments die with the connection, however it ends: on a panic as
/// much as on a close (H-170).
impl Drop for Conn {
    fn drop(&mut self) {
        for (_, task) in self.attachments.drain() {
            task.abort();
        }
        if let Some(task) = self.browser_watch.take() {
            task.abort();
        }
    }
}

/// Returns the authenticated connection's capability grants and issuing
/// device, or None when the handshake fails (an error frame is sent first).
fn handshake(
    app: &Arc<AppState>,
    out: &mpsc::UnboundedSender<Value>,
    text: &str,
) -> Option<(Vec<Capability>, Option<String>, bool)> {
    let Ok(req) = serde_json::from_str::<Value>(text) else {
        return None;
    };
    let req_id = req.get("req_id").cloned().unwrap_or(Value::Null);
    let fail = |code: &str, message: String| {
        let _ = out.send(json!({
            "type": "error", "req_id": req_id.clone(), "code": code, "message": message
        }));
    };
    if req.get("type").and_then(|t| t.as_str()) != Some("hello") {
        fail("invalid_request", "expected hello".to_string());
        return None;
    }
    let version = req
        .get("protocol_version")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    if version != PROTOCOL_VERSION as u64 {
        fail(
            "unsupported_version",
            format!("server speaks protocol {PROTOCOL_VERSION}"),
        );
        return None;
    }
    let token = req.get("token").and_then(|t| t.as_str()).unwrap_or("");

    // The owner token (same machine, mode 0600) grants everything; device
    // tokens carry explicit scoped capabilities and can be revoked.
    // A one-time ticket from the local endpoint is the owner too (H-044 T4):
    // the desktop app by its signature, a CLI command by the owner's card.
    let owner_token = app.secrets.verify_client(token);
    let via_ticket = !owner_token && app.owner.redeem(token);
    if owner_token {
        app.owner.note_client_token();
    }
    // The owner token is readable by every bot of the same user, so it
    // reads and runs the fleet but never rules for the owner (CE-030 N1):
    // `approve` comes only with the app's ticket or a device granted it.
    let (caps, device_id) = if via_ticket {
        (
            vec![Capability::Read, Capability::Control, Capability::Approve],
            None,
        )
    } else if owner_token {
        (vec![Capability::Read, Capability::Control], None)
    } else if let Some(device_id) = app.secrets.device_for_token(token) {
        match app.db.get_device(&device_id) {
            Ok(Some(d)) if d.revoked_at.is_none() => (d.capabilities, Some(device_id)),
            _ => {
                fail("auth_failed", "device credential revoked".to_string());
                return None;
            }
        }
    } else {
        fail("auth_failed", "invalid client token".to_string());
        return None;
    };
    if let Some(id) = &device_id {
        let _ = app.db.touch_device(id);
    }
    let cap_strs: Vec<&str> = caps.iter().map(|c| c.as_str()).collect();
    // Per-surface contract versions (H-020 §1.4). Nothing is served by
    // version yet; a client on another version is logged so the first
    // breaking change has evidence of who still speaks the old one.
    let contracts = bus::contract::versions();
    if let Some(theirs) = req.get("contracts").and_then(Value::as_object) {
        for (surface, version) in theirs {
            if version.as_u64() != contracts.get(surface).map(|v| u64::from(*v)) {
                tracing::info!(%surface, client = %version, "client speaks another contract version");
            }
        }
    }
    let _ = out.send(json!({
        "type": "hello_ok", "req_id": req_id,
        "protocol_version": PROTOCOL_VERSION,
        "server_version": DAEMON_VERSION,
        "capabilities": crate::app::CAPABILITIES,
        "contracts": contracts,
        // Binary frames carry protobuf envelopes for typed surfaces.
        "encodings": bus::contract::ENCODINGS,
        "grants": cap_strs,
        "device_id": device_id,
        // The id peers learn, so a client paired with two daemons can tell
        // which peer row is which daemon.
        "daemon_id": app.db.daemon_id().ok()
    }));
    Some((caps, device_id, via_ticket))
}
