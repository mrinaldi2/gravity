//! Pull requests (H-261): a card's change, reviewed and merged in the app.
//! PR-1 keeps the record, its verified head and its card's moves; PR-5a the
//! checks each head must pass. Reviews and merges come with their own slices.

pub mod anchor;
pub mod check_checkout;
pub mod check_jobs;
pub mod check_log;
pub mod check_model;
pub mod check_remote;
pub mod check_rerun;
pub mod check_route;
pub mod checks;
pub mod comments;
pub mod flow;
pub mod follow_up;
pub mod merge;
pub mod mergeable;
pub mod model;
pub mod owner;
pub mod queue;
pub mod repo;
pub mod review;
pub mod review_model;
pub mod watch;
pub mod worktree;

use std::sync::Arc;

use bus::contract::pr::PrState as Wire;
use serde_json::Value;

use crate::app::AppState;
use model::{Pr, PrState};

/// The short names bots use for `hermes.pr.v1` enums, with their wire
/// names, for the tool schemas (`open` ↔ `PR_STATE_OPEN`).
pub fn spellings(enum_name: &str) -> Option<Vec<(&'static str, &'static str)>> {
    match enum_name {
        "PrState" => Some(
            PrState::ALL
                .iter()
                .map(|s| (s.as_str(), wire(*s).as_str_name()))
                .collect(),
        ),
        "CheckResult" => Some(
            check_model::CheckResult::ALL
                .iter()
                .map(|r| (r.as_str(), r.wire().as_str_name()))
                .collect(),
        ),
        "Verdict" => Some(vec![
            ("approved", "VERDICT_APPROVED"),
            ("changes_requested", "VERDICT_CHANGES_REQUESTED"),
        ]),
        "Side" => Some(vec![("old", "SIDE_OLD"), ("new", "SIDE_NEW")]),
        "Severity" => Some(vec![
            ("must", "SEVERITY_MUST"),
            ("should", "SEVERITY_SHOULD"),
            ("nit", "SEVERITY_NIT"),
        ]),
        _ => None,
    }
}

fn wire(state: PrState) -> Wire {
    match state {
        PrState::Open => Wire::Open,
        PrState::Merging => Wire::Merging,
        PrState::Merged => Wire::Merged,
        PrState::Closed => Wire::Closed,
    }
}

/// Wire states as the record's, open and merging when none is given.
pub fn states(wire_states: &[i32]) -> Vec<PrState> {
    let given: Vec<PrState> = wire_states
        .iter()
        .filter_map(|s| match Wire::try_from(*s).ok()? {
            Wire::Open => Some(PrState::Open),
            Wire::Merging => Some(PrState::Merging),
            Wire::Merged => Some(PrState::Merged),
            Wire::Closed => Some(PrState::Closed),
            Wire::Unspecified => None,
        })
        .collect();
    if given.is_empty() {
        vec![PrState::Open, PrState::Merging]
    } else {
        given
    }
}

/// Before a read: flag branches that moved without a report. A remote that
/// can't be reached leaves the records as they were.
pub fn look(app: &AppState, project: &str) {
    if let Err(e) = flow::observe(app, project) {
        tracing::warn!(project, error = %e, "couldn't check PR branches for unreported pushes");
    }
}

/// One PR in full: the record, its reviews (each with `stale`), the roles
/// it needs, its pushes, its worktrees and its head's checks as they count.
pub fn detail(app: &Arc<AppState>, pr: &Pr) -> anyhow::Result<Value> {
    let (pushes, worktrees, checks, reviews, needs) = app.db.board_read(|t| {
        Ok((
            t.pr_pushes(&pr.id)?,
            t.pr_worktrees(&pr.id)?,
            t.checks_on(&pr.project_id, &pr.head_sha)?,
            t.reviews(&pr.id)?,
            t.pr_needs(&pr.id)?,
        ))
    })?;
    let mut out = pr.to_json();
    out["reviews"] = reviews.iter().map(|r| r.to_json(pr)).collect();
    out["required_roles"] = serde_json::json!(needs);
    let owner = owner::owner_json(app, pr)?;
    out["owner_review_required"] = owner["owner_review_required"].clone();
    out["areas"] = owner["areas"].clone();
    out["mergeable"] = mergeable::compute(app, pr, true)?.to_json();
    out["comments"] = serde_json::json!(comments::list(app, pr, None)?);
    if let Some(row) = app.db.board_read(|t| t.queue_row(&pr.id))? {
        out["merge"] = serde_json::json!({
            "state": row.state, "queued_at": row.queued_at, "merge_at": row.merge_at,
            "task_id": row.task_id,
        });
    }
    out["pushes"] = pushes.iter().map(model::PrPush::to_json).collect();
    out["worktrees"] = worktrees.iter().map(model::PrWorktree::to_json).collect();
    out["checks"] = checks.iter().map(check_model::CheckRun::to_json).collect();
    Ok(out)
}
