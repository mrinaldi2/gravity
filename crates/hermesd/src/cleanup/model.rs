//! A cleanup job's record and what one attempt at it found (H-261 §15.1).

use chrono::{DateTime, Utc};
use serde_json::{json, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A reported or discovered worktree, with its build output.
    Worktree,
    /// Look on one computer for worktrees of the branch nobody reported.
    Discover,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Worktree => "worktree",
            Kind::Discover => "discover",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "worktree" => Some(Kind::Worktree),
            "discover" => Some(Kind::Discover),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobState {
    Queued,
    Done,
    Held,
    Failed,
}

impl JobState {
    pub fn as_str(self) -> &'static str {
        match self {
            JobState::Queued => "queued",
            JobState::Done => "done",
            JobState::Held => "held",
            JobState::Failed => "failed",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "queued" => Some(JobState::Queued),
            "done" => Some(JobState::Done),
            "held" => Some(JobState::Held),
            "failed" => Some(JobState::Failed),
            _ => None,
        }
    }
}

/// One row of `cleanup_job`.
#[derive(Debug, Clone)]
pub struct Job {
    pub id: String,
    pub project_id: String,
    pub pr_id: Option<String>,
    pub machine: String,
    pub kind: Kind,
    pub path_or_ref: String,
    pub main_clone: Option<String>,
    pub bot_id: Option<String>,
    pub state: JobState,
    pub reason: String,
    pub bytes_freed: u64,
    pub attempts: u32,
    pub busy_since: Option<DateTime<Utc>>,
    pub next_at: Option<DateTime<Utc>>,
    pub sent_at: Option<DateTime<Utc>>,
    pub at: DateTime<Utc>,
}

impl Job {
    pub fn to_json(&self) -> Value {
        json!({
            "id": self.id, "machine": self.machine, "kind": self.kind.as_str(),
            "path": self.path_or_ref, "state": self.state.as_str(), "reason": self.reason,
            "bytes_freed": self.bytes_freed, "attempts": self.attempts,
        })
    }
}

/// A job to queue.
#[derive(Debug, Clone)]
pub struct NewJob {
    pub project_id: String,
    pub pr_id: String,
    pub machine: String,
    pub kind: Kind,
    pub path_or_ref: String,
    pub main_clone: Option<String>,
    pub bot_id: Option<String>,
}

/// What one attempt at a job found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Removed (or already gone), freeing `bytes`.
    Done { bytes: u64 },
    /// In use: tried again in 15 minutes, for 24 hours (§15.3 rule 3).
    Busy(String),
    /// Kept, with the reason; nothing was deleted.
    Held(String),
    /// Couldn't be done at all; nothing was deleted.
    Failed(String),
}

impl Outcome {
    pub fn to_json(&self) -> Value {
        let (state, reason, bytes) = match self {
            Outcome::Done { bytes } => ("done", "", *bytes),
            Outcome::Busy(r) => ("busy", r.as_str(), 0),
            Outcome::Held(r) => ("held", r.as_str(), 0),
            Outcome::Failed(r) => ("failed", r.as_str(), 0),
        };
        json!({"state": state, "reason": reason, "bytes": bytes})
    }

    pub fn from_json(v: &Value) -> Option<Self> {
        let reason: String = v["reason"].as_str()?.chars().take(1000).collect();
        Some(match v["state"].as_str()? {
            "done" => Outcome::Done {
                bytes: v["bytes"].as_u64()?,
            },
            "busy" => Outcome::Busy(reason),
            "held" => Outcome::Held(reason),
            "failed" => Outcome::Failed(reason),
            _ => return None,
        })
    }
}

/// `3.1 GB`, `120 MB`, `4 KB`: what the author's note says was freed.
pub fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["bytes", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1000.0 && unit < UNITS.len() - 1 {
        value /= 1000.0;
        unit += 1;
    }
    match unit {
        0 => format!("{bytes} bytes"),
        _ if value < 10.0 => format!("{value:.1} {}", UNITS[unit]),
        _ => format!("{value:.0} {}", UNITS[unit]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_read_as_people_say_them() {
        assert_eq!(human_bytes(512), "512 bytes");
        assert_eq!(human_bytes(3_100_000_000), "3.1 GB");
        assert_eq!(human_bytes(120_000_000), "120 MB");
    }

    #[test]
    fn an_outcome_crosses_the_link_unchanged() {
        for o in [
            Outcome::Done { bytes: 7 },
            Outcome::Busy("in use".into()),
            Outcome::Held("dirty".into()),
            Outcome::Failed("gone wrong".into()),
        ] {
            assert_eq!(Outcome::from_json(&o.to_json()), Some(o));
        }
    }
}
