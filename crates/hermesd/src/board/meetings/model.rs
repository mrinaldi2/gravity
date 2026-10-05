//! Meetings' own types (H-017 §1.5): what `db::meetings` stores and the
//! service works with. Bots and the WebSocket both read them as JSON.

use chrono::{DateTime, Utc};
use serde_json::{json, Value};

use crate::board::model::{text_enum, TextValue};

/// An attendee or action owner who is the owner rather than a bot.
pub const OWNER: &str = "owner";

text_enum!(MeetingType {
    Standup => "standup", Refinement => "refinement", Demo => "demo", Retro => "retro",
    Adhoc => "adhoc",
});
text_enum!(MeetingStatus {
    Scheduled => "scheduled", Collecting => "collecting", Held => "held", Skipped => "skipped",
});
text_enum!(ActionStatus { Open => "open", Done => "done", Dropped => "dropped" });

#[derive(Debug, Clone)]
pub struct Series {
    pub id: String,
    pub project_id: String,
    pub meeting_type: MeetingType,
    pub name: String,
    pub cron: String,
    pub tz: String,
    pub facilitator: String,
    pub attendees: Vec<String>,
    pub input_scope: String,
    pub enabled: bool,
    pub routine_id: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct Attendee {
    pub who: String,
    pub required: bool,
    pub contributed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub struct Contribution {
    pub id: String,
    pub author: String,
    pub section: String,
    pub body: String,
    pub item_refs: Vec<String>,
    pub at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct ActionItem {
    pub id: String,
    pub project_id: String,
    pub meeting_id: String,
    pub series_id: Option<String>,
    pub text: String,
    pub owner: String,
    pub due_at: Option<DateTime<Utc>>,
    pub status: ActionStatus,
    pub item_id: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct Meeting {
    pub id: String,
    pub project_id: String,
    pub series_id: Option<String>,
    pub meeting_type: MeetingType,
    pub name: String,
    pub scheduled_at: Option<DateTime<Utc>>,
    pub started_at: Option<DateTime<Utc>>,
    pub closed_at: Option<DateTime<Utc>>,
    pub status: MeetingStatus,
    pub skip_reason: Option<String>,
    pub facilitator: String,
    pub inputs_snapshot: Value,
    pub outputs: Value,
    pub summary: String,
    /// Filled by a full read; empty in lists.
    pub attendees: Vec<Attendee>,
    pub contributions: Vec<Contribution>,
    pub actions: Vec<ActionItem>,
}

impl Series {
    pub fn to_json(&self) -> Value {
        json!({
            "id": self.id, "type": self.meeting_type.as_str(), "name": self.name,
            "cron": self.cron, "tz": self.tz, "facilitator": self.facilitator,
            "attendees": self.attendees, "input_scope": self.input_scope,
            "enabled": self.enabled, "routine_id": self.routine_id,
            "created_at": self.created_at, "updated_at": self.updated_at,
        })
    }
}

impl ActionItem {
    pub fn to_json(&self) -> Value {
        json!({
            "id": self.id, "meeting_id": self.meeting_id, "series_id": self.series_id,
            "text": self.text, "owner": self.owner, "due_at": self.due_at,
            "status": self.status.as_str(), "item_id": self.item_id,
            "created_at": self.created_at, "updated_at": self.updated_at,
        })
    }
}

impl Meeting {
    /// How many attendees have contributed, of how many.
    pub fn contributed(&self) -> (usize, usize) {
        let done = self
            .attendees
            .iter()
            .filter(|a| a.contributed_at.is_some())
            .count();
        (done, self.attendees.len())
    }

    /// The list form: no contributions, actions or inputs.
    pub fn summary_json(&self) -> Value {
        let (contributed, attendees) = self.contributed();
        json!({
            "id": self.id, "series_id": self.series_id, "type": self.meeting_type.as_str(),
            "name": self.name, "status": self.status.as_str(), "skip_reason": self.skip_reason,
            "scheduled_at": self.scheduled_at, "started_at": self.started_at,
            "closed_at": self.closed_at, "facilitator": self.facilitator,
            "summary": self.summary, "outputs": self.outputs,
            "contributed": contributed, "attendee_count": attendees,
        })
    }

    pub fn to_json(&self, carried_over: &[ActionItem]) -> Value {
        let mut out = self.summary_json();
        out["attendees"] = self
            .attendees
            .iter()
            .map(|a| json!({ "who": a.who, "required": a.required, "contributed_at": a.contributed_at }))
            .collect();
        out["contributions"] = self
            .contributions
            .iter()
            .map(|c| {
                json!({
                    "id": c.id, "author": c.author, "section": c.section, "body": c.body,
                    "item_refs": c.item_refs, "at": c.at,
                })
            })
            .collect();
        out["inputs_snapshot"] = self.inputs_snapshot.clone();
        out["action_items"] = self.actions.iter().map(ActionItem::to_json).collect();
        out["carried_over"] = carried_over.iter().map(ActionItem::to_json).collect();
        out
    }
}
