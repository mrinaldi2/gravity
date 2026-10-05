//! An owner action's own types (H-117 R1).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

/// How the content runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Shell {
    Zsh,
    Bash,
    Powershell,
    Cmd,
}

impl Shell {
    pub const ALL: [Shell; 4] = [Shell::Zsh, Shell::Bash, Shell::Powershell, Shell::Cmd];

    pub fn as_str(self) -> &'static str {
        match self {
            Shell::Zsh => "zsh",
            Shell::Bash => "bash",
            Shell::Powershell => "powershell",
            Shell::Cmd => "cmd",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|s| s.as_str() == text)
    }

    /// This computer's default.
    pub fn here() -> Self {
        if cfg!(windows) {
            Shell::Powershell
        } else {
            Shell::Zsh
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    Proposed,
    Running,
    Succeeded,
    Failed,
    TimedOut,
    Rejected,
    Withdrawn,
    Expired,
}

impl State {
    pub const ALL: [State; 8] = [
        State::Proposed,
        State::Running,
        State::Succeeded,
        State::Failed,
        State::TimedOut,
        State::Rejected,
        State::Withdrawn,
        State::Expired,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            State::Proposed => "proposed",
            State::Running => "running",
            State::Succeeded => "succeeded",
            State::Failed => "failed",
            State::TimedOut => "timed_out",
            State::Rejected => "rejected",
            State::Withdrawn => "withdrawn",
            State::Expired => "expired",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|s| s.as_str() == text)
    }

    pub fn is_open(self) -> bool {
        self == State::Proposed
    }
}

/// A file the content relies on, hashed at proposal and again at run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pinned {
    pub path: String,
    pub sha256: String,
}

/// What a bot proposes: every field the sha256 covers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Proposal {
    pub project_id: String,
    /// `bot:<id>`, or `daemon` for a template the daemon filled (R4).
    pub proposed_by: String,
    pub item_id: Option<String>,
    pub decision_id: Option<String>,
    /// The machine name it runs on: this daemon's, or a peer's.
    pub target_machine: String,
    pub shell: Shell,
    pub cwd: String,
    pub content: String,
    pub pinned_files: Vec<Pinned>,
    pub reason: String,
    pub timeout_s: u32,
}

impl Proposal {
    /// The sha256 over the canonical JSON of every field: object keys sorted
    /// (serde_json's map is ordered), no whitespace. What the owner's client
    /// shows and sends back.
    pub fn sha256(&self) -> String {
        let canonical = json!({
            "project_id": self.project_id,
            "proposed_by": self.proposed_by,
            "item_id": self.item_id,
            "decision_id": self.decision_id,
            "target_machine": self.target_machine,
            "shell": self.shell.as_str(),
            "cwd": self.cwd,
            "content": self.content,
            "pinned_files": self.pinned_files,
            "reason": self.reason,
            "timeout_s": self.timeout_s,
        });
        hex::encode(Sha256::digest(canonical.to_string().as_bytes()))
    }
}

/// One owner action as stored.
#[derive(Debug, Clone, PartialEq)]
pub struct OwnerAction {
    pub id: String,
    pub proposal: Proposal,
    pub sha256: String,
    /// Warnings shown on the card, e.g. a path a bot can write that isn't
    /// pinned.
    pub flags: Vec<String>,
    /// `bot`, `daemon` (R4), or `peer:<id>` for a copy a peer offered (R3).
    pub origin: String,
    pub state: State,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub run_by: Option<String>,
    pub run_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
    pub exit_code: Option<i32>,
    pub output_path: Option<String>,
    /// The last 64 KB of output, redacted.
    pub output_tail: Option<String>,
    pub reject_reason: Option<String>,
}

impl OwnerAction {
    /// What clients and bots read: never the full output, only its
    /// redacted tail.
    pub fn to_json(&self) -> Value {
        let p = &self.proposal;
        json!({
            "id": self.id, "project_id": p.project_id, "proposed_by": p.proposed_by,
            "item_id": p.item_id, "decision_id": p.decision_id,
            "target_machine": p.target_machine, "shell": p.shell.as_str(), "cwd": p.cwd,
            "content": p.content, "pinned_files": p.pinned_files, "reason": p.reason,
            "timeout_s": p.timeout_s, "sha256": self.sha256, "flags": self.flags,
            "origin": self.origin, "state": self.state.as_str(),
            "created_at": self.created_at, "expires_at": self.expires_at,
            "run_by": self.run_by, "run_at": self.run_at, "finished_at": self.finished_at,
            "exit_code": self.exit_code, "output_tail": self.output_tail,
            "reject_reason": self.reject_reason,
        })
    }
}
