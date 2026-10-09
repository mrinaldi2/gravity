//! PRs waiting for the owner's review (H-269; ruling 7629a873): one row per
//! PR the owner must review, shown only once every required bot role has
//! approved the current change, so the owner never reviews what will change.

use bus::contract::home::{attention_row::Target, AttentionKind};

use super::rows::{Builder, Part};
use super::weight;
use crate::app::AppState;
use crate::prs::model::PrState;
use crate::prs::owner;

pub(super) fn owner_reviews(app: &AppState, b: &mut Builder<'_>) -> anyhow::Result<()> {
    let settings = app.db.review_settings(b.project_id)?;
    let prs = app
        .db
        .board_read(|t| t.prs(b.project_id, &[PrState::Open]))?;
    for pr in prs {
        let (shape, needs, reviews) = app
            .db
            .board_read(|t| Ok((t.pr_shape(&pr.id)?, t.pr_needs(&pr.id)?, t.reviews(&pr.id)?)))?;
        if !owner::waits_for_owner(&settings, &shape, &pr, &needs, &reviews) {
            continue;
        }
        let since = reviews
            .iter()
            .filter(|r| r.role != "owner")
            .map(|r| r.at)
            .max()
            .unwrap_or(pr.updated_at);
        let part = Part {
            kind: AttentionKind::PrReview,
            target_id: format!("pr-{}", pr.number),
            title: format!("Review PR #{} ({}): {}", pr.number, pr.item_id, pr.title),
            created_at: since,
            target: Some(Target::PrNumber(pr.number)),
        };
        b.push(part, weight(AttentionKind::PrReview), None);
    }
    Ok(())
}
