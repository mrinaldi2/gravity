//! Action items (H-017 §1.5): recorded by the facilitator, ticked or
//! dropped by their owner, and promoted by the lead to a chore on the board
//! when they need real work. The chore links back to its meeting.

use std::sync::Arc;

use chrono::{DateTime, NaiveDate, Utc};

use crate::app::AppState;
use crate::board::feed::{card_after_commit, Change, ChangeKind};
use crate::board::guards;
use crate::board::model::{ItemType, LinkKind, Priority};
use crate::db::NewItem;
use crate::decisions::{conflict, forbidden, invalid, not_found};

use super::model::{ActionItem, ActionStatus, MeetingStatus};
use super::run::changed;
use super::{load, resolve_who, Party};

/// RFC 3339, or a bare date meaning the end of that day (UTC).
pub fn parse_due(text: &str) -> anyhow::Result<Option<DateTime<Utc>>> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(None);
    }
    if let Ok(at) = DateTime::parse_from_rfc3339(text) {
        return Ok(Some(at.with_timezone(&Utc)));
    }
    NaiveDate::parse_from_str(text, "%Y-%m-%d")
        .ok()
        .and_then(|d| d.and_hms_opt(23, 59, 59))
        .map(|d| Some(d.and_utc()))
        .ok_or_else(|| {
            invalid(format!(
                "due_at {text} isn't a date (RFC 3339 or YYYY-MM-DD)"
            ))
        })
}

/// What `action_add` records.
pub struct NewAction<'a> {
    pub text: &'a str,
    pub owner: &'a str,
    pub due_at: Option<&'a str>,
}

/// The facilitator (or the lead) records an action from a meeting.
pub fn add(
    app: &Arc<AppState>,
    party: &Party<'_>,
    project_id: &str,
    meeting_id: &str,
    new: &NewAction<'_>,
) -> anyhow::Result<ActionItem> {
    let m = load(app, project_id, meeting_id)?;
    party.runs(&m, "add its action items")?;
    if m.status == MeetingStatus::Skipped {
        return Err(conflict(format!("{} was skipped", m.id)));
    }
    let text = new.text.trim();
    if text.is_empty() {
        return Err(invalid("an action item needs a text"));
    }
    let now = Utc::now();
    let action = ActionItem {
        id: bus::new_id(),
        project_id: project_id.to_string(),
        meeting_id: m.id.clone(),
        series_id: m.series_id.clone(),
        text: text.to_string(),
        owner: resolve_who(app, project_id, new.owner)?,
        due_at: new.due_at.map(parse_due).transpose()?.flatten(),
        status: ActionStatus::Open,
        item_id: None,
        created_at: now,
        updated_at: now,
    };
    app.db.board_tx(|t| t.insert_action(&action))?;
    changed(app, project_id, Some(&m.id));
    Ok(action)
}

fn own_action(app: &AppState, project_id: &str, id: &str) -> anyhow::Result<ActionItem> {
    match app.db.board_read(|t| t.action(id))? {
        Some(a) if a.project_id == project_id => Ok(a),
        _ => Err(not_found(format!("no action item {id} in this project"))),
    }
}

/// What `action_update` changes; unset keeps.
#[derive(Default)]
pub struct ActionEdit<'a> {
    pub status: Option<&'a str>,
    pub text: Option<&'a str>,
    /// Empty clears the due date.
    pub due_at: Option<&'a str>,
}

/// The action's owner, its meeting's facilitator or the lead ticks, drops,
/// reopens or rewords it.
pub fn update(
    app: &Arc<AppState>,
    party: &Party<'_>,
    project_id: &str,
    action_id: &str,
    edit: &ActionEdit<'_>,
) -> anyhow::Result<ActionItem> {
    let mut action = own_action(app, project_id, action_id)?;
    if party.who() != action.owner && !party.leads() {
        let m = load(app, project_id, &action.meeting_id)?;
        if party.who() != m.facilitator {
            return Err(forbidden(
                "only the action's owner, its meeting's facilitator or the lead can change it",
            ));
        }
    }
    if let Some(status) = edit.status {
        action.status = ActionStatus::parse(status.trim())
            .ok_or_else(|| invalid(format!("unknown status {status}; open, done or dropped")))?;
    }
    if let Some(text) = edit.text.map(str::trim) {
        if text.is_empty() {
            return Err(invalid("an action item needs a text"));
        }
        action.text = text.to_string();
    }
    if let Some(due) = edit.due_at {
        action.due_at = parse_due(due)?;
    }
    app.db.board_tx(|t| t.save_action(&action))?;
    changed(app, project_id, Some(&action.meeting_id));
    own_action(app, project_id, action_id)
}

/// The lead (or the owner) turns an open action into a chore on the board,
/// linked to its meeting. The action keeps its owner and stays open until
/// ticked; it now names the item.
pub fn promote(
    app: &Arc<AppState>,
    party: &Party<'_>,
    project_id: &str,
    action_id: &str,
    title: Option<&str>,
) -> anyhow::Result<ActionItem> {
    party.require_lead("promote an action item")?;
    if app.db.board_settings(project_id)?.is_none() {
        return Err(conflict(
            "this project has no board here; promote it on the board's home computer",
        ));
    }
    let mut action = own_action(app, project_id, action_id)?;
    if let Some(item) = &action.item_id {
        return Err(conflict(format!("already promoted to {item}")));
    }
    if action.status != ActionStatus::Open {
        return Err(conflict(format!(
            "the action is {}",
            action.status.as_str()
        )));
    }
    let title = title
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .unwrap_or(&action.text)
        .to_string();
    if let Some(u) = guards::check_new(&title).into_iter().next() {
        return Err(invalid(u.text));
    }
    let meeting = load(app, project_id, &action.meeting_id)?;
    let actor = party.actor();
    let mut feed = app.board.writer();
    let item = app.db.board_tx(|t| {
        let item = t.create_item(
            &NewItem {
                project_id,
                item_type: ItemType::Chore,
                title: &title,
                description: "",
                platforms: &[],
                size: None,
                priority: Priority::P2,
                labels: &[],
                parent_id: None,
                acceptance_criteria: &[],
            },
            &actor,
        )?;
        t.add_item_link(
            &item.id,
            LinkKind::Meeting,
            &meeting.id,
            Some(&meeting.name),
            &actor,
        )?;
        action.item_id = Some(item.id.clone());
        t.save_action(&action)?;
        Ok(item)
    })?;
    // Committed: from here on nothing may fail, or the push is lost.
    feed.publish(Change {
        project_id,
        kind: ChangeKind::ItemUpserted,
        item_id: &item.id,
        card: card_after_commit(&app.db, &item.id),
        from_column: None,
    });
    changed(app, project_id, Some(&meeting.id));
    own_action(app, project_id, action_id)
}
