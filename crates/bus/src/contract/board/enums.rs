//! Closed value sets of the board surface (H-017 §1, H-020 §1.1).

use serde::{Deserialize, Serialize};

/// The canonical category a column maps to. Guards and metrics key on it, so
/// renaming or splitting a column never changes the rules.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum ColumnCategory {
    Inbox,
    Ready,
    Doing,
    Review,
    Verify,
    Approval,
    Deploying,
    Done,
    Cancelled,
}

/// What a column's WIP limit counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum WipScope {
    Column,
    PerAssignee,
}

/// A bot's role in a project's way of working.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum Role {
    #[serde(rename = "lead")]
    Lead,
    #[serde(rename = "coach")]
    Coach,
    #[serde(rename = "devops")]
    Devops,
    #[serde(rename = "reviewer.arch")]
    ReviewerArch,
    #[serde(rename = "reviewer.ux")]
    ReviewerUx,
    #[serde(rename = "tester")]
    Tester,
    #[serde(rename = "dev")]
    Dev,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum ItemType {
    Epic,
    Feature,
    Bug,
    Spike,
    Chore,
}

/// Where an item's change lands; decides who must verify it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum Platform {
    Desktop,
    Ios,
    Daemon,
    Infra,
}

/// L is allowed only in the inbox: it must be split before it is ready.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum Size {
    S,
    M,
    L,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum Priority {
    P0,
    P1,
    P2,
    P3,
}

/// A bot's part on one item, beyond the assignee.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum PersonRole {
    Reviewer,
    Verifier,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum VerificationResult {
    Pass,
    Fail,
    Blocked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum LinkKind {
    #[serde(rename = "task")]
    Task,
    #[serde(rename = "decision")]
    Decision,
    #[serde(rename = "artifact")]
    Artifact,
    #[serde(rename = "branch")]
    Branch,
    #[serde(rename = "pr")]
    Pr,
    #[serde(rename = "meeting")]
    Meeting,
    #[serde(rename = "item:blocks")]
    ItemBlocks,
    #[serde(rename = "item:relates")]
    ItemRelates,
    #[serde(rename = "item:duplicates")]
    ItemDuplicates,
}

/// What an item-history event records. Metrics are computed from these.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum ItemEventKind {
    Created,
    Moved,
    Edited,
    Commented,
    Linked,
    Assigned,
    Blocked,
    Ranked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum TemplateKind {
    ItemType,
    MeetingType,
}
