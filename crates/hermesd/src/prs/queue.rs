//! The per-project merge queue (H-261 §5.2, §16; ruling fc46b042): nobody
//! presses merge. A PR that becomes mergeable joins its project's queue,
//! first in first; the head of the queue gets a 10 s `merging` window in
//! which the owner may Undo, and nothing is pushed; then DevOps gets a
//! `pr_merge` task (the executor is H-284). One PR at a time per project:
//! after a merge, main has moved, so the next one shows "needs update with
//! main" until its author updates it.

use std::sync::Arc;
use std::time::Duration;

use bus::{MessageKind, TaskState, DEFAULT_TASK_DEADLINE_HOURS};
use chrono::{DateTime, Utc};

use crate::actor::Actor;
use crate::app::AppState;
use crate::board::model::Role;
use crate::db::merge_queue::QueueRow;
use crate::db::OwnerProof;
use crate::decisions::{conflict, not_found};
use crate::messaging::{self, daemon_sender, Dm};
use crate::prs::model::{Pr, PrState};
use crate::prs::review_model::Verdict;
use crate::prs::{mergeable, owner};

/// The owner's Undo window (ruling 7629a873, UX-051 decision 4).
pub const WINDOW: chrono::Duration = chrono::Duration::seconds(10);
/// A handed merge DevOps hasn't run by then goes to DevOps again, and the
/// owner sees it (H-284, Architect S2 on H-271).
pub const STUCK_AFTER: chrono::Duration = chrono::Duration::minutes(30);
/// How often the daemon's ticker moves every project's queue.
pub const EVERY: Duration = Duration::from_secs(2);

pub fn spawn(app: Arc<AppState>) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(EVERY);
        loop {
            tick.tick().await;
            let app = app.clone();
            let _ = tokio::task::spawn_blocking(move || {
                if let Err(e) = step(&app, Utc::now()) {
                    tracing::warn!(error = %e, "the merge queue's pass failed");
                }
            })
            .await;
        }
    });
}

/// One pass over every project with PR work, as of `now`.
pub fn step(app: &Arc<AppState>, now: DateTime<Utc>) -> anyhow::Result<()> {
    for project in app.db.board_read(|t| t.projects_with_queue_work())? {
        if let Err(e) = tick(app, &project, now) {
            tracing::warn!(project, error = %e, "the merge queue couldn't move");
        }
    }
    Ok(())
}

/// The owner approved this change after the Undo: the PR may queue again.
fn undo_lifted(app: &AppState, pr: &Pr, undone_at: DateTime<Utc>) -> anyhow::Result<bool> {
    let reviews = app.db.board_read(|t| t.reviews(&pr.id))?;
    Ok(reviews.iter().any(|r| {
        r.role == "owner" && r.verdict == Verdict::Approved && r.at > undone_at && !r.stale(pr)
    }))
}

/// Moves one project's queue as of `now`.
pub fn tick(app: &Arc<AppState>, project: &str, now: DateTime<Utc>) -> anyhow::Result<()> {
    let prs = app
        .db
        .board_read(|t| t.prs(project, &[PrState::Open, PrState::Merging]))?;
    for pr in &prs {
        let row = app.db.board_read(|t| t.queue_row(&pr.id))?;
        if let Some(row) = &row {
            if row.state == "handed" {
                // DevOps runs it now (`hermesd pr merge`), unless it's stuck.
                if now - row.updated_at >= STUCK_AFTER {
                    stuck(app, pr, row, now)?;
                }
                continue;
            }
            if row.state == "undone" {
                if row.patch_id == pr.head_patch_id && !undo_lifted(app, pr, row.updated_at)? {
                    continue;
                }
                app.db.board_tx(|t| t.dequeue(&pr.id))?;
            }
        }
        let ok = mergeable::compute(app, pr, false)?.ok();
        let row = app.db.board_read(|t| t.queue_row(&pr.id))?;
        app.db.board_tx(|t| {
            match (&row, ok) {
                (None, true) => t.enqueue(pr, now)?,
                (Some(r), false) if r.state == "queued" || r.state == "window" => {
                    t.dequeue(&pr.id)?;
                    if pr.state == PrState::Merging {
                        t.set_pr_state(pr, PrState::Open)?;
                    }
                }
                _ => {}
            }
            Ok(())
        })?;
    }
    let queue = app.db.board_read(|t| t.queue(project))?;
    if !queue
        .iter()
        .any(|r| r.state == "window" || r.state == "handed")
    {
        if let Some(first) = queue.iter().find(|r| r.state == "queued") {
            let pr = prs.iter().find(|p| p.id == first.pr_id);
            if let Some(pr) = pr {
                app.db.board_tx(|t| {
                    t.set_queue_state(
                        &pr.id,
                        "window",
                        Some(now + WINDOW),
                        None,
                        &first.patch_id,
                        now,
                    )?;
                    t.set_pr_state(pr, PrState::Merging)
                })?;
            }
        }
    }
    let queue = app.db.board_read(|t| t.queue(project))?;
    for row in queue.iter().filter(|r| r.state == "window") {
        if row.merge_at.is_some_and(|end| end <= now) {
            if let Some(pr) = prs.iter().find(|p| p.id == row.pr_id) {
                hand_off(app, pr, row.patch_id.as_str(), now)?;
            }
        }
    }
    Ok(())
}

/// The window ended: DevOps gets the `pr_merge` task.
fn hand_off(
    app: &Arc<AppState>,
    pr: &Pr,
    patch_id: &str,
    now: DateTime<Utc>,
) -> anyhow::Result<()> {
    let devops = app
        .db
        .project_roles(&pr.project_id)?
        .into_iter()
        .find(|r| r.role == Role::Devops)
        .map(|r| r.bot_id);
    let Some(devops) = devops else {
        tracing::warn!(
            pr = pr.number,
            "no DevOps to merge it; the PR waits in its window"
        );
        return Ok(());
    };
    let body = format!(
        "Merge PR #{} ({}: {}) into main: main fast-forwards to {}. Run `hermesd pr merge {}` \
         in your checkout; it checks the PR is still mergeable first.",
        pr.number, pr.item_id, pr.title, pr.head_sha, pr.number
    );
    let sender = daemon_sender();
    let msg = messaging::send_dm(
        &app.db,
        &app.events,
        Dm::new(&devops, &sender, MessageKind::Task, &body),
    )?;
    let deadline = Utc::now() + chrono::Duration::hours(DEFAULT_TASK_DEADLINE_HOURS);
    let task = app
        .db
        .create_task(&msg.id, None, &devops, Some(deadline), 1, "")?;
    app.db.link_task_item(&task.id, &pr.item_id, &Actor::User)?;
    app.db
        .board_tx(|t| t.set_queue_state(&pr.id, "handed", None, Some(&task.id), patch_id, now))
}

/// A handed merge timed out: its task expires, DevOps gets it again, and
/// it's recorded as stuck since it was first handed, for the owner's
/// Needs you.
fn stuck(app: &Arc<AppState>, pr: &Pr, row: &QueueRow, now: DateTime<Utc>) -> anyhow::Result<()> {
    if let Some(task) = &row.task_id {
        app.db.try_close_task(task, TaskState::Expired)?;
    }
    let since = app
        .db
        .board_read(|t| t.merge_stuck(&pr.id))?
        .map_or(row.updated_at, |s| s.since);
    app.db.board_tx(|t| {
        t.mark_merge_stuck(pr, since, now)?;
        // Restarts the clock even with no DevOps to hand it to.
        t.set_queue_state(&pr.id, "handed", None, None, &row.patch_id, now)
    })?;
    tracing::warn!(
        pr = pr.number,
        "a handed merge timed out; DevOps gets it again"
    );
    hand_off(app, pr, &row.patch_id, now)
}

/// The owner's Undo within the window (device or ticket): the PR goes back
/// to open, the owner's approval of this change is withdrawn, nothing was
/// pushed. It stays out of the queue until the change or the owner's
/// approval changes.
pub fn undo(
    app: &Arc<AppState>,
    project: &str,
    number: u32,
    proof: &OwnerProof,
) -> anyhow::Result<Pr> {
    let by = owner::provenance(proof)?;
    let now = Utc::now();
    app.db.board_tx(|t| {
        let pr = t
            .pr(project, number)?
            .ok_or_else(|| not_found(format!("no PR #{number} in this project")))?;
        let row = t.queue_row(&pr.id)?;
        match row.as_ref().map(|r| r.state.as_str()) {
            Some("window") => {}
            Some("handed") => {
                return Err(conflict(format!(
                    "PR #{number} has gone to DevOps to merge; Undo is only in the 10 s window"
                )))
            }
            _ => return Err(conflict(format!("PR #{number} isn't merging"))),
        }
        for review in t.reviews(&pr.id)? {
            if review.role == "owner" && review.verdict == Verdict::Approved && !review.stale(&pr) {
                t.withdraw_review(&review.id, &by)?;
            }
        }
        t.set_queue_state(&pr.id, "undone", None, None, &pr.head_patch_id, now)?;
        t.set_pr_state(&pr, PrState::Open)?;
        t.pr(project, number)?
            .ok_or_else(|| not_found(format!("no PR #{number}")))
    })
}
