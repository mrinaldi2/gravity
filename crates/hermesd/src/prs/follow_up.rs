//! A `should` or `nit` finding turned into its own card (H-261 §1.3): an
//! Inbox card related to the PR's card, recorded on the finding so it isn't
//! filed twice.

use std::sync::Arc;

use crate::actor::Actor;
use crate::app::AppState;
use crate::board::model::{ItemType, LinkKind, Priority, Role};
use crate::db::NewItem;
use crate::decisions::{conflict, forbidden, invalid, not_found};

/// `pr_follow_up`: the reviewer, the PR's author or the lead files finding
/// `index` of review `review_id` as an Inbox card; returns its id.
pub fn file(
    app: &Arc<AppState>,
    bot: &bus::Bot,
    roles: &[Role],
    number: u32,
    review_id: &str,
    index: usize,
) -> anyhow::Result<String> {
    let project = bot.project_id.as_str();
    let pr = app
        .db
        .board_read(|t| t.pr(project, number))?
        .ok_or_else(|| not_found(format!("no PR #{number} in this project")))?;
    let review = app
        .db
        .board_read(|t| t.review(review_id))?
        .filter(|r| r.pr_id == pr.id)
        .ok_or_else(|| not_found(format!("no review {review_id} on PR #{number}")))?;
    if review.reviewer != bot.id && pr.author != bot.id && !roles.contains(&Role::Lead) {
        return Err(forbidden(
            "the reviewer, the PR's author or the lead files a follow-up",
        ));
    }
    let finding = review
        .findings
        .get(index)
        .ok_or_else(|| invalid(format!("the review has no finding {index}")))?;
    if finding.severity == "must" {
        return Err(invalid(
            "a `must` finding is fixed in this PR, not filed for later",
        ));
    }
    if let Some(id) = &finding.follow_up_item_id {
        return Err(conflict(format!("that finding is already {id}")));
    }
    let title: String = finding.text.trim().chars().take(80).collect();
    let where_ = match (&finding.path, finding.line) {
        (Some(path), Some(line)) => format!(" ({path}:{line})"),
        (Some(path), None) => format!(" ({path})"),
        _ => String::new(),
    };
    let description = format!(
        "## Why\nA {} finding from the {} review of PR #{} ({}){where_}.\n\n## What\n{}",
        finding.severity,
        review.role,
        pr.number,
        pr.item_id,
        finding.text.trim()
    );
    let actor = Actor::Bot {
        id: &bot.id,
        project_id: project,
    };
    let mut findings = review.findings.clone();
    let item = app.db.board_tx(|t| {
        let item = t.create_item(
            &NewItem {
                project_id: project,
                item_type: ItemType::Chore,
                title: &title,
                description: &description,
                platforms: &[],
                size: None,
                priority: Priority::P2,
                labels: &["follow-up".to_string()],
                parent_id: None,
                acceptance_criteria: &[],
            },
            &actor,
        )?;
        t.add_item_link(&item.id, LinkKind::ItemRelates, &pr.item_id, None, &actor)?;
        findings[index].follow_up_item_id = Some(item.id.clone());
        t.set_review_findings(&review.id, &findings)?;
        Ok(item)
    })?;
    Ok(item.id)
}
