//! Boards homed on a peer (B9, H-020 §1.3), from the side that mirrors them:
//! fetched when the home's link comes up, kept current by its `board_event`
//! relay, and republished on this daemon's own feed, so a client watching
//! the project here (a phone attached to imac) sees the board move.

use std::sync::Arc;

use bus::contract::board as c;
use bus::ProjectLink;
use serde_json::{json, Value};

use super::board_home::kind_from;
use super::{board_ids, PeerError};
use crate::app::AppState;
use crate::board::feed::{Change, ChangeKind};

/// Fetches the boards this peer holds for projects linked through it.
pub(super) async fn link_up(app: Arc<AppState>, peer_id: String) {
    for link in app.db.links_through(&peer_id).unwrap_or_default() {
        if !has_own_board(&app, &link.project_id) {
            fetch(&app, &link).await;
        }
    }
}

/// A project just linked: the side holding the board has its peers mirror
/// it (whichever side asked, the link may only now exist there too); the
/// other side fetches it.
pub(super) fn linked(app: &Arc<AppState>, project_id: &str, peer_id: &str) {
    if has_own_board(app, project_id) {
        app.board.writer().publish(Change {
            project_id,
            kind: ChangeKind::SettingsChanged,
            item_id: "",
            card: None,
            from_column: None,
        });
    } else if let Ok(Some(link)) = app.db.project_link(project_id, peer_id) {
        refetch(app, &link);
    }
}

fn has_own_board(app: &AppState, project_id: &str) -> bool {
    app.db.board_settings(project_id).ok().flatten().is_some()
}

/// Mirrors the board the link's peer holds for the project, and tells the
/// project's watchers here to refetch it.
pub async fn fetch(app: &AppState, link: &ProjectLink) {
    // Ids in a frame are the sender's own (see `frames`).
    let frame = json!({ "type": "board_snapshot", "project_id": link.project_id });
    match app.peers.request(&link.peer_id, frame).await {
        Ok(reply) => {
            let Ok(mut board) = serde_json::from_value::<c::BoardSnapshot>(reply["board"].clone())
            else {
                tracing::warn!(peer_id = %link.peer_id, "unreadable board snapshot from peer");
                return;
            };
            board_ids::snapshot(&mut board, &link.project_id, |id| {
                board_ids::from_home(&app.db, &link.peer_id, &link.project_id, id)
            });
            let mut feed = app.board.writer();
            app.board_mirror.set(&link.project_id, &link.peer_id, board);
            // Kept across restarts (ARCH-R59 a): an empty mirror must not
            // read as "no board" and let tasks go out without a card.
            if let Err(e) = app.db.set_board_home(&link.project_id, &link.peer_id) {
                tracing::warn!(error = %e, "can't record the board's home");
            }
            feed.publish(Change {
                project_id: &link.project_id,
                kind: ChangeKind::SettingsChanged,
                item_id: "",
                card: None,
                from_column: None,
            });
        }
        // The peer holds no board for it (any more).
        Err(PeerError::Refused { code, .. }) if code == "no_board" || code == "not_linked" => {
            app.board_mirror.forget(&link.project_id, &link.peer_id);
            let _ = app.db.forget_board_home(&link.project_id, &link.peer_id);
        }
        // Offline or failing: keep what was last seen.
        Err(e) => tracing::debug!(peer_id = %link.peer_id, error = %e, "board fetch failed"),
    }
}

/// `board_event {project_id, kind, item_id, card?, from_column?}` from a
/// board's home: applied to the mirror, then published here.
pub(super) fn receive_event(app: &Arc<AppState>, peer_id: &str, frame: &Value) {
    let Some(remote) = frame.get("project_id").and_then(Value::as_str) else {
        return;
    };
    let Ok(Some(link)) = app.db.project_link_by_remote(peer_id, remote) else {
        return;
    };
    if has_own_board(app, &link.project_id) {
        return;
    }
    let kind = frame
        .get("kind")
        .and_then(Value::as_str)
        .and_then(kind_from);
    let mirrored = app.board_mirror.home_peer(&link.project_id).as_deref() == Some(peer_id);
    match kind {
        Some(kind @ (ChangeKind::ItemUpserted | ChangeKind::ItemMoved)) if mirrored => {
            apply_card(app, &link, kind, frame);
        }
        Some(ChangeKind::ItemRemoved) if mirrored => {
            let item_id = frame["item_id"].as_str().unwrap_or_default();
            let mut feed = app.board.writer();
            if app.board_mirror.remove_card(&link.project_id, item_id) {
                feed.publish(Change {
                    project_id: &link.project_id,
                    kind: ChangeKind::ItemRemoved,
                    item_id,
                    card: None,
                    from_column: None,
                });
            }
        }
        // Columns, settings, a resync, or a board not mirrored yet (just
        // started there): fetch it whole.
        _ => refetch(app, &link),
    }
}

/// Fetches in the background: events and links are applied off the async
/// runtime's workers, on its blocking pool.
fn refetch(app: &Arc<AppState>, link: &ProjectLink) {
    let Ok(runtime) = tokio::runtime::Handle::try_current() else {
        return;
    };
    let (app, link) = (app.clone(), link.clone());
    runtime.spawn(async move { fetch(&app, &link).await });
}

fn apply_card(app: &Arc<AppState>, link: &ProjectLink, kind: ChangeKind, frame: &Value) {
    let Ok(mut card) = serde_json::from_value::<c::ItemCard>(frame["card"].clone()) else {
        // Pushed without its card (a failed read on the home): refetch.
        refetch(app, link);
        return;
    };
    board_ids::card(&mut card, |id| {
        board_ids::from_home(&app.db, &link.peer_id, &link.project_id, id)
    });
    let Ok(model) = crate::board::model::ItemCard::try_from(card.clone()) else {
        return;
    };
    let item_id = model.id.clone();
    let mut feed = app.board.writer();
    if app.board_mirror.apply_card(&link.project_id, card) {
        feed.publish(Change {
            project_id: &link.project_id,
            kind,
            item_id: &item_id,
            card: Some(model),
            from_column: frame["from_column"].as_str().map(str::to_string),
        });
    }
}

/// Rewrites the bot ids in a tool result the board's home sent back.
pub fn ids_from_home(app: &AppState, link: &ProjectLink, value: &mut Value) {
    board_ids::json(value, &|id| {
        board_ids::from_home(&app.db, &link.peer_id, &link.project_id, id)
    });
}

/// The name of the peer holding a mirrored board, for messages.
pub fn home_name(app: &AppState, peer_id: &str) -> String {
    app.db
        .get_peer(peer_id)
        .ok()
        .flatten()
        .map_or_else(|| "its home computer".to_string(), |p| p.name)
}
