//! What a new board starts with: the default columns (H-017 §1.3, names from
//! the glossary), the item-type templates (H-017 §2.1) and a first guess at
//! the project's roles. All of it is editable afterwards.

use super::model::{ColumnCategory, ItemType, Role, WipScope};
use serde_json::{json, Value};

pub struct DefaultColumn {
    pub key: &'static str,
    pub name: &'static str,
    pub category: ColumnCategory,
    pub wip_limit: Option<u32>,
    pub wip_scope: WipScope,
    pub visible: bool,
}

const fn column(
    key: &'static str,
    name: &'static str,
    category: ColumnCategory,
    wip_limit: Option<u32>,
    wip_scope: WipScope,
) -> DefaultColumn {
    DefaultColumn {
        key,
        name,
        category,
        wip_limit,
        wip_scope,
        visible: !matches!(category, ColumnCategory::Cancelled),
    }
}

pub const COLUMNS: &[DefaultColumn] = &[
    column(
        "inbox",
        "Inbox",
        ColumnCategory::Inbox,
        None,
        WipScope::Column,
    ),
    column(
        "ready",
        "Ready",
        ColumnCategory::Ready,
        Some(10),
        WipScope::Column,
    ),
    column(
        "doing",
        "Doing",
        ColumnCategory::Doing,
        Some(1),
        WipScope::PerAssignee,
    ),
    column(
        "review",
        "Review",
        ColumnCategory::Review,
        Some(3),
        WipScope::Column,
    ),
    column(
        "verify",
        "Verify",
        ColumnCategory::Verify,
        Some(3),
        WipScope::Column,
    ),
    column(
        "approval",
        "Awaiting owner",
        ColumnCategory::Approval,
        Some(5),
        WipScope::Column,
    ),
    column(
        "deploying",
        "Deploying",
        ColumnCategory::Deploying,
        None,
        WipScope::Column,
    ),
    column("done", "Done", ColumnCategory::Done, None, WipScope::Column),
    column(
        "cancelled",
        "Cancelled",
        ColumnCategory::Cancelled,
        None,
        WipScope::Column,
    ),
];

/// Description sections per item type, and the fields the Definition of
/// Ready requires before the item may leave the inbox.
pub fn item_templates() -> Vec<(ItemType, Value)> {
    vec![
        (
            ItemType::Feature,
            json!({
                "sections": ["User/problem", "Proposed behaviour", "Out of scope", "Design/spec link"],
                "ready": ["acceptance_criteria", "platforms", "size", "spec_link_for_ui_or_daemon"],
                "flow": "full"
            }),
        ),
        (
            ItemType::Bug,
            json!({
                "sections": ["Steps to reproduce", "Expected", "Actual", "Machine/version",
                             "Logs/screenshots", "Severity"],
                "ready": ["steps_to_reproduce", "expected_actual", "acceptance_criteria", "platforms"],
                "flow": "full"
            }),
        ),
        (
            ItemType::Spike,
            json!({
                "sections": ["Question to answer", "Timebox (≤ 24h)", "Options considered", "Outcome"],
                "ready": ["question", "timebox"],
                "flow": "doing_review_done"
            }),
        ),
        (
            ItemType::Chore,
            json!({
                "sections": ["What and why", "Risk", "Rollback"],
                "ready": ["description", "platforms"],
                "flow": "full_if_code"
            }),
        ),
        (
            ItemType::Epic,
            json!({
                "sections": ["Goal", "Success measure", "Children"],
                "ready": ["goal"],
                "flow": "none"
            }),
        ),
    ]
}

/// A new item's description: the template's sections as empty headings.
pub fn description_skeleton(template: &Value) -> String {
    template["sections"]
        .as_array()
        .map(|sections| {
            sections
                .iter()
                .filter_map(Value::as_str)
                .map(|s| format!("## {s}\n\n"))
                .collect::<String>()
        })
        .unwrap_or_default()
}

/// The item-id prefix a project starts with: the initials of its name in
/// capitals ("The Hermes" → "TH", "gravity" → "G"), or "B" when it has none.
pub fn key_for(project_name: &str) -> String {
    let initials: String = project_name
        .split(|c: char| !c.is_alphanumeric())
        .filter_map(|word| word.chars().next())
        .filter(char::is_ascii_alphabetic)
        .map(|c| c.to_ascii_uppercase())
        .take(4)
        .collect();
    if initials.is_empty() {
        "B".to_string()
    } else {
        initials
    }
}

/// The role a bot's name suggests, for seeding a new board: "Team Lead" leads,
/// "Architect" reviews architecture, and so on. Only a first guess; the lead
/// or owner sets roles properly.
pub fn guess_role(bot_name: &str) -> Option<Role> {
    let name = bot_name.to_lowercase();
    let has = |needle: &str| name.contains(needle);
    if has("lead") {
        Some(Role::Lead)
    } else if has("scrum") || has("coach") {
        Some(Role::Coach)
    } else if has("devops") {
        Some(Role::Devops)
    } else if has("architect") {
        Some(Role::ReviewerArch)
    } else if has("ux") || has("designer") {
        Some(Role::ReviewerUx)
    } else if has("tester") || has("qa") {
        Some(Role::Tester)
    } else if has("dev") || has("engineer") {
        Some(Role::Dev)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_come_from_initials() {
        assert_eq!(key_for("The Hermes"), "TH");
        assert_eq!(key_for("gravity"), "G");
        assert_eq!(key_for("my_app 2"), "MA");
        assert_eq!(key_for("42"), "B");
    }

    #[test]
    fn roles_are_guessed_from_names() {
        assert_eq!(guess_role("Team Lead"), Some(Role::Lead));
        assert_eq!(guess_role("Architect"), Some(Role::ReviewerArch));
        assert_eq!(guess_role("UX Designer"), Some(Role::ReviewerUx));
        assert_eq!(guess_role("iOS QA"), Some(Role::Tester));
        assert_eq!(guess_role("DevOps"), Some(Role::Devops));
        assert_eq!(guess_role("Desktop Dev"), Some(Role::Dev));
        assert_eq!(guess_role("Scrum Master"), Some(Role::Coach));
        assert_eq!(guess_role("Context Engineer"), Some(Role::Dev));
        assert_eq!(guess_role("Writer"), None);
    }

    #[test]
    fn every_item_type_has_a_template_and_the_skeleton_lists_its_sections() {
        let templates = item_templates();
        assert_eq!(templates.len(), 5);
        let bug = &templates
            .iter()
            .find(|(t, _)| *t == ItemType::Bug)
            .expect("bug")
            .1;
        assert!(description_skeleton(bug).starts_with("## Steps to reproduce\n\n## Expected"));
    }

    #[test]
    fn only_cancelled_is_hidden() {
        assert!(COLUMNS
            .iter()
            .filter(|c| !c.visible)
            .all(|c| c.key == "cancelled"));
        assert_eq!(COLUMNS.iter().filter(|c| !c.visible).count(), 1);
    }
}
