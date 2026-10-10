//! The owner's Leave out (H-272; UX-051 decision 10, Architect msg 4655):
//! a PR the owner doesn't want in a release cut from main.
//!
//! - `release_leave_out {project_id, release_id, prs}` (WS, the owner's
//!   device or ticket): if every left-out PR merged after every kept one,
//!   the release is cut again at the last kept merge. Otherwise each later
//!   PR touching the same files is refused by name; then DevOps gets a task
//!   to run `hermesd release leave-out <id>`, which reverts them on a branch
//!   from main. That branch becomes the owner's PR: checks, no bot review,
//!   the normal queue. Once it merges, the release is cut again at the new
//!   main, the left-out PRs are `reverted`, and their cards go back to Doing.
//! - Either way the package starts over: builds, tests and any ruling were
//!   for the old commit.
//! - DevOps' command and the re-cut after the revert merges are in
//!   `leave_out_gate`.

use std::sync::Arc;

use bus::{MessageKind, DEFAULT_TASK_DEADLINE_HOURS};
use serde_json::{json, Value};

use crate::actor::Actor;
use crate::app::AppState;
use crate::board::model::{ColumnCategory, Role};
use crate::db::{LeaveOut, OwnerProof};
use crate::decisions::{conflict, invalid};
use crate::messaging::{self, daemon_sender, Dm};
use crate::prs::model::Pr;
use crate::prs::{owner, repo};

use super::cut::{self, version_of};
use super::model::{Release, ReleaseStatus};
use super::{daemon_move, git_cache, load, publish_moves, publish_touched};

/// The owner's Leave out of `numbers` from `release_id`.
pub fn request(
    app: &Arc<AppState>,
    project: &str,
    release_id: &str,
    numbers: &[u32],
    proof: &OwnerProof,
) -> anyhow::Result<Value> {
    let by = format!("owner:{}", owner::provenance(proof)?);
    let release = app.db.board_read(|t| load(t, project, release_id))?;
    let cut = release
        .cut
        .clone()
        .ok_or_else(|| conflict(format!("release {} wasn't cut from main", release.name)))?;
    let open = matches!(
        release.status,
        ReleaseStatus::Assembling
            | ReleaseStatus::Built
            | ReleaseStatus::AwaitingOwner
            | ReleaseStatus::Held
    );
    if !open || cut.tag.is_some() {
        return Err(conflict(format!(
            "release {} is {}: a PR is left out only before it is approved",
            release.name,
            release.status.as_str()
        )));
    }
    if app
        .db
        .board_read(|t| t.open_leave_out(&release.id))?
        .is_some()
    {
        return Err(conflict(format!(
            "release {} is already leaving PRs out",
            release.name
        )));
    }
    let live: Vec<&super::model::CutPr> = cut
        .prs
        .iter()
        .filter(|p| !p.reverted && !p.revert)
        .collect();
    if numbers.is_empty() || numbers.iter().any(|n| !live.iter().any(|p| p.number == *n)) {
        return Err(invalid(format!(
            "name PRs that are in release {}: {}",
            release.name,
            live.iter()
                .map(|p| format!("#{}", p.number))
                .collect::<Vec<_>>()
                .join(", ")
        )));
    }
    let prs = |keep: bool| -> anyhow::Result<Vec<Pr>> {
        let mut out = Vec::new();
        for p in live.iter().filter(|p| numbers.contains(&p.number) != keep) {
            out.extend(app.db.board_read(|t| t.pr(project, p.number))?);
        }
        out.sort_by_key(|p| (p.merged_at, p.number));
        Ok(out)
    };
    let (left, kept) = (prs(false)?, prs(true)?);
    let Some(last_kept) = kept.last() else {
        return Err(conflict(format!(
            "nothing would be left in release {}; cancel it instead",
            release.name
        )));
    };
    let repo = repo::of_project(app, project, Some(&cut.repo))?;
    let cache = repo.fetch(app)?;
    if left.iter().all(|l| l.merged_at > last_kept.merged_at) {
        let commit = last_kept.merged_sha.clone().unwrap_or_default();
        let range = cut::range(app, &cache, project, &repo.name, &commit, Some(&release.id))?;
        let mut feed = app.board.writer();
        let (lo, moved, touched) = app.db.board_tx(|t| {
            let (moved, touched) = unsubmit(t, &release)?;
            cut::write(t, &release, &range, &cut.planned, &by, "left_out")?;
            t.reset_release_builds(&release.id)?;
            let ids: Vec<String> = left.iter().map(|p| p.id.clone()).collect();
            for l in &left {
                let body = format!(
                    "PR #{} was left out of {}: it stays on main and ships in a later release",
                    l.number,
                    version_of(&release)
                );
                t.add_item_comment(&l.item_id, &body, None, &Actor::User)?;
            }
            Ok((
                t.insert_leave_out(&release.id, &ids, "recut", &by)?,
                moved,
                touched,
            ))
        })?;
        publish_moves(app, &mut feed, project, &moved);
        publish_touched(app, &mut feed, project, &touched, &moved);
        drop(feed);
        withdraw(app, &release)?;
        return Ok(json!({ "leave_out": lo_json(&lo), "mode": "recut", "commit": commit }));
    }
    // A later kept PR that touched the same files can't survive the revert.
    let mut clashes = Vec::new();
    for l in &left {
        let mine = paths(&cache, l)?;
        for k in kept.iter().filter(|k| k.merged_at > l.merged_at) {
            let both: Vec<String> = paths(&cache, k)?
                .into_iter()
                .filter(|p| mine.contains(p))
                .collect();
            if !both.is_empty() {
                clashes.push(format!(
                    "PR #{} ({}) changed {} after #{}",
                    k.number,
                    k.item_id,
                    both.join(", "),
                    l.number
                ));
            }
        }
    }
    if !clashes.is_empty() {
        return Err(conflict(format!(
            "{}; leave those out too, or keep what they build on",
            clashes.join("; ")
        )));
    }
    let ids: Vec<String> = left.iter().map(|p| p.id.clone()).collect();
    let mut feed = app.board.writer();
    let (lo, moved, touched) = app.db.board_tx(|t| {
        let (moved, touched) = unsubmit(t, &release)?;
        t.reset_release_builds(&release.id)?;
        Ok((
            t.insert_leave_out(&release.id, &ids, "revert", &by)?,
            moved,
            touched,
        ))
    })?;
    publish_moves(app, &mut feed, project, &moved);
    publish_touched(app, &mut feed, project, &touched, &moved);
    drop(feed);
    withdraw(app, &release)?;
    task_devops(app, &release, &lo, &left)?;
    Ok(json!({ "leave_out": lo_json(&lo), "mode": "revert" }))
}

fn paths(cache: &std::path::Path, pr: &Pr) -> anyhow::Result<Vec<String>> {
    git_cache::changed_paths(
        cache,
        &pr.base_sha,
        pr.merged_sha.as_deref().unwrap_or_default(),
    )
}

fn lo_json(lo: &LeaveOut) -> Value {
    json!({ "id": lo.id, "release_id": lo.release_id, "mode": lo.mode, "state": lo.state })
}

/// Cards a change moved (card, the column it left) and touched.
type Moves = (Vec<(String, String)>, Vec<String>);

/// A submitted package's items wait in Owner testing: they go back to
/// Verify, free of it, until it is submitted again.
fn unsubmit(t: &crate::db::BoardTx<'_>, release: &Release) -> anyhow::Result<Moves> {
    let (mut moved, mut touched) = (Vec::new(), Vec::new());
    let note = format!("{} is built again without the PRs left out", release.name);
    for ri in &release.items {
        let Some(item) = t.item(&ri.item_id)? else {
            continue;
        };
        if item.category == ColumnCategory::Approval {
            let from = daemon_move(
                t,
                &release.project_id,
                &item.id,
                ColumnCategory::Verify,
                &note,
                false,
                &Actor::User,
            )?;
            moved.extend(from.map(|f| (item.id.clone(), f)));
        }
        if item.release_id.as_deref() == Some(release.id.as_str())
            && t.set_item_release(&item.id, None, &Actor::User)?
        {
            touched.push(item.id.clone());
        }
    }
    Ok((moved, touched))
}

/// The package starts over: the owner's ruling on it is withdrawn.
fn withdraw(app: &Arc<AppState>, release: &Release) -> anyhow::Result<()> {
    if let Some(decision) = &release.decision_id {
        let reason = format!("PRs were left out of {}; it is built again.", release.name);
        if app.db.try_withdraw(decision, &reason)? {
            crate::decisions::service::pushed(app, decision)?;
        }
    }
    Ok(())
}

fn task_devops(
    app: &Arc<AppState>,
    release: &Release,
    lo: &LeaveOut,
    left: &[Pr],
) -> anyhow::Result<()> {
    let devops = app
        .db
        .project_roles(&release.project_id)?
        .into_iter()
        .find(|r| r.role == Role::Devops)
        .map(|r| r.bot_id);
    let Some(devops) = devops else {
        tracing::warn!(
            release = release.name,
            "no DevOps to revert the left-out PRs"
        );
        return Ok(());
    };
    let names: Vec<String> = left.iter().map(|p| format!("#{}", p.number)).collect();
    let body = format!(
        "The owner left {} out of release {}. Run `hermesd release leave-out {}` in your \
         checkout: it reverts them on a branch from main and opens that as the owner's PR, which \
         merges through the queue (checks, no bot review). The release is then cut again.",
        names.join(", "),
        release.name,
        lo.id
    );
    let sender = daemon_sender();
    let msg = messaging::send_dm(
        &app.db,
        &app.events,
        Dm::new(&devops, &sender, MessageKind::Task, &body),
    )?;
    let deadline = chrono::Utc::now() + chrono::Duration::hours(DEFAULT_TASK_DEADLINE_HOURS);
    let task = app
        .db
        .create_task(&msg.id, None, &devops, Some(deadline), 1, "")?;
    if let Some(card) = left.first() {
        app.db
            .link_task_item(&task.id, &card.item_id, &Actor::User)?;
    }
    app.db
        .board_tx(|t| t.set_leave_out(&lo.id, "reverting", Some(&task.id), None))
}
