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
use crate::board::release::confine;
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
    // The served folder is refused, so no build can be published (H-100).
    if let Some(reason) = confine::serving_refused(&app.cfg) {
        rows.push(
            json!({ "kind": "serving_off", "title": confine::SERVING_OFF, "reason": reason }),
        );
    }
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
    let relayed: Vec<Value> = relayed.iter().map(|d| relayed_ruling(d)).collect();
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

/// What the confirm dialog lists for one relayed ruling: its title, the
/// recorded answer, the bot that recorded it (`bot_id`, so a linked
/// computer reads it in its own ids) and when.
fn relayed_ruling(decision: &Decision) -> Value {
    let ruling = decision.ruling.as_ref();
    json!({
        "id": decision.id, "title": decision.title,
        "answer": ruling.map(|r| r.text.as_str()).unwrap_or_default(),
        "bot_id": relayed_by(decision).unwrap_or_default(),
        "at": ruling.map(|r| r.answered_at),
    })
}

/// One row for every relayed ruling (from `relayed_ruling`): how many,
/// which bots recorded them, and each one, for the dialog that confirms
/// exactly those.
fn relayed_row(relayed: &[Value]) -> Option<Value> {
    if relayed.is_empty() {
        return None;
    }
    let mut by: BTreeMap<&str, usize> = BTreeMap::new();
    for ruling in relayed {
        *by.entry(ruling["bot_id"].as_str().unwrap_or_default())
            .or_default() += 1;
    }
    let by: Vec<Value> = by
        .into_iter()
        .map(|(bot_id, count)| json!({ "bot_id": bot_id, "count": count }))
        .collect();
    let ids: Vec<&Value> = relayed.iter().map(|r| &r["id"]).collect();
    Some(json!({
        "kind": "relayed", "count": relayed.len(), "decision_ids": ids, "by": by,
        "rulings": relayed,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relayed_rulings_are_one_row_counted_by_bot() {
        let ruling = |id: &str, by: &str| json!({ "id": id, "title": id, "bot_id": by });
        let rulings = [
            ruling("d1", "lead"),
            ruling("d2", "lead"),
            ruling("d3", "pm"),
        ];
        let row = relayed_row(&rulings).expect("a row");
        assert_eq!(row["count"], 3);
        assert_eq!(row["decision_ids"], json!(["d1", "d2", "d3"]));
        assert_eq!(
            row["by"],
            json!([{"bot_id": "lead", "count": 2}, {"bot_id": "pm", "count": 1}])
        );
        assert_eq!(row["rulings"][2]["title"], "d3");
        assert!(relayed_row(&[]).is_none());
    }
}
