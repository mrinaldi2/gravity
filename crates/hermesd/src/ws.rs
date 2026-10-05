//! Versioned WebSocket control plane: handshake, request/response with
//! `req_id`, server pushes, terminal attach with replay cursors, and input
//! grant-gated typing. See `docs/protocol.md`.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::ws::{Message as WsMessage, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::http::HeaderMap;
use axum::response::IntoResponse;
use bus::Capability;
use futures::StreamExt;
use serde_json::{json, Value};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::app::{AppState, DAEMON_VERSION, PROTOCOL_VERSION};
use crate::browser::view::Viewer;

mod admin;
mod binary;
mod board;
mod board_import;
mod browser;
mod chat;
mod commands;
mod conversations;
mod decisions;
mod decisions_publish;
mod dispatch;
mod entities;
mod links;
mod messaging;
mod peers;
mod permissions;
mod profiles;
mod project_repo;
mod routines;
mod runtime;
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
    upgrade: WebSocketUpgrade,
) -> axum::response::Response {
    if !origin_allowed(&app, &headers) {
        return (
            axum::http::StatusCode::FORBIDDEN,
            "origin not allowed".to_string(),
        )
            .into_response();
    }
    upgrade
        .max_frame_size(MAX_FRAME_BYTES)
        .on_upgrade(move |socket| handle_socket(app, socket))
        .into_response()
}

/// Native clients send no Origin header; browser contexts must match the
/// localhost/tauri defaults or the configured allowlist.
fn origin_allowed(app: &Arc<AppState>, headers: &HeaderMap) -> bool {
    let Some(origin) = headers.get("origin").and_then(|v| v.to_str().ok()) else {
        return true;
    };
    if origin == "null"
        || origin.starts_with("tauri://")
        || origin.starts_with("http://tauri.")
        || origin.starts_with("http://localhost")
        || origin.starts_with("http://127.0.0.1")
    {
        return true;
    }
    app.cfg.allowed_origins.iter().any(|o| o == origin)
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
}

async fn handle_socket(app: Arc<AppState>, socket: WebSocket) {
    let (sink, mut stream) = socket.split();
    let (out_tx, out_rx) = mpsc::unbounded_channel::<Value>();
    let (viewer, frames) = Viewer::new(out_tx.clone());

    // Writer task: everything outbound goes through one sink.
    // Binary frames (protobuf envelopes) have their own queue to the writer.
    let (bin_tx, bin_rx) = mpsc::unbounded_channel::<Vec<u8>>();
    let writer = tokio::spawn(writer::write_out(
        sink,
        out_rx,
        bin_rx,
        frames,
        PING_INTERVAL,
    ));

    // Handshake: first frame must be a valid hello.
    let session = match tokio::time::timeout(IDLE_TIMEOUT, stream.next()).await {
        Ok(Some(Ok(WsMessage::Text(text)))) => {
            handshake(&app, &out_tx, &text).map(|session| (session, shows_permission_cards(&text)))
        }
        _ => None,
    };
    let Some(((caps, device_id), cards)) = session else {
        drop((out_tx, viewer));
        finish(writer).await;
        return;
    };

    // A client that renders permission cards and may answer them is what lets
    // the daemon hold a prompt for the app instead of the terminal.
    let _answerer =
        (cards && caps.contains(&Capability::Control)).then(|| crate::approval::answerer(&app));

    // Forward server pushes to this client.
    let push_tx = out_tx.clone();
    let mut push_rx = app.events.subscribe_push();
    let push_app = app.clone();
    let push_task = tokio::spawn(async move {
        loop {
            let push = match push_rx.recv().await {
                Ok(push) => push,
                // Falling behind a burst must not end the feed for good: the
                // client keeps the pushes that follow and resyncs the rest on
                // its own. Ending the loop here left a connection silently
                // stale until it reconnected.
                Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                    tracing::warn!(skipped, "client push feed lagged; some pushes were dropped");
                    continue;
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            };
            // `BotUpdated` carries a database row, whose `state` and
            // `unread_count` are placeholders the supervisor normally overlays.
            // Serialising it raw would tell clients every changed bot is
            // stopped with nothing unread, so it is rendered the same way a
            // `list_bots` reply is.
            let value = match &push {
                crate::events::Push::BotUpdated { bot } => {
                    Ok(json!({ "type": "bot_updated", "bot": bot_view(&push_app, bot) }))
                }
                // Archiving tombstones the name so it can be reused; clients
                // should see the name the project actually had.
                crate::events::Push::ProjectUpdated { project } => Ok(
                    json!({ "type": "project_updated", "project": project_view(&push_app, project) }),
                ),
                other => serde_json::to_value(other),
            };
            if let Ok(v) = value {
                if push_tx.send(v).is_err() {
                    break;
                }
            }
        }
    });

    let mut conn = Conn {
        app: app.clone(),
        out: out_tx.clone(),
        attachments: HashMap::new(),
        browser_watch: None,
        viewer,
        caps,
        device_id,
        bin: bin_tx,
        watch: board::Watch::default(),
    };

    // Any frame counts as a sign of life, a ping or pong as much as a request.
    // Pongs to the client's pings go out with the next read or write, both of
    // which flush.
    loop {
        let frame = match tokio::time::timeout(IDLE_TIMEOUT, stream.next()).await {
            Ok(Some(Ok(frame))) => frame,
            Ok(_) => break,
            Err(_) => {
                tracing::info!(
                    idle_secs = IDLE_TIMEOUT.as_secs(),
                    "client went silent; closing its connection"
                );
                break;
            }
        };
        match frame {
            WsMessage::Text(text) => {
                if text.len() > MAX_FRAME_BYTES {
                    continue;
                }
                let Ok(req) = serde_json::from_str::<Value>(&text) else {
                    continue;
                };
                conn.dispatch(&req);
            }
            WsMessage::Binary(bytes) => {
                if bytes.len() <= MAX_FRAME_BYTES {
                    conn.binary_frame(&bytes);
                }
            }
            WsMessage::Close(_) => break,
            _ => {}
        }
    }

    // Cleanup: attachments die with the connection.
    for (_, task) in conn.attachments.drain() {
        task.abort();
    }
    if let Some(task) = conn.browser_watch.take() {
        task.abort();
    }
    push_task.abort();
    drop(out_tx);
    drop(conn);
    finish(writer).await;
}

/// Lets the writer send what is left, unless the link is too slow to.
async fn finish(mut writer: JoinHandle<()>) {
    if tokio::time::timeout(WRITER_GRACE, &mut writer)
        .await
        .is_err()
    {
        writer.abort();
    }
}

/// Whether the client's hello says it shows permission cards.
fn shows_permission_cards(hello: &str) -> bool {
    serde_json::from_str::<Value>(hello).is_ok_and(|hello| {
        hello["features"]
            .as_array()
            .is_some_and(|features| features.iter().any(|f| f == "permission_cards"))
    })
}

/// Returns the authenticated connection's capability grants and issuing
/// device, or None when the handshake fails (an error frame is sent first).
fn handshake(
    app: &Arc<AppState>,
    out: &mpsc::UnboundedSender<Value>,
    text: &str,
) -> Option<(Vec<Capability>, Option<String>)> {
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
    let (caps, device_id) = if app.secrets.verify_client(token) {
        (
            vec![Capability::Read, Capability::Control, Capability::Approve],
            None,
        )
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
    Some((caps, device_id))
}
