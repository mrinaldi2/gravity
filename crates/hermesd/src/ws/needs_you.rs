//! The dashboard's "Needs you" (H-112): only what the owner must act on.
//! Releases awaiting a ruling, open decisions, P0 items, and the rulings a
//! bot recorded for the owner as one row to confirm together. WIP overrides
//! are the lead's to make, so they're listed beside it, for information.
//!
//! Built where the board lives. A linked computer asks the home for it
//! (`dashboard_needs_you`), and says where to look when the home is away.

use std::collections::BTreeMap;

use bus::{Decision, DecisionState};
use chrono::{DateTime, Utc};
use serde_json::{json, Value};

use crate::app::AppState;
use crate::board::model::{ColumnCategory, Priority};
use crate::board::release::model::{Release, ReleaseStatus};
use crate::db::DecisionFilter;
use crate::decisions::authority::is_relayed;

/// The rows, and the WIP overrides shown beside them.
pub(crate) struct NeedsYou {
    pub rows: Vec<Value>,
    pub wip_overrides: Vec<Value>,
}

/// Needs you for a project whose board is on this computer.
pub(crate) fn home_needs_you(
    app: &AppState,
    project_id: &str,
    since: DateTime<Utc>,
    release_json: impl Fn(&Release) -> anyhow::Result<Value>,
) -> anyhow::Result<NeedsYou> {
    let db = &app.db;
    let releases = db.board_read(|t| t.releases(project_id))?;
    let mut rows = Vec::new();
    for release in releases
        .iter()
        .filter(|r| r.status == ReleaseStatus::AwaitingOwner)
    {
        rows.push(json!({ "kind": "release", "release": release_json(release)? }));
    }
    let release_decisions: Vec<&str> = releases
        .iter()
        .filter_map(|r| r.decision_id.as_deref())
        .collect();
    let decisions = project_decisions(app, project_id)?;
    let (relayed, open): (Vec<&Decision>, Vec<&Decision>) = decisions
        .iter()
        .filter(|d| !release_decisions.contains(&d.id.as_str()))
        .filter(|d| d.state == DecisionState::Open || is_relayed(d))
        .partition(|d| is_relayed(d));
    for d in open {
        rows.push(json!({
            "kind": "decision", "id": d.id, "title": d.title,
            "priority": d.priority, "deadline_at": d.deadline_at, "relayed": false,
            "raised_by": d.raised_by_bot_id,
        }));
    }
    let relayed: Vec<(&str, &str)> = relayed
        .iter()
        .map(|d| (d.id.as_str(), relayed_by(d).unwrap_or_default()))
        .collect();
    if let Some(row) = relayed_row(&relayed) {
        rows.push(row);
    }
    let columns = db.board_columns(project_id)?;
    let done = |key: &str| {
        columns.iter().any(|c| {
            c.key == key && matches!(c.category, ColumnCategory::Done | ColumnCategory::Cancelled)
        })
    };
    for card in db
        .board_cards(project_id)?
        .iter()
        .filter(|c| c.priority == Priority::P0 && !done(&c.column_key))
    {
        rows.push(json!({
            "kind": "p0", "id": card.id, "title": card.title,
            "column_key": card.column_key, "assignee": card.assignee,
        }));
    }
    let wip_overrides = db
        .wip_overrides_since(project_id, since)?
        .into_iter()
        .map(|o| {
            json!({
                "id": o.item_id, "title": o.title, "column_key": o.column_key,
                "actor": o.actor, "note": o.note, "at": o.at,
            })
        })
        .collect();
    Ok(NeedsYou {
        rows,
        wip_overrides,
    })
}

/// Open and settled decisions of the project, newest first.
pub(crate) fn project_decisions(app: &AppState, project_id: &str) -> anyhow::Result<Vec<Decision>> {
    app.db.list_decisions(&DecisionFilter {
        project_id: Some(project_id),
        states: &[DecisionState::Open, DecisionState::Settled],
        tag: None,
        bot_id: None,
        query: None,
        before: None,
        limit: 200,
    })
}

/// The bot that relayed a ruling: `owner-via-bot:<id>`.
fn relayed_by(decision: &Decision) -> Option<&str> {
    decision
        .ruling
        .as_ref()?
        .answered_by
        .strip_prefix("owner-via-bot:")
}

/// One row for every relayed ruling (`(decision, bot)`): how many, and
/// which bots recorded them.
fn relayed_row(relayed: &[(&str, &str)]) -> Option<Value> {
    if relayed.is_empty() {
        return None;
    }
    let mut by: BTreeMap<&str, usize> = BTreeMap::new();
    for (_, bot) in relayed {
        *by.entry(bot).or_default() += 1;
    }
    let by: Vec<Value> = by
        .into_iter()
        .map(|(bot_id, count)| json!({ "bot_id": bot_id, "count": count }))
        .collect();
    let ids: Vec<&str> = relayed.iter().map(|(id, _)| *id).collect();
    Some(json!({
        "kind": "relayed", "count": relayed.len(), "decision_ids": ids, "by": by,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relayed_rulings_are_one_row_counted_by_bot() {
        let row = relayed_row(&[("d1", "lead"), ("d2", "lead"), ("d3", "pm")]).expect("a row");
        assert_eq!(row["count"], 3);
        assert_eq!(row["decision_ids"], json!(["d1", "d2", "d3"]));
        assert_eq!(
            row["by"],
            json!([{"bot_id": "lead", "count": 2}, {"bot_id": "pm", "count": 1}])
        );
        assert!(relayed_row(&[]).is_none());
    }
}
