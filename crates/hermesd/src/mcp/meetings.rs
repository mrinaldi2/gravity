//! The meeting tools (H-020 §1.6, §4): every bot reads meetings, attendees
//! contribute and owners tick their actions; the facilitator starts and
//! closes a meeting and records its actions; the lead sets up series and
//! promotes actions to the board. The service checks who may act on which
//! meeting, so a facilitator needs no board role.

use std::sync::Arc;

use bus::contract::board as c;
use serde_json::{json, Map, Value};

use crate::app::AppState;
use crate::board::meetings::actions::{self, ActionEdit, NewAction};
use crate::board::meetings::model::MeetingType;
use crate::board::meetings::run::{self, AdHoc, Close, Contribute};
use crate::board::meetings::series::{self, SeriesEdit};
use crate::board::meetings::{self as meetings, Party};
use crate::board::model::Role;
use crate::decisions::invalid;

use super::board_schema::{decode, shared, tool, Audience, BoardTool};

pub(super) const MEETING_TOOLS: &[BoardTool] = &[
    tool("meeting_list", "MeetingList", Audience::Everyone),
    tool("meeting_get", "MeetingGet", Audience::Everyone),
    shared(
        "meeting_start",
        "MeetingStart",
        Audience::Everyone,
        "Facilitator (or lead): start a meeting of a series (series_id), or the lead an \
         ad-hoc one (name, attendees). Freezes the board as its inputs and sends each \
         attendee one note asking for a contribution.",
    ),
    tool(
        "meeting_contribute",
        "MeetingContribute",
        Audience::Everyone,
    ),
    tool("meeting_close", "MeetingClose", Audience::Everyone),
    tool("action_add", "ActionAdd", Audience::Everyone),
    tool("action_update", "ActionUpdate", Audience::Everyone),
    tool("action_promote", "ActionPromote", Audience::Lead),
    tool(
        "meeting_series_upsert",
        "MeetingSeriesUpsert",
        Audience::Lead,
    ),
];

pub(super) fn handles(name: &str) -> bool {
    MEETING_TOOLS.iter().any(|t| t.name == name)
}

/// A meeting type argument, when given.
pub(crate) fn meeting_type(text: Option<&str>) -> anyhow::Result<Option<MeetingType>> {
    text.map(str::trim)
        .filter(|t| !t.is_empty())
        .map(|t| MeetingType::parse(t).ok_or_else(|| invalid(format!("unknown meeting type {t}"))))
        .transpose()
}

pub(super) fn call(
    app: &Arc<AppState>,
    bot: &bus::Bot,
    roles: Vec<Role>,
    name: &str,
    args: &Value,
) -> anyhow::Result<Value> {
    let party = Party::Bot { bot, roles };
    let project = bot.project_id.as_str();
    let meeting = |m: meetings::model::Meeting| -> anyhow::Result<Value> {
        Ok(json!({ "meeting": meetings::get(app, project, &m.id)? }))
    };
    let action = |a: meetings::model::ActionItem| Ok(json!({ "action": a.to_json() }));
    match name {
        "meeting_list" => {
            let req: c::MeetingList = decode("MeetingList", args, project)?;
            let of_type = meeting_type(req.r#type.as_deref())?;
            meetings::list(app, project, of_type, req.upcoming.unwrap_or(false))
        }
        "meeting_get" => {
            let req: c::MeetingGet = decode("MeetingGet", args, project)?;
            Ok(json!({ "meeting": meetings::get(app, project, &req.meeting_id)? }))
        }
        "meeting_start" => {
            let req: c::MeetingStart = decode("MeetingStart", args, project)?;
            let adhoc = AdHoc {
                name: req.name.as_deref().unwrap_or_default(),
                attendees: &req.attendees,
            };
            meeting(run::start(
                app,
                &party,
                project,
                req.series_id.as_deref(),
                &adhoc,
            )?)
        }
        "meeting_contribute" => {
            let req: c::MeetingContribute = decode("MeetingContribute", args, project)?;
            let c = Contribute {
                section: &req.section,
                body: &req.body,
                item_refs: &req.item_refs,
            };
            meeting(run::contribute(app, &party, project, &req.meeting_id, &c)?)
        }
        "meeting_close" => {
            let req: c::MeetingClose = decode("MeetingClose", args, project)?;
            let close = Close {
                outputs: outputs(req.outputs),
                summary: req.summary.as_deref().unwrap_or_default(),
                skip_reason: req.skip_reason.as_deref(),
            };
            meeting(run::close(app, &party, project, &req.meeting_id, &close)?)
        }
        "action_add" => {
            let req: c::ActionAdd = decode("ActionAdd", args, project)?;
            let new = NewAction {
                text: &req.text,
                owner: &req.owner,
                due_at: req.due_at.as_deref(),
            };
            action(actions::add(app, &party, project, &req.meeting_id, &new)?)
        }
        "action_update" => {
            let req: c::ActionUpdate = decode("ActionUpdate", args, project)?;
            let edit = ActionEdit {
                status: req.status.as_deref(),
                text: req.text.as_deref(),
                due_at: req.due_at.as_deref(),
            };
            action(actions::update(
                app,
                &party,
                project,
                &req.action_id,
                &edit,
            )?)
        }
        "action_promote" => {
            let req: c::ActionPromote = decode("ActionPromote", args, project)?;
            action(actions::promote(
                app,
                &party,
                project,
                &req.action_id,
                req.title.as_deref(),
            )?)
        }
        "meeting_series_upsert" => {
            let req: c::MeetingSeriesUpsert = decode("MeetingSeriesUpsert", args, project)?;
            let edit = SeriesEdit {
                series_id: req.series_id.as_deref(),
                meeting_type: req.r#type.as_deref(),
                name: req.name.as_deref(),
                cron: req.cron.as_deref(),
                tz: req.tz.as_deref(),
                facilitator: req.facilitator.as_deref(),
                attendees: &req.attendees,
                input_scope: req.input_scope.as_deref(),
                enabled: req.enabled,
            };
            let s = series::upsert(app, &party, project, &edit)?;
            Ok(json!({ "series": s.to_json() }))
        }
        other => anyhow::bail!("unknown tool: {other}"),
    }
}

/// A close's outputs, by section, as stored.
pub(crate) fn outputs(
    by_section: impl IntoIterator<Item = (String, String)>,
) -> Map<String, Value> {
    by_section
        .into_iter()
        .filter(|(_, body)| !body.trim().is_empty())
        .map(|(section, body)| (section, Value::String(body)))
        .collect()
}
