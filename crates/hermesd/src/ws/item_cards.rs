//! Card ids as links (H-203, UX-035 §9): `item_cards_get {ids}` answers, for
//! each id, the card and where it is kept, so a client can show a preview
//! and open it. One request covers a whole message or page.
//!
//! An id is looked up on the boards kept here, then on the boards mirrored
//! from a linked computer. A card on a computer that is offline comes back
//! `unreachable`, with the last copy the mirror holds. An id whose prefix is
//! a known project's but which no board has is `missing`; an id with no known
//! prefix too, without a project.

use bus::contract::board as c;
use serde_json::{json, Value};

use super::Conn;
use crate::app::AppState;
use crate::board::release::machines;

/// The most ids one request may ask for.
const MAX_IDS: usize = 200;

/// A project's item prefix (`H` for `H-017`): its board's key here, or the
/// key of the board mirrored from its home.
pub(crate) fn item_prefix(app: &AppState, project_id: &str) -> Option<String> {
    if let Ok(Some(settings)) = app.db.board_settings(project_id) {
        return Some(settings.key);
    }
    app.board_mirror
        .get(project_id)
        .and_then(|m| m.snapshot.settings.map(|s| s.key))
}

impl Conn {
    pub(super) fn item_cards_get(&self, req_id: &Value, req: &Value) -> anyhow::Result<()> {
        let mut ids: Vec<String> = req["ids"]
            .as_array()
            .map(|ids| {
                ids.iter()
                    .filter_map(|v| v.as_str())
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect()
            })
            .unwrap_or_default();
        ids.dedup();
        if ids.len() > MAX_IDS {
            self.reply_err(
                req_id,
                "invalid_request",
                &format!("at most {MAX_IDS} ids at a time"),
            );
            return Ok(());
        }
        let cards = ids
            .iter()
            .map(|id| card_entry(&self.app, id))
            .collect::<anyhow::Result<Vec<_>>>()?;
        self.send(json!({ "type": "item_cards", "req_id": req_id, "cards": cards }));
        Ok(())
    }
}

/// One id's entry.
fn card_entry(app: &AppState, id: &str) -> anyhow::Result<Value> {
    let db = &app.db;
    if let Some(project_id) = db.board_read(|t| t.item_project(id))? {
        return here(app, id, &project_id);
    }
    if let Some(project_id) = app.board_mirror.project_of(id) {
        if let Some(entry) = mirrored(app, id, &project_id)? {
            return Ok(entry);
        }
    }
    missing(app, id)
}

/// A card on a board kept on this computer.
fn here(app: &AppState, id: &str, project_id: &str) -> anyhow::Result<Value> {
    let db = &app.db;
    let (card, item, computer) =
        db.board_read(|t| Ok((t.card(id)?, t.item(id)?, machines::this_computer(t)?)))?;
    let Some(card) = card else {
        return missing(app, id);
    };
    let column_name = db
        .board_columns(project_id)?
        .into_iter()
        .find(|col| col.key == card.column_key)
        .map(|col| col.name);
    let release = match item.and_then(|i| i.release_id) {
        Some(release_id) => db
            .board_read(|t| t.release(&release_id))?
            .map(|r| r.display_version.unwrap_or(r.name)),
        None => None,
    };
    Ok(json!({
        "id": id,
        "project_id": project_id,
        "project_name": project_name(app, project_id),
        "computer": computer,
        "card": serde_json::to_value(c::ItemCard::from(card))?,
        "column_name": column_name,
        "release": release,
    }))
}

/// A card on a board mirrored from its home: live when the home is
/// reachable, the last copy with `unreachable` when it isn't.
fn mirrored(app: &AppState, id: &str, project_id: &str) -> anyhow::Result<Option<Value>> {
    let Some(board) = app.board_mirror.get(project_id) else {
        return Ok(None);
    };
    let Some(card) = board.snapshot.cards.iter().find(|card| card.id == id) else {
        return Ok(None);
    };
    let column_name = board
        .snapshot
        .columns
        .iter()
        .find(|col| col.key == card.column_key)
        .map(|col| col.name.clone());
    let (computer, last_seen) = peer(app, &board.peer_id);
    let mut entry = json!({
        "id": id,
        "project_id": project_id,
        "project_name": project_name(app, project_id),
        "computer": computer,
        "card": serde_json::to_value(card)?,
        "column_name": column_name,
    });
    if !app.peers.is_online(&board.peer_id) {
        entry["unreachable"] = json!({ "computer": computer, "last_seen": last_seen });
    }
    Ok(Some(entry))
}

/// No board holds `id`. With a known prefix, the project it would be on.
fn missing(app: &AppState, id: &str) -> anyhow::Result<Value> {
    let Some((prefix, _)) = id.rsplit_once('-') else {
        return Ok(json!({ "id": id, "missing": true }));
    };
    for project in app.db.list_projects()? {
        if !item_prefix(app, &project.id).is_some_and(|p| p.eq_ignore_ascii_case(prefix)) {
            continue;
        }
        let mut entry = json!({
            "id": id,
            "project_id": project.id,
            "project_name": project_name(app, &project.id),
            "missing": true,
        });
        // Kept on a computer that's offline: not missing, out of reach.
        if let Some(peer_id) = app.board_mirror.home_peer(&project.id) {
            let (computer, last_seen) = peer(app, &peer_id);
            entry["computer"] = json!(computer);
            if !app.peers.is_online(&peer_id) {
                entry["missing"] = Value::Null;
                entry["unreachable"] = json!({ "computer": computer, "last_seen": last_seen });
            }
        }
        return Ok(entry);
    }
    Ok(json!({ "id": id, "missing": true }))
}

fn project_name(app: &AppState, project_id: &str) -> Option<String> {
    app.db
        .get_project(project_id)
        .ok()
        .flatten()
        .map(|p| crate::db::Db::display_project_name(&p))
}

/// A linked computer's name and when it was last seen.
fn peer(app: &AppState, peer_id: &str) -> (String, Option<String>) {
    match app.db.get_peer(peer_id).ok().flatten() {
        Some(peer) => (
            crate::db::Db::display_peer_name(&peer),
            peer.last_seen_at.map(|t| t.to_rfc3339()),
        ),
        None => (peer_id.to_string(), None),
    }
}
