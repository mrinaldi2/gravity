//! Holding a meeting (H-020 §4): start freezes the inputs and sends each
//! attendee one note; attendees contribute while it collects; the
//! facilitator closes it as held, with outputs and a summary, or skipped.

use std::sync::Arc;

use chrono::Utc;
use serde_json::{json, Map, Value};

use crate::app::AppState;
use crate::board::model::ColumnCategory;
use crate::decisions::{conflict, forbidden, invalid, not_found};
use crate::events::Push;
use crate::messaging::{self, user_sender, Dm};

use super::model::{Attendee, Contribution, Meeting, MeetingStatus, MeetingType, OWNER};
use super::{load, resolve_who, Party};

/// The facilitator's summary, at most (H-017 §1.5).
pub const SUMMARY_LINES: usize = 10;

/// An ad-hoc meeting: what `meeting_start` without a series holds.
pub struct AdHoc<'a> {
    pub name: &'a str,
    pub attendees: &'a [String],
}

/// Tell open clients a project's meetings changed.
pub(super) fn changed(app: &AppState, project_id: &str, meeting_id: Option<&str>) {
    app.events.push(Push::MeetingEvent {
        project_id: project_id.to_string(),
        meeting_id: meeting_id.map(str::to_string),
    });
}

/// Start a series' meeting (its facilitator, or the lead) or an ad-hoc one
/// (the lead or the owner, who then facilitates it).
pub fn start(
    app: &Arc<AppState>,
    party: &Party<'_>,
    project_id: &str,
    series_id: Option<&str>,
    adhoc: &AdHoc<'_>,
) -> anyhow::Result<Meeting> {
    let (series_id, meeting_type, name, facilitator, attendees) = match series_id {
        Some(id) => {
            let series = app
                .db
                .board_read(|t| t.series(id))?
                .filter(|s| s.project_id == project_id)
                .ok_or_else(|| not_found(format!("no meeting series {id} in this project")))?;
            if !party.leads() && party.who() != series.facilitator {
                return Err(forbidden(
                    "only the series' facilitator or the lead can start its meeting",
                ));
            }
            if let Some(open) = app.db.board_read(|t| t.collecting_in(&series.id))? {
                return Err(conflict(format!(
                    "{open} is still collecting; close it (or skip it) before starting another"
                )));
            }
            (
                Some(series.id),
                series.meeting_type,
                series.name,
                series.facilitator,
                series.attendees,
            )
        }
        None => {
            party.require_lead("start an ad-hoc meeting")?;
            let name = adhoc.name.trim();
            if name.is_empty() {
                return Err(invalid(
                    "an ad-hoc meeting needs a name (or pass series_id)",
                ));
            }
            let mut ids = Vec::new();
            for who in adhoc.attendees {
                let id = resolve_who(app, project_id, who)?;
                if !ids.contains(&id) {
                    ids.push(id);
                }
            }
            (
                None,
                MeetingType::Adhoc,
                name.to_string(),
                party.who().to_string(),
                ids,
            )
        }
    };
    let now = Utc::now();
    let mut meeting = Meeting {
        id: String::new(),
        project_id: project_id.to_string(),
        series_id,
        meeting_type,
        name,
        scheduled_at: Some(now),
        started_at: Some(now),
        closed_at: None,
        status: MeetingStatus::Collecting,
        skip_reason: None,
        facilitator,
        inputs_snapshot: inputs(app, project_id)?,
        outputs: json!({}),
        summary: String::new(),
        attendees: attendees
            .iter()
            .map(|who| Attendee {
                who: who.clone(),
                required: true,
                contributed_at: None,
            })
            .collect(),
        contributions: Vec::new(),
        actions: Vec::new(),
    };
    app.db.board_tx(|t| {
        let base = format!("MTG-{}-{}", now.format("%Y-%m-%d"), meeting_type.as_str());
        meeting.id = base.clone();
        let mut n = 1;
        while t.meeting_id_taken(&meeting.id)? {
            n += 1;
            meeting.id = format!("{base}-{n}");
        }
        t.insert_meeting(&meeting)
    })?;
    ask_attendees(app, party, &meeting)?;
    changed(app, project_id, Some(&meeting.id));
    load(app, project_id, &meeting.id)
}

/// What the attendees work from, frozen at the start: the board's columns
/// with their counts, what is blocked or stale, who is doing what, and the
/// open action items.
fn inputs(app: &AppState, project_id: &str) -> anyhow::Result<Value> {
    let db = &app.db;
    let board = if db.board_settings(project_id)?.is_some() {
        let columns = db.board_columns(project_id)?;
        let cards = db.board_cards(project_id)?;
        let card = |c: &crate::board::model::ItemCard| json!({ "id": c.id, "title": c.title, "column": c.column_key, "assignee": c.assignee });
        let in_category = |cat: ColumnCategory| {
            let keys: Vec<&str> = columns
                .iter()
                .filter(|c| c.category == cat)
                .map(|c| c.key.as_str())
                .collect();
            cards
                .iter()
                .filter(|c| keys.contains(&c.column_key.as_str()))
                .map(card)
                .collect::<Vec<_>>()
        };
        json!({
            "columns": columns.iter().map(|c| json!({
                "key": c.key, "name": c.name,
                "count": cards.iter().filter(|card| card.column_key == c.key).count(),
            })).collect::<Vec<_>>(),
            "doing": in_category(ColumnCategory::Doing),
            "blocked": cards.iter().filter(|c| c.blocked).map(card).collect::<Vec<_>>(),
            "stale": cards.iter().filter(|c| c.stale).map(card).collect::<Vec<_>>(),
        })
    } else {
        Value::Null
    };
    let open = db.board_read(|t| t.open_actions(project_id))?;
    Ok(json!({
        "as_of": Utc::now(),
        "board": board,
        "open_actions": open.iter().map(|a| a.to_json()).collect::<Vec<_>>(),
    }))
}

/// One note per attendee bot; the facilitator and the owner aren't sent one.
fn ask_attendees(app: &AppState, party: &Party<'_>, m: &Meeting) -> anyhow::Result<()> {
    let sender = match party {
        Party::Bot { bot, .. } => crate::mcp::bot_sender(bot),
        Party::Owner(_) => user_sender(),
    };
    let note = format!(
        "Meeting {id} (\"{name}\") has started. Read its inputs with meeting_get \
         {{\"meeting_id\": \"{id}\"}}, then add yours with meeting_contribute (one call per \
         section, naming the items with item_refs). {facilitator} closes it.",
        id = m.id,
        name = m.name,
        facilitator = facilitator_name(app, &m.facilitator),
    );
    for a in &m.attendees {
        if a.who == OWNER || a.who == m.facilitator {
            continue;
        }
        messaging::send_dm(
            &app.db,
            &app.events,
            Dm::new(&a.who, &sender, bus::MessageKind::Note, &note),
        )?;
    }
    Ok(())
}

fn facilitator_name(app: &AppState, who: &str) -> String {
    if who == OWNER {
        return "The owner".to_string();
    }
    app.db
        .get_bot(who)
        .ok()
        .flatten()
        .map_or_else(|| who.to_string(), |b| b.name)
}

/// What one contribution adds.
pub struct Contribute<'a> {
    pub section: &'a str,
    pub body: &'a str,
    pub item_refs: &'a [String],
}

/// An attendee's (or the facilitator's) contribution to a meeting that is
/// collecting.
pub fn contribute(
    app: &Arc<AppState>,
    party: &Party<'_>,
    project_id: &str,
    meeting_id: &str,
    c: &Contribute<'_>,
) -> anyhow::Result<Meeting> {
    let m = load(app, project_id, meeting_id)?;
    if m.status != MeetingStatus::Collecting {
        return Err(conflict(format!(
            "{} is {}; contributions are taken while it is collecting",
            m.id,
            m.status.as_str()
        )));
    }
    let who = party.who();
    if who != m.facilitator && !m.attendees.iter().any(|a| a.who == who) {
        return Err(forbidden(format!("you aren't an attendee of {}", m.id)));
    }
    let (section, body) = (c.section.trim(), c.body.trim());
    if section.is_empty() || body.is_empty() {
        return Err(invalid("a contribution needs a section and a body"));
    }
    for item in c.item_refs {
        if app.db.item_project(item)?.as_deref() != Some(project_id) {
            return Err(invalid(format!("no item {item} in this project")));
        }
    }
    let contribution = Contribution {
        id: bus::new_id(),
        author: who.to_string(),
        section: section.to_string(),
        body: body.to_string(),
        item_refs: c.item_refs.to_vec(),
        at: Utc::now(),
    };
    app.db
        .board_tx(|t| t.add_contribution(&m.id, &contribution))?;
    changed(app, project_id, Some(&m.id));
    load(app, project_id, meeting_id)
}

/// How a meeting ends.
pub struct Close<'a> {
    pub outputs: Map<String, Value>,
    pub summary: &'a str,
    pub skip_reason: Option<&'a str>,
}

/// The facilitator (or the lead) closes a meeting: held, with a summary of
/// at most ten lines, or skipped with a reason.
pub fn close(
    app: &Arc<AppState>,
    party: &Party<'_>,
    project_id: &str,
    meeting_id: &str,
    c: &Close<'_>,
) -> anyhow::Result<Meeting> {
    let m = load(app, project_id, meeting_id)?;
    party.runs(&m, "close it")?;
    if !matches!(
        m.status,
        MeetingStatus::Collecting | MeetingStatus::Scheduled
    ) {
        return Err(conflict(format!(
            "{} is already {}",
            m.id,
            m.status.as_str()
        )));
    }
    let summary = c.summary.trim();
    let skip = c.skip_reason.map(str::trim).filter(|r| !r.is_empty());
    let status = if skip.is_some() {
        MeetingStatus::Skipped
    } else {
        if summary.is_empty() {
            return Err(invalid(
                "a held meeting needs a summary (or pass skip_reason)",
            ));
        }
        MeetingStatus::Held
    };
    if summary.lines().count() > SUMMARY_LINES {
        return Err(invalid(format!(
            "the summary has {} lines; keep it to {SUMMARY_LINES}",
            summary.lines().count()
        )));
    }
    let outputs = Value::Object(c.outputs.clone());
    app.db
        .board_tx(|t| t.close_meeting(&m.id, status, &outputs, summary, skip))?;
    changed(app, project_id, Some(&m.id));
    load(app, project_id, meeting_id)
}
