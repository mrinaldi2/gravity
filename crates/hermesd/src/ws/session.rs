//! One client's connection: the writer, the handshake, the push feed and
//! the read loop. However the connection's task ends, a panic included, the
//! socket closes, so the client sees a disconnect and reconnects instead of
//! waiting on a socket nobody serves (H-170).

use std::collections::HashMap;
use std::sync::Arc;

use axum::extract::ws::{Message as WsMessage, WebSocket};
use bus::Capability;
use futures::stream::SplitStream;
use futures::{FutureExt, StreamExt};
use serde_json::{json, Value};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use super::{
    board, handshake, owner_actions, writer, Conn, IDLE_TIMEOUT, MAX_FRAME_BYTES, PING_INTERVAL,
    WRITER_GRACE,
};
use crate::app::AppState;
use crate::browser::view::Viewer;
use crate::ws::{bot_view, project_view};

/// A task stopped when its handle is dropped, so an unwinding connection
/// leaves no feed running behind it.
struct AbortOnDrop(JoinHandle<()>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

pub(super) async fn handle_socket(
    app: Arc<AppState>,
    socket: WebSocket,
    peer: Option<std::net::SocketAddr>,
) {
    let (sink, stream) = socket.split();
    let (out_tx, out_rx) = mpsc::unbounded_channel::<Value>();
    let (viewer, frames) = Viewer::new(out_tx.clone());

    // Writer task: everything outbound goes through one sink.
    // Binary frames (protobuf envelopes) have their own queue to the writer.
    let (bin_tx, bin_rx) = mpsc::unbounded_channel::<Vec<u8>>();
    let mut writer = tokio::spawn(writer::write_out(
        sink,
        out_rx,
        bin_rx,
        frames,
        PING_INTERVAL,
    ));

    // However the session ends, a panic included, its senders are dropped
    // and the writer closes the socket: a client must see a disconnect and
    // reconnect, never wait on a socket nobody serves (H-170).
    let session = serve(app, stream, out_tx, viewer, bin_tx, peer, &mut writer);
    let served = std::panic::AssertUnwindSafe(session).catch_unwind().await;
    if let Err(panic) = served {
        tracing::error!(
            panic = %crate::contain::panic_text(panic.as_ref()),
            "a connection's task panicked; closing its socket"
        );
    }
    finish(writer).await;
}

/// The handshake, then requests until the client goes, the link goes quiet,
/// or the writer or push feed stops.
async fn serve(
    app: Arc<AppState>,
    mut stream: SplitStream<WebSocket>,
    out_tx: mpsc::UnboundedSender<Value>,
    viewer: Viewer,
    bin_tx: mpsc::UnboundedSender<Vec<u8>>,
    peer: Option<std::net::SocketAddr>,
    writer: &mut JoinHandle<()>,
) {
    // Handshake: first frame must be a valid hello.
    let session = match tokio::time::timeout(IDLE_TIMEOUT, stream.next()).await {
        Ok(Some(Ok(WsMessage::Text(text)))) => {
            handshake(&app, &out_tx, &text).map(|session| (session, hello_features(&text)))
        }
        _ => None,
    };
    let Some(((caps, device_id, via_ticket), features)) = session else {
        return;
    };

    // A client that renders permission cards and may answer them is what lets
    // the daemon hold a prompt for the app instead of the terminal.
    let cards = features.iter().any(|f| f == "permission_cards");
    // Terminal cards (an owner command waiting on the owner) go only to a
    // client that says it renders them: an older one would show them as a
    // bot's prompt and could grant owner power without saying so (H-044 T4).
    let terminal_cards = cards && features.iter().any(|f| f == TERMINAL_CARD);
    // Only a client that could allow a terminal card (it holds approve) makes
    // a terminal command wait for the app; a control-only one would leave it
    // waiting on a card nobody there may answer (ARCH-R36).
    let terminal_answerer = terminal_cards && caps.contains(&Capability::Approve);
    let _answerer = (cards && caps.contains(&Capability::Control))
        .then(|| crate::approval::answerer(&app, terminal_answerer));

    // Owner actions go only to a client that renders them (H-117 R1).
    let owner = owner_actions::Client::new(&features, &caps, device_id.is_some(), via_ticket, peer);
    // Forward server pushes to this client.
    let push_tx = out_tx.clone();
    let mut push_rx = app.events.subscribe_push();
    let push_app = app.clone();
    let mut push_task = AbortOnDrop(tokio::spawn(async move {
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
            if (!terminal_cards && is_terminal_card(&push)) || !owner.sees(&push) {
                continue;
            }
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
    }));

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
        terminal_cards,
        owner,
        kind: String::new(),
    };

    // Any frame counts as a sign of life, a ping or pong as much as a request.
    // Pongs to the client's pings go out with the next read or write, both of
    // which flush.
    loop {
        let next = tokio::select! {
            next = tokio::time::timeout(IDLE_TIMEOUT, stream.next()) => next,
            // A writer that stopped can send nothing more: replies would be
            // lost while the client waits. A push feed that stopped leaves
            // the client stale. Either way the client must reconnect.
            _ = &mut *writer => {
                tracing::warn!("a connection's writer stopped; closing its socket");
                break;
            }
            _ = &mut push_task.0 => {
                tracing::warn!("a connection's push feed stopped; closing its socket");
                break;
            }
        };
        let frame = match next {
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
                #[cfg(test)]
                if req["type"] == super::probe::PANIC_CONNECTION {
                    panic!("a test connection task panicked");
                }
                conn.handle(&req);
            }
            WsMessage::Binary(bytes) => {
                if bytes.len() <= MAX_FRAME_BYTES {
                    conn.handle_binary(&bytes);
                }
            }
            WsMessage::Close(_) => break,
            _ => {}
        }
    }

    // Dropping `conn` and `push_task` stops the attachments and the feed,
    // as an unwind would: then only in-flight answers hold the writer open.
}

/// Lets the writer send what is left and close the socket, unless the link
/// is too slow to; then the writer is stopped and the socket dropped.
async fn finish(mut writer: JoinHandle<()>) {
    if writer.is_finished() {
        return;
    }
    if tokio::time::timeout(WRITER_GRACE, &mut writer)
        .await
        .is_err()
    {
        writer.abort();
    }
}

/// The hello feature a client sends when it renders terminal cards.
const TERMINAL_CARD: &str = "terminal_card";

/// The optional behaviours the client's hello says it supports.
fn hello_features(hello: &str) -> Vec<String> {
    serde_json::from_str::<Value>(hello)
        .ok()
        .and_then(|hello| {
            hello["features"].as_array().map(|features| {
                features
                    .iter()
                    .filter_map(|f| f.as_str().map(str::to_string))
                    .collect()
            })
        })
        .unwrap_or_default()
}

/// Whether a push is about a terminal card.
fn is_terminal_card(push: &crate::events::Push) -> bool {
    use crate::events::Push;
    match push {
        Push::PermissionRequest { request } => request.bot_id == crate::approval::TERMINAL,
        Push::PermissionResolved { bot_id, .. } => bot_id == crate::approval::TERMINAL,
        _ => false,
    }
}
