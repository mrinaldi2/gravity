//! Meetings (H-017 §1.5, H-020 §4; H-102). A series is a recurring meeting
//! whose routine wakes its facilitator on the cron; the facilitator starts
//! an occurrence (`run`), attendees contribute, and the facilitator closes
//! it with outputs, a summary and action items (`actions`). Open actions
//! carry over to the series' next meeting and show on the dashboard; one
//! that needs real work is promoted to a chore on the board.
//!
//! Bots act over MCP, the owner over the WebSocket; both reach here as a
//! `Party`. Meetings live with the board, on its home.

use chrono::{DateTime, Utc};
use serde_json::{json, Value};

use crate::actor::Actor;
use crate::app::AppState;
use crate::board::model::Role;
use crate::decisions::{forbidden, invalid, not_found};

use model::{ActionItem, Meeting, MeetingStatus, MeetingType, Series, OWNER};

pub mod actions;
pub mod model;
pub mod run;
pub mod series;
#[cfg(test)]
mod tests;

/// Meetings a list or the dashboard shows.
const RECENT: usize = 20;

/// Who is acting: a bot with its board roles, or the owner.
pub enum Party<'a> {
    Bot { bot: &'a bus::Bot, roles: Vec<Role> },
    Owner(Actor<'a>),
}

impl Party<'_> {
    /// How meetings store this party: a bot id, or `owner`.
    pub fn who(&self) -> &str {
        match self {
            Party::Bot { bot, .. } => &bot.id,
            Party::Owner(_) => OWNER,
        }
    }

    pub fn actor(&self) -> Actor<'_> {
        match self {
            Party::Bot { bot, .. } => Actor::Bot {
                id: &bot.id,
                project_id: &bot.project_id,
            },
            Party::Owner(actor) => *actor,
        }
    }

    /// The owner or the project's lead.
    pub fn leads(&self) -> bool {
        match self {
            Party::Bot { roles, .. } => roles.contains(&Role::Lead),
            Party::Owner(_) => true,
        }
    }

    /// Refused unless this party leads or runs the meeting.
    fn runs(&self, m: &Meeting, what: &str) -> anyhow::Result<()> {
        if self.leads() || self.who() == m.facilitator {
            Ok(())
        } else {
            Err(forbidden(format!(
                "only {m_f} (the facilitator) or the lead can {what}",
                m_f = m.facilitator
            )))
        }
    }

    fn require_lead(&self, what: &str) -> anyhow::Result<()> {
        if self.leads() {
            Ok(())
        } else {
            Err(forbidden(format!("only the lead or the owner can {what}")))
        }
    }
}

/// A bot of the project by id or name, or `owner`: stored as an id.
pub fn resolve_who(app: &AppState, project_id: &str, who: &str) -> anyhow::Result<String> {
    let who = who.trim();
    if who.eq_ignore_ascii_case(OWNER) {
        return Ok(OWNER.to_string());
    }
    if let Some(bot) = app.db.get_bot(who)? {
        if bot.project_id == project_id {
            return Ok(bot.id);
        }
    }
    match app.db.get_bot_by_name(project_id, who)? {
        Some(bot) => Ok(bot.id),
        None => Err(invalid(format!("no bot {who} in this project"))),
    }
}

/// A meeting of the project; another project's meetings don't exist here.
pub fn load(app: &AppState, project_id: &str, id: &str) -> anyhow::Result<Meeting> {
    match app.db.board_read(|t| t.meeting(id))? {
        Some(m) if m.project_id == project_id => Ok(m),
        _ => Err(not_found(format!("no meeting {id} in this project"))),
    }
}

/// One meeting in full, with the open actions it carries over.
pub fn get(app: &AppState, project_id: &str, id: &str) -> anyhow::Result<Value> {
    let m = load(app, project_id, id)?;
    let carried = app.db.board_read(|t| t.carried_over(&m))?;
    Ok(m.to_json(&carried))
}

/// The series' next time, from its cron, while it is enabled.
pub fn next_at(s: &Series, from: DateTime<Utc>) -> Option<DateTime<Utc>> {
    if !s.enabled {
        return None;
    }
    let trigger = bus::Trigger::Cron {
        expr: s.cron.clone(),
        tz: s.tz.clone(),
    };
    crate::scheduler::next_occurrence(&trigger, from, from)
}

/// `meeting_list`: the series with their next time, and the recent
/// meetings unless only upcoming ones are asked for.
pub fn list(
    app: &AppState,
    project_id: &str,
    of_type: Option<MeetingType>,
    upcoming: bool,
) -> anyhow::Result<Value> {
    let now = Utc::now();
    let (series, meetings) = app.db.board_read(|t| {
        Ok((
            t.series_list(project_id)?,
            if upcoming {
                Vec::new()
            } else {
                t.meetings(project_id, RECENT)?
            },
        ))
    })?;
    let fits = |t: MeetingType| of_type.is_none_or(|want| want == t);
    Ok(json!({
        "series": series.iter().filter(|s| fits(s.meeting_type)).map(|s| {
            let mut out = s.to_json();
            out["next_at"] = json!(next_at(s, now));
            out
        }).collect::<Vec<_>>(),
        "meetings": meetings.iter().filter(|m| fits(m.meeting_type))
            .map(Meeting::summary_json).collect::<Vec<_>>(),
    }))
}

/// The dashboard's widgets 5 and 6 (H-018 §2.1): per series its next time,
/// the meeting collecting now and the last one held; then ad-hoc meetings
/// still collecting; and every open action item, soonest due first.
pub fn dashboard(app: &AppState, project_id: &str) -> anyhow::Result<(Value, Value)> {
    let now = Utc::now();
    let (series, meetings, actions) = app.db.board_read(|t| {
        Ok((
            t.series_list(project_id)?,
            t.meetings(project_id, RECENT)?,
            t.open_actions(project_id)?,
        ))
    })?;
    let of = |s: &Series, status: MeetingStatus| {
        meetings
            .iter()
            .find(|m| m.series_id.as_deref() == Some(s.id.as_str()) && m.status == status)
            .map(Meeting::summary_json)
    };
    let mut rows: Vec<Value> = series
        .iter()
        .map(|s| {
            json!({
                "series": s.to_json(), "next_at": next_at(s, now),
                "collecting": of(s, MeetingStatus::Collecting),
                "last_held": of(s, MeetingStatus::Held),
            })
        })
        .collect();
    rows.extend(
        meetings
            .iter()
            .filter(|m| m.series_id.is_none() && m.status == MeetingStatus::Collecting)
            .map(|m| json!({ "series": null, "next_at": null, "collecting": m.summary_json(), "last_held": null })),
    );
    let names: std::collections::HashMap<&str, &str> = meetings
        .iter()
        .map(|m| (m.id.as_str(), m.name.as_str()))
        .collect();
    let actions = actions
        .iter()
        .map(|a| action_row(a, names.get(a.meeting_id.as_str()).copied(), now))
        .collect();
    Ok((json!(rows), Value::Array(actions)))
}

fn action_row(a: &ActionItem, meeting_name: Option<&str>, now: DateTime<Utc>) -> Value {
    let mut out = a.to_json();
    out["meeting_name"] = json!(meeting_name);
    out["overdue"] = json!(a.due_at.is_some_and(|due| due < now));
    out
}
