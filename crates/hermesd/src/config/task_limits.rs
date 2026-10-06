//! How many tasks a bot may hold open (H-125 G2, ARCH-R57 M1). A root task
//! — one sent by a bot holding no task — is budgeted per board card, under a
//! ceiling across the whole project, so the board throttles the work without
//! one budget of three for the lead's entire project. A nested task keeps
//! `MAX_TASK_FANOUT` per incoming task.
//!
//! ```toml
//! [tasks]
//! per_card = 3
//! root_lead = 12
//! root = 3
//!
//! [tasks.projects.gravity]   # by project name
//! per_card = 4
//! ```

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// The limits one project's bots send under.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TaskLimits {
    /// Open root tasks a bot may hold on one card.
    pub per_card: i64,
    /// Open root tasks the project's lead may hold across all cards.
    pub root_lead: i64,
    /// Open root tasks any other bot may hold across all cards.
    pub root: i64,
}

impl Default for TaskLimits {
    fn default() -> Self {
        Self {
            per_card: bus::MAX_TASK_FANOUT,
            root_lead: 12,
            root: bus::MAX_TASK_FANOUT,
        }
    }
}

/// One project's overrides; a field left out keeps the daemon-wide value.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct TaskLimitsOverride {
    pub per_card: Option<i64>,
    pub root_lead: Option<i64>,
    pub root: Option<i64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct TaskLimitsConfig {
    #[serde(flatten)]
    pub limits: TaskLimits,
    /// Keyed by the project's name.
    pub projects: BTreeMap<String, TaskLimitsOverride>,
}

impl TaskLimitsConfig {
    /// The limits for a project, each at least 1 so a typo can't stop every
    /// root send.
    pub fn for_project(&self, project_name: &str) -> TaskLimits {
        let base = self.limits;
        let over = self.projects.get(project_name).cloned().unwrap_or_default();
        TaskLimits {
            per_card: over.per_card.unwrap_or(base.per_card).max(1),
            root_lead: over.root_lead.unwrap_or(base.root_lead).max(1),
            root: over.root.unwrap_or(base.root).max(1),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_project_override_replaces_only_what_it_names() {
        let cfg: TaskLimitsConfig = toml::from_str(
            "per_card = 2\nroot_lead = 9\n[projects.gravity]\nper_card = 5\nroot = 0\n",
        )
        .expect("parse");
        assert_eq!(
            cfg.for_project("gravity"),
            TaskLimits {
                per_card: 5,
                root_lead: 9,
                root: 1
            }
        );
        assert_eq!(cfg.for_project("other").per_card, 2);
        assert_eq!(TaskLimitsConfig::default().for_project("x").root_lead, 12);
    }
}
