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

/// Merges DevOps hasn't run within 30 min (H-284 S2): one row per PR,
/// since it was first handed, until it merges or leaves the queue.
pub(super) fn stuck_merges(app: &AppState, b: &mut Builder<'_>) -> anyhow::Result<()> {
    let stuck = app.db.board_read(|t| t.merges_stuck(b.project_id))?;
    for s in stuck {
        let pr = app.db.board_read(|t| {
            Ok(t.prs(b.project_id, &[PrState::Merging])?
                .into_iter()
                .find(|p| p.id == s.pr_id))
        })?;
        let Some(pr) = pr else { continue };
        let part = Part {
            kind: AttentionKind::PrMergeStuck,
            target_id: format!("pr-{}", pr.number),
            title: format!(
                "PR #{} ({}) is waiting for DevOps to merge it; asked {} times",
                pr.number,
                pr.item_id,
                s.retasks + 1
            ),
            created_at: s.since,
            target: Some(Target::PrNumber(pr.number)),
        };
        b.push(part, weight(AttentionKind::PrMergeStuck), None);
    }
    Ok(())
}

/// Main reached a PR's head with no `hermesd pr merge` pass for it (ARCH M1
/// on H-284): nothing was recorded; the owner decides what to do.
pub(super) fn mains_moved(app: &AppState, b: &mut Builder<'_>) -> anyhow::Result<()> {
    let moved = app.db.board_read(|t| t.mains_moved(b.project_id))?;
    for (pr_id, sha, at) in moved {
        let pr = app.db.board_read(|t| {
            Ok(t.prs(b.project_id, &[PrState::Open, PrState::Merging])?
                .into_iter()
                .find(|p| p.id == pr_id))
        })?;
        let Some(pr) = pr else { continue };
        let part = Part {
            kind: AttentionKind::MainMovedOutside,
            target_id: format!("pr-{}", pr.number),
            title: format!(
                "Main moved to PR #{}'s head {} outside the merge gate; nothing was recorded",
                pr.number,
                &sha[..sha.len().min(7)]
            ),
            created_at: at,
            target: Some(Target::PrNumber(pr.number)),
        };
        b.push(part, weight(AttentionKind::MainMovedOutside), None);
    }
    Ok(())
}
