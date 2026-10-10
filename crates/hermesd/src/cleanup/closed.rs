//! Closed, unmerged PRs (H-261 §15.4, CL-2; retention per ruling 7629a873):
//! the worktrees wait 7 days, then go under the same rules as a merged PR's
//! (a dirty or unpushed one is salvaged and held); the remote branch waits
//! 14 days, then DevOps' next `hermesd pr merge` deletes it while its tip is
//! still the head the PR was closed at. A PR opened again on the branch
//! within the window cancels both.

use std::sync::Arc;

use chrono::{Duration, Utc};
use serde_json::{json, Value};

use crate::app::AppState;
use crate::prs::model::{Pr, PrState};

/// A closed PR's worktrees are removed this long after it closed…
pub const TREES_AFTER: Duration = Duration::days(7);
/// …and its remote branch after this long.
pub const BRANCH_AFTER: Duration = Duration::days(14);

/// Queues a closed PR's cleanup, due 7 days after it closed.
pub fn enqueue(app: &AppState, pr: &Pr) -> anyhow::Result<usize> {
    let at = pr.closed_at.unwrap_or_else(Utc::now) + TREES_AFTER;
    super::enqueue_until(app, pr, Some(at))
}

/// After `pr_close`: queues it, and says so in the log when it can't.
pub fn after_close(app: &Arc<AppState>, pr: &Pr) {
    if pr.state != PrState::Closed {
        return;
    }
    if let Err(error) = enqueue(app, pr) {
        tracing::warn!(pr = pr.number, %error, "a closed PR's cleanup wasn't queued");
    }
}

/// The closed PRs of `pr`'s repository whose remote branch may go now, for
/// the merge run DevOps is about to make.
pub fn stale_branches(app: &AppState, pr: &Pr) -> anyhow::Result<Value> {
    let before = Utc::now() - BRANCH_AFTER;
    let stale = app
        .db
        .board_read(|t| t.stale_closed_branches(&pr.project_id, &pr.repo, before))?;
    Ok(stale
        .into_iter()
        .filter(|s| s.branch != pr.branch && !s.sha.is_empty())
        .map(|s| json!({"number": s.number, "branch": s.branch, "sha": s.sha}))
        .collect())
}

/// What DevOps' run did with each stale branch: kept only for a closed PR of
/// the same repository that was due, so a run can't mark any other branch.
pub fn record_stale(app: &AppState, merged: &Pr, done: &Value) -> anyhow::Result<usize> {
    let due: Vec<u32> = app
        .db
        .board_read(|t| {
            t.stale_closed_branches(&merged.project_id, &merged.repo, Utc::now() - BRANCH_AFTER)
        })?
        .into_iter()
        .map(|s| s.number)
        .collect();
    let mut kept = 0;
    for row in done.as_array().map(Vec::as_slice).unwrap_or_default() {
        let Some(number) = row["number"].as_u64().and_then(|n| u32::try_from(n).ok()) else {
            continue;
        };
        if !due.contains(&number) {
            tracing::warn!(number, "a stale-branch result for a PR that wasn't due");
            continue;
        }
        let deleted = row["deleted"].as_bool() == Some(true);
        let note = row["note"]
            .as_str()
            .unwrap_or(if deleted { "deleted" } else { "kept" });
        app.db
            .board_tx(|t| {
                let pr = t.pr(&merged.project_id, number)?;
                match pr {
                    Some(pr) => t.set_branch_cleanup(&pr.id, deleted, note).map(|_| 1),
                    None => Ok(0),
                }
            })
            .map(|n| kept += n)?;
    }
    Ok(kept)
}
