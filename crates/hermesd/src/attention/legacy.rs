//! The dashboard's Needs you (H-112) as it has always been sent: the rows
//! in their JSON shapes, and the WIP overrides beside them, for information
//! and never counted. Built on [`super::rows`], so it lists exactly the rows
//! the projects home counts.

use std::collections::BTreeMap;

use bus::Decision;
use chrono::{DateTime, Utc};
use serde_json::{json, Value};

use super::{rows, Built, Scope};
use crate::app::AppState;
use crate::board::release::model::Release;

/// The rows, the WIP overrides shown beside them, and how many rows the
/// projects home counts for them (H-161): the dashboard's header shows
/// that number, so both say the same.
pub(crate) struct NeedsYou {
    pub rows: Vec<Value>,
    pub wip_overrides: Vec<Value>,
    pub count: u32,
}

/// Needs you as this computer has it for the project: its own rows, and the
/// board's when the board lives here. Without `all_kinds`, only the kinds
/// the dashboard has always shown, so an older client reads every row.
pub(crate) fn needs_you(
    app: &AppState,
    project_id: &str,
    since: DateTime<Utc>,
    all_kinds: bool,
    release_json: &dyn Fn(&Release) -> anyhow::Result<Value>,
) -> anyhow::Result<NeedsYou> {
    let built = rows(app, project_id, Scope::here(app, project_id), release_json)?;
    let wip_overrides = app
        .db
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
        rows: dashboard_rows(&built, all_kinds),
        wip_overrides,
        count: super::summary(built.iter().map(|b| &b.row)).count,
    })
}

/// The rows as the dashboard sends them. The kinds added with the projects
/// home (H-128) are sent only to a client that asks for every kind, as the
/// typed row in proto3 JSON under `kind`.
pub(crate) fn dashboard_rows(built: &[Built], all_kinds: bool) -> Vec<Value> {
    built
        .iter()
        .filter_map(|b| match &b.legacy {
            Some(legacy) => Some(legacy.clone()),
            None if all_kinds => {
                let mut row = serde_json::to_value(&b.row).ok()?;
                row["kind"] = json!(super::kind_key(b.row.kind()));
                Some(row)
            }
            None => None,
        })
        .collect()
}

/// One row listing this computer's routines that name no card (H-135 G5),
/// while the project has a board. Shown on this computer's dashboard only,
/// for information: it is not an attention row, so it isn't in `count` nor
/// the projects home's, and a linked computer lists its own.
pub(crate) fn routines_without_card(
    app: &AppState,
    project_id: &str,
) -> anyhow::Result<Option<Value>> {
    if !crate::mcp::has_board(app, project_id) {
        return Ok(None);
    }
    let mut cardless = Vec::new();
    for bot in app.db.list_bots(Some(project_id))? {
        if bot.is_linked() {
            continue;
        }
        for routine in app.db.list_routines(Some(&bot.id))? {
            if app.db.routine_card(&routine.id)?.is_none() {
                cardless.push(json!({ "id": routine.id, "name": routine.name, "bot_id": bot.id }));
            }
        }
    }
    Ok((!cardless.is_empty())
        .then(|| json!({ "kind": "routines_without_card", "routines": cardless })))
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
pub(super) fn relayed_ruling(decision: &Decision) -> Value {
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
pub(super) fn relayed_row(relayed: &[Value]) -> Option<Value> {
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
