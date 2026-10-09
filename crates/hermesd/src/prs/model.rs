//! A pull request as the daemon keeps it (H-261 §1.1): a card's change on a
//! branch, its verified head and the patch-id its approvals are bound to.

use chrono::{DateTime, Utc};
use serde_json::{json, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrState {
    Open,
    /// Mergeable, inside the owner's Undo window (H-271).
    Merging,
    Merged,
    Closed,
}

impl PrState {
    pub const ALL: [PrState; 4] = [Self::Open, Self::Merging, Self::Merged, Self::Closed];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Merging => "merging",
            Self::Merged => "merged",
            Self::Closed => "closed",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|v| v.as_str() == s)
    }

    /// Open or merging: the card's one live PR.
    pub fn is_live(self) -> bool {
        matches!(self, Self::Open | Self::Merging)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Pr {
    pub id: String,
    pub project_id: String,
    pub number: u32,
    /// "owner/name".
    pub repo: String,
    pub item_id: String,
    pub branch: String,
    pub base: String,
    /// Main when the head was last verified.
    pub base_sha: String,
    /// The latest reported and verified head (§1.2).
    pub head_sha: String,
    /// `git patch-id --stable` of the PR's own change (§3).
    pub head_patch_id: String,
    /// The branch's tip as the daemon last saw it.
    pub remote_sha: String,
    /// The tip moved and nobody reported it.
    pub moved_unreported: bool,
    pub state: PrState,
    /// A bot id: the card's assignee at open.
    pub author: String,
    pub title: String,
    pub change_note: String,
    pub owner_flagged: bool,
    pub owner_flag_reason: Option<String>,
    pub merged_sha: Option<String>,
    pub merged_at: Option<DateTime<Utc>>,
    pub close_reason: Option<String>,
    pub opened_at: DateTime<Utc>,
    pub closed_at: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
    pub version: u64,
}

impl Pr {
    /// The record for bots and clients, with the `hermes.pr.v1` field names.
    pub fn to_json(&self) -> Value {
        json!({
            "id": self.id, "project_id": self.project_id, "number": self.number,
            "repo": self.repo, "item_id": self.item_id, "branch": self.branch,
            "base": self.base, "base_sha": self.base_sha, "head_sha": self.head_sha,
            "state": self.state.as_str(), "author": self.author, "title": self.title,
            "change_note": self.change_note, "owner_flagged": self.owner_flagged,
            "owner_flag_reason": self.owner_flag_reason,
            "moved_unreported": self.moved_unreported,
            "remote_sha": self.remote_sha, "merged_sha": self.merged_sha,
            "merged_at": self.merged_at, "close_reason": self.close_reason,
            "opened_at": self.opened_at, "closed_at": self.closed_at,
            "version": self.version,
        })
    }
}

/// A head the daemon accepted or saw (§1.2). `pushed_by` is `None` for a
/// tip that moved without a report: attributed to no one.
#[derive(Debug, Clone, PartialEq)]
pub struct PrPush {
    pub sha: String,
    pub patch_id: String,
    pub pushed_by: Option<String>,
    pub at: DateTime<Utc>,
}

impl PrPush {
    pub fn to_json(&self) -> Value {
        json!({ "sha": self.sha, "patch_id": self.patch_id, "pushed_by": self.pushed_by, "at": self.at })
    }
}

/// Where a bot works on the PR's branch, verified by its computer (§15.1).
#[derive(Debug, Clone, PartialEq)]
pub struct PrWorktree {
    pub machine: String,
    pub bot_id: String,
    pub path: String,
    pub main_clone: String,
}

impl PrWorktree {
    pub fn to_json(&self) -> Value {
        json!({ "machine": self.machine, "bot_id": self.bot_id, "path": self.path,
                "main_clone": self.main_clone })
    }
}
