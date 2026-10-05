//! Meetings over the WebSocket (H-020 §1.5; H-102): the owner reads them,
//! sets up series, contributes as an attendee, and ticks, drops or promotes
//! action items from the dashboard. Fields are the request messages' own
//! (meetings.proto), except a meeting's `type`, sent as `meeting_type` since
//! `type` names the request. Meetings live on the board's home; off-home
//! they are refused with where to go.

use serde_json::{json, Value};

use crate::board::meetings::actions::{self, ActionEdit};
use crate::board::meetings::run::{self, Contribute};
use crate::board::meetings::series::{self, SeriesEdit};
use crate::board::meetings::{self as meetings, Party};
use crate::decisions::{forbidden, invalid};
use crate::mcp::meetings::meeting_type;

use super::Conn;

fn opt<'a>(req: &'a Value, field: &str) -> Option<&'a str> {
    req.get(field).and_then(Value::as_str)
}

fn strings(req: &Value, field: &str) -> Vec<String> {
    req.get(field)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|v| v.as_str().map(str::to_string))
        .collect()
}

impl Conn {
    /// The project a meeting request is about, when its board lives here.
    fn meetings_home(&self, project_id: &str) -> anyhow::Result<()> {
        match self.app.board_mirror.home_peer(project_id) {
            Some(home) => Err(forbidden(format!(
                "This project's meetings are kept on {}, which holds its board.",
                crate::peer::board::home_name(&self.app, &home)
            ))),
            None => Ok(()),
        }
    }

    fn meeting_project<'a>(&self, req: &'a Value) -> anyhow::Result<&'a str> {
        let project_id = Self::str_field(req, "project_id")?;
        self.meetings_home(project_id)?;
        Ok(project_id)
    }

    pub(super) fn meeting_list(&self, req_id: &Value, req: &Value) -> anyhow::Result<()> {
        let project_id = self.meeting_project(req)?;
        let of_type = meeting_type(opt(req, "meeting_type"))?;
        let upcoming = req
            .get("upcoming")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let mut out = meetings::list(&self.app, project_id, of_type, upcoming)?;
        out["type"] = json!("meetings");
        out["req_id"] = req_id.clone();
        self.send(out);
        Ok(())
    }

    pub(super) fn meeting_get(&self, req_id: &Value, req: &Value) -> anyhow::Result<()> {
        let project_id = self.meeting_project(req)?;
        let id = Self::str_field(req, "meeting_id")?;
        let meeting = meetings::get(&self.app, project_id, id)?;
        self.send(json!({ "type": "meeting", "req_id": req_id, "meeting": meeting }));
        Ok(())
    }

    pub(super) fn meeting_series_upsert(&self, req_id: &Value, req: &Value) -> anyhow::Result<()> {
        let project_id = self.meeting_project(req)?;
        let attendees = strings(req, "attendees");
        let edit = SeriesEdit {
            series_id: opt(req, "series_id"),
            meeting_type: opt(req, "meeting_type"),
            name: opt(req, "name"),
            cron: opt(req, "cron"),
            tz: opt(req, "tz"),
            facilitator: opt(req, "facilitator"),
            attendees: &attendees,
            input_scope: opt(req, "input_scope"),
            enabled: req.get("enabled").and_then(Value::as_bool),
        };
        let party = Party::Owner(self.actor());
        let s = series::upsert(&self.app, &party, project_id, &edit)?;
        self.send(json!({ "type": "meeting_series", "req_id": req_id, "series": s.to_json() }));
        Ok(())
    }

    /// The owner as an attendee.
    pub(super) fn meeting_contribute(&self, req_id: &Value, req: &Value) -> anyhow::Result<()> {
        let project_id = self.meeting_project(req)?;
        let id = Self::str_field(req, "meeting_id")?;
        let item_refs = strings(req, "item_refs");
        let c = Contribute {
            section: Self::str_field(req, "section")?,
            body: Self::str_field(req, "body")?,
            item_refs: &item_refs,
        };
        let party = Party::Owner(self.actor());
        run::contribute(&self.app, &party, project_id, id, &c)?;
        let meeting = meetings::get(&self.app, project_id, id)?;
        self.send(json!({ "type": "meeting", "req_id": req_id, "meeting": meeting }));
        Ok(())
    }

    /// Tick, drop, reopen or reword an action: `status`, `text`, `due_at`.
    pub(super) fn action_update(&self, req_id: &Value, req: &Value) -> anyhow::Result<()> {
        let project_id = self.meeting_project(req)?;
        let id = Self::str_field(req, "action_id")?;
        let edit = ActionEdit {
            status: opt(req, "status"),
            text: opt(req, "text"),
            due_at: opt(req, "due_at"),
        };
        if edit.status.is_none() && edit.text.is_none() && edit.due_at.is_none() {
            return Err(invalid("nothing to change: pass status, text or due_at"));
        }
        let party = Party::Owner(self.actor());
        let action = actions::update(&self.app, &party, project_id, id, &edit)?;
        self.send(
            json!({ "type": "meeting_action", "req_id": req_id, "action": action.to_json() }),
        );
        Ok(())
    }

    /// Turn an action into a chore on the board, linked to its meeting.
    pub(super) fn action_promote(&self, req_id: &Value, req: &Value) -> anyhow::Result<()> {
        let project_id = self.meeting_project(req)?;
        let id = Self::str_field(req, "action_id")?;
        let party = Party::Owner(self.actor());
        let action = actions::promote(&self.app, &party, project_id, id, opt(req, "title"))?;
        self.send(
            json!({ "type": "meeting_action", "req_id": req_id, "action": action.to_json() }),
        );
        Ok(())
    }
}
