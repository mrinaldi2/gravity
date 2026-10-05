//! `dashboard_get {project_id}` (H-076): what U5's first four widgets show,
//! in one read (H-018 §2.1). Needs you: releases awaiting a ruling, open and
//! relayed decisions, P0 items, recent WIP overrides. The board's column
//! strip with its blocked, stale, done and rework counts. The current
//! release and the last two. The team, each bot with its items in Doing and
//! its open tasks. Meetings and action items come with their own item (H-102).
//!
//! A board mirrored from its home serves the strip and the P0s from the
//! mirror; its history, and so the weekly counts and releases, stay there.

use std::collections::HashMap;

use bus::DecisionState;
use chrono::{Duration, Utc};
use serde_json::{json, Value};

use super::Conn;
use crate::board::model::{ColumnCategory, ItemCard, Priority};
use crate::board::release::model::ReleaseStatus;
use crate::db::DecisionFilter;
use crate::decisions::authority::is_relayed;

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
        let mut needs_you = Vec::new();
        for release in releases
            .iter()
            .filter(|r| r.status == ReleaseStatus::AwaitingOwner)
        {
            needs_you.push(json!({ "kind": "release", "release": self.release_json(release)? }));
        }
        let release_decisions: Vec<&str> = releases
            .iter()
            .filter_map(|r| r.decision_id.as_deref())
            .collect();
        let decisions = db.list_decisions(&DecisionFilter {
            project_id: Some(project_id),
            states: &[DecisionState::Open, DecisionState::Settled],
            tag: None,
            bot_id: None,
            query: None,
            before: None,
            limit: 200,
        })?;
        for d in decisions
            .iter()
            .filter(|d| !release_decisions.contains(&d.id.as_str()))
        {
            let relayed = is_relayed(d);
            if d.state == DecisionState::Open || relayed {
                needs_you.push(json!({
                    "kind": "decision", "id": d.id, "title": d.title,
                    "priority": d.priority, "deadline_at": d.deadline_at, "relayed": relayed,
                    "raised_by": d.raised_by_bot_id,
                }));
            }
        }
        for card in cards
            .iter()
            .filter(open)
            .filter(|c| c.priority == Priority::P0)
        {
            needs_you.push(json!({
                "kind": "p0", "id": card.id, "title": card.title,
                "column_key": card.column_key, "assignee": card.assignee,
            }));
        }
        if local {
            for o in db.wip_overrides_since(project_id, since)? {
                needs_you.push(json!({
                    "kind": "wip_override", "id": o.item_id, "title": o.title,
                    "column_key": o.column_key, "actor": o.actor, "note": o.note, "at": o.at,
                }));
            }
        }

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
        let home = (!local)
            .then(|| self.app.board_mirror.home_peer(project_id))
            .flatten()
            .map(|peer| crate::peer::board::home_name(&self.app, &peer));
        self.send(json!({
            "type": "dashboard", "req_id": req_id,
            "dashboard": {
                "project_id": project_id, "as_of": now, "since": since,
                "home": home, "needs_you": needs_you, "board": board,
                "releases": shown, "team": team,
                "meetings": [], "action_items": [],
            },
        }));
        Ok(())
    }
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
