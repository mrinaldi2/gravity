//! A check on one commit as the daemon keeps it (H-261 §1.5): queued when
//! the head is reported, run by the worker it was dispatched to, and
//! reported by that worker alone.

use std::collections::BTreeMap;

use bus::contract::pr::CheckResult as Wire;
use chrono::{DateTime, Utc};
use serde_json::{json, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckResult {
    Queued,
    Running,
    Pass,
    Fail,
    /// It couldn't run: no computer with its tools, the runner died, or the
    /// base's `checks.toml` doesn't parse.
    Error,
}

impl CheckResult {
    pub const ALL: [CheckResult; 5] = [
        Self::Queued,
        Self::Running,
        Self::Pass,
        Self::Fail,
        Self::Error,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Pass => "pass",
            Self::Fail => "fail",
            Self::Error => "error",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|v| v.as_str() == s)
    }

    /// Pass, fail or error: the run is over.
    pub fn is_final(self) -> bool {
        matches!(self, Self::Pass | Self::Fail | Self::Error)
    }

    pub fn wire(self) -> Wire {
        match self {
            Self::Queued => Wire::Queued,
            Self::Running => Wire::Running,
            Self::Pass => Wire::Pass,
            Self::Fail => Wire::Fail,
            Self::Error => Wire::Error,
        }
    }

    pub fn from_wire(wire: i32) -> Option<Self> {
        Self::ALL.into_iter().find(|r| r.wire() as i32 == wire)
    }
}

/// A check to queue on a head: its `checks.toml` entry at the base.
#[derive(Debug, Clone, PartialEq)]
pub struct NewCheck {
    pub name: String,
    pub run: String,
    pub needs: Vec<String>,
    pub machine: Option<String>,
    pub required: bool,
    /// `Queued`, or `Error` (with `note`) when the base's policy is broken.
    pub result: CheckResult,
    pub note: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CheckRun {
    pub id: String,
    pub project_id: String,
    pub repo: String,
    pub sha: String,
    pub tree: String,
    pub name: String,
    pub run: String,
    pub needs: Vec<String>,
    pub machine: Option<String>,
    pub required: bool,
    pub result: CheckResult,
    pub note: Option<String>,
    /// The worker it was dispatched to (H-283).
    pub runner: Option<String>,
    /// The computer it ran on.
    pub ran_on: Option<String>,
    /// The log, under the project's artifacts on the board home.
    pub log_artifact: Option<String>,
    pub tool_versions: BTreeMap<String, String>,
    pub queued_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
    /// Set when this result is another commit's with the same tree, which
    /// counts for this one (§1.5): that commit.
    pub tree_of: Option<String>,
}

impl CheckRun {
    /// The record with the `hermes.pr.v1` `CheckRun` field names.
    pub fn to_json(&self) -> Value {
        json!({
            "name": self.name, "sha": self.sha, "tree": self.tree,
            "result": self.result.as_str(), "required": self.required,
            "note": self.note, "runner": self.runner, "ran_on": self.ran_on,
            "log_url": self.log_artifact, "tool_versions": self.tool_versions,
            "started_at": self.started_at, "finished_at": self.finished_at,
            "tree_of": self.tree_of.clone().unwrap_or_default(),
        })
    }
}

/// What the dispatched worker reported.
#[derive(Debug, Clone, PartialEq)]
pub struct Report<'a> {
    pub result: CheckResult,
    pub ran_on: &'a str,
    pub log_artifact: Option<&'a str>,
    pub tool_versions: &'a BTreeMap<String, String>,
}
