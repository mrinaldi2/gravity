//! `dashboard_get {project_id}` (H-076): what U5's widgets show, in one read
//! (H-018 §2.1). Needs you: releases awaiting a ruling, open and relayed
//! decisions, P0 items, recent WIP overrides. The board's column strip with
//! its blocked, stale, done and rework counts. The current release and the
//! last two. The team, each bot with its items in Doing and its open tasks.
//! Meetings (H-102): each series with its next time, the meeting collecting
//! and the last one held; and the open action items.
//!
//! A board mirrored from its home serves the strip and the P0s from the
//! mirror; its history, and so the weekly counts and releases, stay there.

use std::collections::HashMap;

use chrono::{Duration, Utc};
use serde_json::{json, Value};

use super::Conn;
use crate::board::model::{ColumnCategory, ItemCard};

/// The window "this week" covers: the last seven days, as of the read.
const WEEK_DAYS: i64 = 7;
/// Releases listed: the current one and the last two.
const RELEASES_SHOWN: usize = 3;

impl Conn {
    pub(super) fn dashboard_get(&self, req_id: &Value, req: &Value) -> anyhow::Result<()> {
        let project_id = Self::str_field(req, "project_id")?;
        let now = Utc::now();
        let since = now - Duration::days(WEEK_DAYS);
        let db = &self.app.db;
        let local = db.board_settings(project_id)?.is_some();
        let (columns, cards) = if local {
            (db.board_columns(project_id)?, db.board_cards(project_id)?)
        } else {
            mirrored(&self.app, project_id)
        };
        let category: HashMap<&str, ColumnCategory> = columns
            .iter()
            .map(|c| (c.key.as_str(), c.category))
            .collect();
        let open = |card: &&ItemCard| {
            !matches!(
                category.get(card.column_key.as_str()),
                Some(ColumnCategory::Done | ColumnCategory::Cancelled)
            )
        };

        let releases = if local {
            db.board_read(|t| t.releases(project_id))?
        } else {
            Vec::new()
        };
        // What's kept here: the board's rows when it lives here, and this
        // computer's own decisions either way (they don't sync, H-030).
        // Off-home the home's rows are added once it answers.
        let home_peer = (!local)
            .then(|| self.app.board_mirror.home_peer(project_id))
            .flatten();
        // The kinds added with the projects home (H-128) go to a client that
        // asks for them; an older one would not know how to show them.
        let all_kinds = req["all_kinds"].as_bool().unwrap_or(false);
        let mut needs =
            crate::attention::needs_you(&self.app, project_id, since, all_kinds, &|r| {
                self.release_json(r)
            })?;
        // This computer's routines with no card (H-135 G5): here only, and
        // not sent to a linked computer.
        needs.rows.extend(crate::attention::routines_without_card(
            &self.app, project_id,
        )?);

        let strip: Vec<Value> = columns
            .iter()
            .filter(|c| c.visible)
            .map(|c| {
                json!({
                    "key": c.key, "name": c.name, "category": c.category.as_str(),
                    "count": cards.iter().filter(|card| card.column_key == c.key).count(),
                    "wip_limit": c.wip_limit, "wip_scope": c.wip_scope.as_str(),
                })
            })
            .collect();
        // The weekly counts come from the board's history, on its home only.
        let (done, rework) = if local {
            (
                Some(db.done_since(project_id, since)?),
                Some(db.rework_since(project_id, since)?),
            )
        } else {
            (None, None)
        };
        let board = (!columns.is_empty()).then(|| {
            json!({
                "columns": strip,
                "blocked": cards.iter().filter(open).filter(|c| c.blocked).count(),
                "stale": cards.iter().filter(open).filter(|c| c.stale).count(),
                "done_this_week": done,
                "rework_this_week": rework,
            })
        });

        let doing: Vec<&ItemCard> = cards
            .iter()
            .filter(|c| category.get(c.column_key.as_str()) == Some(&ColumnCategory::Doing))
            .collect();
        let team = db
            .list_bots(Some(project_id))?
            .iter()
            .map(|bot| {
                // Usually one (Doing's limit is per bot), in board order.
                let items: Vec<Value> = doing
                    .iter()
                    .filter(|c| c.assignee.as_deref() == Some(bot.id.as_str()))
                    .map(|c| json!({"id": c.id, "title": c.title}))
                    .collect();
                Ok(json!({
                    "bot": self.bot_json(bot),
                    "items": items,
                    "open_tasks": db.open_tasks_for(&bot.id)?.len(),
                }))
            })
            .collect::<anyhow::Result<Vec<_>>>()?;

        let shown = releases
            .iter()
            .take(RELEASES_SHOWN)
            .map(|r| self.release_json(r))
            .collect::<anyhow::Result<Vec<_>>>()?;
        // Meetings live with the board, on its home (H-102).
        let (meetings, action_items) = if local {
            crate::board::meetings::dashboard(&self.app, project_id)?
        } else {
            (json!([]), json!([]))
        };
        let home = home_peer
            .as_deref()
            .map(|peer| crate::peer::board::home_name(&self.app, peer));
        let dashboard = json!({
            "project_id": project_id, "as_of": now, "since": since,
            "home": home, "needs_you": needs.rows, "wip_overrides": needs.wip_overrides,
            "needs_you_note": Value::Null, "board": board,
            "releases": shown, "team": team,
            "meetings": meetings, "action_items": action_items,
        });
        match home_peer {
            Some(peer) => self.answer_later(
                req_id,
                from_home(
                    self.app.clone(),
                    project_id.to_string(),
                    peer,
                    all_kinds,
                    dashboard,
                ),
            ),
            None => {
                self.send(json!({ "type": "dashboard", "req_id": req_id, "dashboard": dashboard }))
            }
        }
        Ok(())
    }
}

/// Off-home, Needs you adds the board's home's rows (H-112): read-only here,
/// each marked with where to act on it. When the home can't be reached the
/// dashboard says so.
async fn from_home(
    app: std::sync::Arc<crate::app::AppState>,
    project_id: String,
    home: String,
    all_kinds: bool,
    mut dashboard: Value,
) -> anyhow::Result<Value> {
    let name = crate::peer::board::home_name(&app, &home);
    let frame = json!({
        "type": "dashboard_needs_you", "project_id": project_id, "all_kinds": all_kinds,
    });
    let link = app.db.project_link(&project_id, &home)?;
    match (app.peers.request(&home, frame).await, link) {
        (Ok(mut answer), Some(link)) => {
            crate::peer::board::ids_from_home(&app, &link, &mut answer);
            let mut rows = match answer["rows"].take() {
                Value::Array(rows) => rows,
                _ => Vec::new(),
            };
            for row in &mut rows {
                row["elsewhere"] = json!(name);
            }
            if let Some(here) = dashboard["needs_you"].as_array_mut() {
                here.extend(rows);
            }
            dashboard["wip_overrides"] = answer["wip_overrides"].take();
        }
        _ => {
            // What's here may not be all; "nothing" can't be known (UX-016 §3).
            dashboard["needs_you_note"] = json!(format!(
                "Can't reach {name} right now, so this may not be everything that needs you."
            ));
        }
    }
    Ok(json!({ "type": "dashboard", "dashboard": dashboard }))
}

/// The columns and cards of a board mirrored from its home, in model types.
fn mirrored(
    app: &crate::app::AppState,
    project_id: &str,
) -> (Vec<crate::board::model::BoardColumn>, Vec<ItemCard>) {
    let Some(board) = app.board_mirror.get(project_id) else {
        return (Vec::new(), Vec::new());
    };
    let columns = board
        .snapshot
        .columns
        .into_iter()
        .filter_map(|c| c.try_into().ok())
        .collect();
    let cards = board
        .snapshot
        .cards
        .into_iter()
        .filter_map(|c| c.try_into().ok())
        .collect();
    (columns, cards)
}
