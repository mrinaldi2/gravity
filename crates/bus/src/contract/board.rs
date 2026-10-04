//! Board surface, contract version 1: the entities (H-020 §1.1). Requests,
//! responses and pushes join them when the WS surface lands (B4).

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

mod enums;
pub use enums::*;

/// Bumped only on a breaking change to this surface.
pub const VERSION: u32 = 1;

/// One project's board configuration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct BoardSettings {
    pub project_id: String,
    /// Item id prefix: "H" gives "H-017".
    pub key: String,
    /// The next item number; ids are never reused.
    pub next_seq: u32,
    pub stale_after_hours: u32,
    /// Machines that must verify each platform, e.g. `{"desktop": ["mac", "win-pc"]}`.
    pub required_machines: BTreeMap<Platform, Vec<String>>,
    /// The daemon that holds the board; linked computers forward to it.
    pub home_daemon_id: String,
    pub version: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct BoardColumn {
    pub project_id: String,
    pub key: String,
    pub name: String,
    pub ord: u32,
    pub category: ColumnCategory,
    pub wip_limit: Option<u32>,
    pub wip_scope: WipScope,
    pub visible: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ProjectRole {
    pub project_id: String,
    pub role: Role,
    pub bot_id: String,
    /// The machine a tester verifies on.
    pub machine: Option<String>,
}

/// Set while an item is blocked: a flag, not a column.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Blocked {
    /// The blocking item, when it is one.
    pub by: Option<String>,
    pub reason: String,
    pub since: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct AcceptanceCriterion {
    pub idx: u32,
    pub text: String,
    pub checked: bool,
    pub checked_by: Option<String>,
    pub checked_at: Option<DateTime<Utc>>,
    /// Where it was checked, for platform criteria.
    pub machine: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ItemPerson {
    pub bot_id: String,
    pub role: PersonRole,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ItemVerification {
    pub machine: String,
    pub result: VerificationResult,
    pub by: String,
    pub at: DateTime<Utc>,
    pub note: Option<String>,
}

/// A work item in full, as the item drawer shows it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Item {
    /// "H-017"; immutable.
    pub id: String,
    pub seq: u32,
    #[serde(rename = "type")]
    pub item_type: ItemType,
    pub title: String,
    /// Markdown, filled from the type's template.
    pub description: String,
    pub platforms: Vec<Platform>,
    pub size: Option<Size>,
    pub priority: Priority,
    /// Fractional-index key: order within the backlog.
    pub rank: String,
    pub column_key: String,
    pub category: ColumnCategory,
    pub blocked: Option<Blocked>,
    pub assignee: Option<String>,
    pub parent_id: Option<String>,
    pub release_id: Option<String>,
    pub labels: Vec<String>,
    pub acceptance_criteria: Vec<AcceptanceCriterion>,
    pub people: Vec<ItemPerson>,
    pub verifications: Vec<ItemVerification>,
    pub created_by: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub state_entered_at: DateTime<Utc>,
    pub done_at: Option<DateTime<Utc>>,
    /// Optimistic concurrency: every mutation names the version it read.
    pub version: u64,
}

/// The compact form a board column lists; the drawer fetches the [`Item`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ItemCard {
    pub id: String,
    #[serde(rename = "type")]
    pub item_type: ItemType,
    pub title: String,
    pub priority: Priority,
    pub size: Option<Size>,
    pub rank: String,
    pub column_key: String,
    pub assignee: Option<String>,
    pub platforms: Vec<Platform>,
    pub labels: Vec<String>,
    pub blocked: bool,
    pub stale: bool,
    pub ac_checked: u32,
    pub ac_total: u32,
    pub version: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ItemLink {
    pub item_id: String,
    pub kind: LinkKind,
    /// What the link points at: a task id, a branch name, another item id…
    #[serde(rename = "ref")]
    pub target: String,
    pub label: Option<String>,
    pub created_by: String,
    pub at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ItemComment {
    pub id: String,
    pub item_id: String,
    pub author: String,
    pub body: String,
    pub reply_to: Option<String>,
    pub at: DateTime<Utc>,
}

/// One append-only history entry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ItemEvent {
    pub id: i64,
    pub item_id: String,
    pub at: DateTime<Utc>,
    /// The actor as stored: `user`, `bot:<id>` or `device:<id>`.
    pub actor: String,
    pub kind: ItemEventKind,
    pub from: Option<String>,
    pub to: Option<String>,
    pub field: Option<String>,
    pub note: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Template {
    pub project_id: String,
    pub kind: TemplateKind,
    pub name: String,
    pub version: u32,
    /// Sections and required fields; its shape is the template's own.
    pub body: serde_json::Value,
}

/// A guard a move does not meet yet, with what would fix it. The UI shows these
/// texts as they are and never re-implements a rule.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Unmet {
    pub code: String,
    pub text: String,
    pub fix: Option<String>,
}

/// Every board entity, so one schema (and the generated TypeScript and
/// Swift) names them all. Not a message on the wire.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct BoardContract {
    pub settings: BoardSettings,
    pub column: BoardColumn,
    pub role: ProjectRole,
    pub item: Item,
    pub card: ItemCard,
    pub link: ItemLink,
    pub comment: ItemComment,
    pub event: ItemEvent,
    pub template: Template,
    pub unmet: Unmet,
}
