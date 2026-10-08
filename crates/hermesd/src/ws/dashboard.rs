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
        // computer's own decisions either way (they don't sync, H-030). Every
        // linked computer's rows, the board's home included, come from the
        // part the projects home counts (H-178), not a request per read.
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
        let (count, note) = linked_needs(&self.app, project_id, all_kinds, &mut needs)?;

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
            "needs_you_count": count, "needs_you_note": note, "board": board,
            "releases": shown, "team": team,
            "meetings": meetings, "action_items": action_items,
        });
        self.send(json!({ "type": "dashboard", "req_id": req_id, "dashboard": dashboard }));
        Ok(())
    }
}

/// Every linked computer's rows, as the projects home counts them (H-178):
/// each linked computer's last good part, the board's home included. Each
/// row goes out as `{kind: "elsewhere", row, elsewhere}`: the typed row in
/// proto3 JSON (a `decision` there is not the dashboard's own `decision`
/// shape) and the computer to act on it. An older client asks without
/// `all_kinds` and gets only this computer's rows, as before. Returns the
/// count the projects home shows, and a note naming any computer not
/// answering, whose last good rows are listed but may be out of date.
fn linked_needs(
    app: &crate::app::AppState,
    project_id: &str,
    all_kinds: bool,
    needs: &mut crate::attention::NeedsYou,
) -> anyhow::Result<(u32, Option<String>)> {
    let linked = crate::overview::linked_rows(app, project_id)?;
    let count = needs.count + crate::attention::summary(linked.rows.iter().map(|(_, r)| r)).count;
    if all_kinds {
        for (computer, row) in &linked.rows {
            let mut typed = serde_json::to_value(row)?;
            typed["kind"] = json!(crate::attention::kind_key(row.kind()));
            needs
                .rows
                .push(json!({"kind": "elsewhere", "row": typed, "elsewhere": computer}));
        }
    }
    let away: Vec<&str> = linked
        .sources
        .iter()
        .filter(|s| s.state() != bus::contract::home::source::State::Ok)
        .map(|s| s.name.as_str())
        .collect();
    // What's here may not be all; "nothing" can't be known (UX-016 §3).
    let note = (!away.is_empty()).then(|| {
        format!(
            "Can't reach {} right now, so this may not be everything that needs you.",
            away.join(" or ")
        )
    });
    Ok((count, note))
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
