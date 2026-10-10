//! Frames exchanged on a peer link. Ids in a frame are the sender's own,
//! except `to_bot_id` and `closes_task.id`, which the sender has already
//! translated into the receiver's ids.

use bus::{MessageKind, RemoteBot, TaskState};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// One message crossing the link, with whatever it opens or closes.
#[derive(Debug, Serialize, Deserialize)]
pub struct MessageFrame {
    /// The message's id on the sending daemon. The receiver dedupes on it.
    pub id: String,
    pub to_bot_id: String,
    pub from: FromFrame,
    pub kind: MessageKind,
    pub body: String,
    /// The message this answers, in whichever id the receiver knows it by.
    #[serde(rename = "ref", default, skip_serializing_if = "Option::is_none")]
    pub ref_message_id: Option<String>,
    /// Set when the message opens a task: the receiver mirrors it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task: Option<TaskFrame>,
    /// Set when the message is the result or the cancellation of a task the
    /// receiver holds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub closes_task: Option<ClosesFrame>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub artifacts: Vec<ArtifactFrame>,
    /// Set on the owner's chat when the sender recorded it as the owner's own
    /// (H-195 D1): how the owner proved it there, `device` or `ticket`. A
    /// peer older than that never sends it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_verified: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FromFrame {
    /// The owner, writing from the other machine's app.
    User,
    Bot {
        bot: RemoteBot,
    },
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TaskFrame {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deadline_at: Option<DateTime<Utc>>,
    pub hop_count: i64,
    /// The board card the task is for, by the board home's id (H-125 S1b).
    /// An older peer sends none: the task is then "card unknown", and the
    /// receiver allows nested sends under it rather than refusing them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub item_id: Option<String>,
    /// The release a deploy or rollback task is for, by the release home's
    /// id (H-158): the receiver counts the task as on the board, since the
    /// release accounts for it. An older peer sends none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub release_id: Option<String>,
    /// Where that deploy or rollback installs, by the release home's name for
    /// it (H-288): the receiver flags an install as pending only when it is
    /// this computer, never the iPhone. An older peer sends none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ClosesFrame {
    pub id: String,
    pub state: TaskState,
}

/// A file attached to a result. `bytes` is base64; it is absent when the file
/// could not be sent, and `skipped` says why.
#[derive(Debug, Serialize, Deserialize)]
pub struct ArtifactFrame {
    /// The path as the sender's result listed it.
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bytes: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skipped: Option<String>,
}

/// What the receiver answers a message frame with.
#[derive(Debug, Serialize, Deserialize)]
pub struct Received {
    pub message_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
}
