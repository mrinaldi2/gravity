//! The board's home serving its linked peers (B9, H-020 §1.3). A peer's bot
//! calls a board tool through `board_call`, which runs here as that bot's
//! stand-in, so roles and guards apply unchanged and the peer itself has no
//! authority. A peer mirrors the board with `board_snapshot`, and every
//! change the board's feed publishes goes to each linked peer as a
//! `board_event`.

use std::sync::Arc;

use bus::contract::board as c;
use bus::{Peer, ProjectLink};
use serde_json::{json, Value};
use tokio::sync::broadcast::error::RecvError;

use super::{board_ids, refuse};
use crate::app::AppState;
use crate::board::feed::{BoardChange, ChangeKind};

/// The project of this daemon that `frame.project_id`, the peer's own, is
/// linked to, with a board whose home is here.
fn home_link(app: &AppState, peer: &Peer, frame: &Value) -> anyhow::Result<ProjectLink> {
    let remote = frame
        .get("project_id")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("'project_id' is required"))?;
    let link = app
        .db
        .project_link_by_remote(&peer.id, remote)?
        .ok_or_else(|| refuse("not_linked", "that project isn't linked with this computer"))?;
    let home = app
        .db
        .board_settings(&link.project_id)?
        .is_some_and(|s| s.home_daemon_id == app.db.daemon_id().unwrap_or_default());
    if !home {
        return Err(refuse("no_board", "this computer doesn't hold that board"));
    }
    Ok(link)
}

/// `board_call {project_id, bot_id, tool, args}`: a board tool, run as the
/// stand-in for the peer's bot in the linked project.
pub(super) fn serve_call(app: &Arc<AppState>, peer: &Peer, frame: &Value) -> anyhow::Result<Value> {
    let link = home_link(app, peer, frame)?;
    let field = |name: &str| {
        frame
            .get(name)
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("'{name}' is required"))
    };
    let (bot_id, tool) = (field("bot_id")?, field("tool")?);
    // A pause stops this computer's projects: never for another computer's
    // install. Newer peers pause themselves and ask here only for the gate
    // (H-166).
    if tool == "install_quiesce" {
        return Err(refuse(
            "forbidden",
            "the board's home doesn't pause itself for another computer's install; update \
             The Hermes on the installing computer",
        ));
    }
    let stand_in = app
        .db
        .linked_bot(&peer.id, bot_id, &link.project_id)?
        .ok_or_else(|| {
            refuse(
                "forbidden",
                "you aren't on this project's team on the board's home; ask the owner to link you",
            )
        })?;
    let args = frame.get("args").cloned().unwrap_or_else(|| json!({}));
    let mut result = match crate::mcp::board_call_as(app, &stand_in.id, tool, &args) {
        Ok(mut result) => {
            if tool == "install_release" {
                published_builds(&mut result)?;
            }
            result
        }
        // A stale write goes back as data, so the item in it gets the
        // peer's ids below like any result (H-113).
        Err(error) => error.downcast::<crate::mcp::Conflict>()?.to_result(),
    };
    board_ids::json(&mut result, &|id| board_ids::to_peer(&app.db, &peer.id, id));
    Ok(result)
}

/// A tester on another computer can't read the home's disk: it installs
/// from each build's HTTPS `url` (ARCH-R28 d), never the local `artifact`.
fn published_builds(result: &mut Value) -> anyhow::Result<()> {
    for build in result["builds"].as_array_mut().into_iter().flatten() {
        if build["url"].as_str().is_none_or(str::is_empty) {
            anyhow::bail!("build not published over HTTPS; DevOps runs release_publish");
        }
        if let Some(fields) = build.as_object_mut() {
            fields.remove("artifact");
        }
    }
    Ok(())
}

/// `board_read {project_id, request}`: an item's details, history or move
/// check for the owner's drawer on the peer (ARCH-R28 c), in its ids.
pub(super) fn serve_read(app: &Arc<AppState>, peer: &Peer, frame: &Value) -> anyhow::Result<Value> {
    let link = home_link(app, peer, frame)?;
    let request: c::BoardRequest = serde_json::from_value(frame["request"].clone())?;
    let response = crate::ws::peer_read(app, &link.project_id, request)?;
    let mut response = serde_json::to_value(response)?;
    board_ids::json(&mut response, &|id| {
        board_ids::to_peer(&app.db, &peer.id, id)
    });
    Ok(json!({ "response": response }))
}

/// `metrics_get {project_id, days}`: the Flow widget's numbers from this
/// home's history (B11). They name items, not bots, so nothing is rewritten.
pub(super) fn serve_metrics(
    app: &Arc<AppState>,
    peer: &Peer,
    frame: &Value,
) -> anyhow::Result<Value> {
    let link = home_link(app, peer, frame)?;
    let days = frame["days"].as_i64().unwrap_or(7).clamp(1, 28);
    Ok(json!({ "metrics": crate::ws::home_metrics(app, &link.project_id, days)? }))
}

/// `dashboard_needs_you {project_id}`: the dashboard's Needs you as this home
/// has it, in the peer's ids (H-112). Rulings stay here, so no package can
/// be ruled from there. With `all_kinds`, the kinds the projects home added
/// too (H-128).
pub(super) fn serve_needs_you(
    app: &Arc<AppState>,
    peer: &Peer,
    frame: &Value,
) -> anyhow::Result<Value> {
    let link = home_link(app, peer, frame)?;
    let since = chrono::Utc::now() - chrono::Duration::days(7);
    let all_kinds = frame["all_kinds"].as_bool().unwrap_or(false);
    let needs = crate::attention::needs_you(app, &link.project_id, since, all_kinds, &|release| {
        let mut v = release.to_json();
        v["can_rule"] = json!(false);
        Ok(v)
    })?;
    let mut answer = json!({
        "rows": needs.rows, "wip_overrides": needs.wip_overrides, "count": needs.count,
    });
    board_ids::json(&mut answer, &|id| board_ids::to_peer(&app.db, &peer.id, id));
    Ok(answer)
}

/// `board_snapshot {project_id}`: the whole board, in the peer's ids.
pub(super) fn serve_snapshot(
    app: &Arc<AppState>,
    peer: &Peer,
    frame: &Value,
) -> anyhow::Result<Value> {
    let link = home_link(app, peer, frame)?;
    let mut board = crate::ws::home_snapshot(app, &link.project_id)?
        .ok_or_else(|| refuse("no_board", "this computer doesn't hold that board"))?;
    board_ids::snapshot(&mut board, &link.remote_project_id, |id| {
        board_ids::to_peer(&app.db, &peer.id, id)
    });
    Ok(json!({ "board": board }))
}

/// Relays every change to a board homed here to the peers linked to its
/// project. Changes to a mirrored board are published locally too, and are
/// not sent back: only a project with its board here is relayed.
pub fn spawn_relay(app: Arc<AppState>) {
    let mut rx = app.board.subscribe();
    tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(change) => relay(&app, &change),
                // Too far behind: each peer refetches what it mirrors.
                Err(RecvError::Lagged(_)) => resync_all(&app),
                Err(RecvError::Closed) => break,
            }
        }
    });
}

fn relay(app: &AppState, change: &BoardChange) {
    let db = &app.db;
    if db
        .board_settings(&change.project_id)
        .ok()
        .flatten()
        .is_none()
    {
        return;
    }
    for link in db.project_links(&change.project_id).unwrap_or_default() {
        if !app.peers.is_online(&link.peer_id) {
            continue;
        }
        let card = change.card.clone().map(|card| {
            let mut card = c::ItemCard::from(card);
            board_ids::card(&mut card, |id| board_ids::to_peer(db, &link.peer_id, id));
            card
        });
        app.peers.notify(
            &link.peer_id,
            json!({
                "type": "board_event",
                "project_id": change.project_id,
                "kind": kind_name(change.kind),
                "item_id": change.item_id,
                "card": card,
                "from_column": change.from_column,
            }),
        );
    }
}

fn resync_all(app: &AppState) {
    tracing::warn!("board relay lagged; asking linked peers to refetch");
    for peer in app.db.list_peers().unwrap_or_default() {
        for link in app.db.links_through(&peer.id).unwrap_or_default() {
            if app
                .db
                .board_settings(&link.project_id)
                .ok()
                .flatten()
                .is_some()
            {
                app.peers.notify(
                    &peer.id,
                    json!({"type": "board_event", "project_id": link.project_id,
                           "kind": "resync"}),
                );
            }
        }
    }
}

pub(super) fn kind_name(kind: ChangeKind) -> &'static str {
    match kind {
        ChangeKind::ItemUpserted => "item_upserted",
        ChangeKind::ItemMoved => "item_moved",
        ChangeKind::ItemRemoved => "item_removed",
        ChangeKind::ColumnsChanged => "columns_changed",
        ChangeKind::SettingsChanged => "settings_changed",
    }
}

/// A relayed kind; `None` for `resync` and anything newer than this daemon.
pub(super) fn kind_from(name: &str) -> Option<ChangeKind> {
    [
        ChangeKind::ItemUpserted,
        ChangeKind::ItemMoved,
        ChangeKind::ItemRemoved,
        ChangeKind::ColumnsChanged,
        ChangeKind::SettingsChanged,
    ]
    .into_iter()
    .find(|kind| kind_name(*kind) == name)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    #[test]
    fn a_remote_install_gets_urls_never_the_homes_paths() {
        let mut ok = json!({"builds": [{"artifact": "/b/x", "url": "https://d/x"}]});
        super::published_builds(&mut ok).expect("published");
        assert_eq!(ok, json!({"builds": [{"url": "https://d/x"}]}));
        let mut local = json!({"builds": [{"artifact": "/b/x", "url": null}]});
        let refused = super::published_builds(&mut local).unwrap_err();
        assert_eq!(
            refused.to_string(),
            "build not published over HTTPS; DevOps runs release_publish"
        );
    }
}
