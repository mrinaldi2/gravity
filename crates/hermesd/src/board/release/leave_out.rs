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
//! - IPC `hermes/leave_out {id}` and `hermes/leave_out_pushed {id, branch,
//!   head}` serve DevOps' command (its task, its role).

use std::sync::Arc;

use bus::{MessageKind, TaskState, DEFAULT_TASK_DEADLINE_HOURS};
use serde_json::{json, Value};

use crate::actor::Actor;
use crate::app::AppState;
use crate::board::model::{ColumnCategory, LinkKind, Role};
use crate::db::prs::NewPr;
use crate::db::{LeaveOut, OwnerProof};
use crate::decisions::{conflict, forbidden, invalid, not_found};
use crate::messaging::{self, daemon_sender, Dm};
use crate::prs::model::Pr;
use crate::prs::{checks, owner, repo};

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

/// A submitted package's items wait in Owner testing: they go back to
/// Verify, free of it, until it is submitted again.
fn unsubmit(
    t: &crate::db::BoardTx<'_>,
    release: &Release,
) -> anyhow::Result<(Vec<(String, String)>, Vec<String>)> {
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

const GATE: &str = "hermes/leave_out";
const PUSHED: &str = "hermes/leave_out_pushed";

/// Answers DevOps' `hermesd release leave-out`, or `None` for anything else.
pub async fn serve(app: &Arc<AppState>, bot_id: &str, request: &Value) -> Option<Value> {
    let method = request.get("method").and_then(Value::as_str)?;
    if ![GATE, PUSHED].contains(&method) {
        return None;
    }
    let id = request.get("id").cloned().unwrap_or(Value::Null);
    let params = request.get("params").cloned().unwrap_or(json!({}));
    let (app, bot_id, method) = (app.clone(), bot_id.to_string(), method.to_string());
    let answer = tokio::task::spawn_blocking(move || executor(&app, &bot_id, &method, &params))
        .await
        .unwrap_or_else(|e| Err(anyhow::anyhow!("the leave-out gate failed: {e}")));
    Some(match answer {
        Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
        Err(e) => json!({ "jsonrpc": "2.0", "id": id,
                          "error": { "code": -32001, "message": e.to_string() } }),
    })
}

/// DevOps' side: what to revert, then the pushed branch becomes a PR.
pub fn executor(
    app: &Arc<AppState>,
    bot_id: &str,
    method: &str,
    params: &Value,
) -> anyhow::Result<Value> {
    let bot = app
        .db
        .get_live_bot(bot_id)?
        .ok_or_else(|| forbidden("no such bot"))?;
    let id = params["id"]
        .as_str()
        .ok_or_else(|| invalid("'id' is required"))?;
    let lo = app
        .db
        .board_read(|t| t.leave_out(id))?
        .filter(|lo| lo.state == "reverting")
        .ok_or_else(|| conflict(format!("no Leave out {id} waits for its revert")))?;
    let mine = match &lo.task_id {
        Some(task) => app
            .db
            .get_task(task)?
            .is_some_and(|t| t.to_bot_id == bot.id && t.state == TaskState::Open),
        None => false,
    };
    if !mine {
        return Err(forbidden("this Leave out's task isn't yours"));
    }
    let release = app
        .db
        .board_read(|t| load(t, &bot.project_id, &lo.release_id))?;
    let cut = release
        .cut
        .clone()
        .ok_or_else(|| conflict("the release has no cut"))?;
    let repo = repo::of_project(app, &bot.project_id, Some(&cut.repo))?;
    let mut left = Vec::new();
    for pr_id in &lo.pr_ids {
        let pr = app
            .db
            .board_read(|t| t.pr_by_id(pr_id))?
            .ok_or_else(|| not_found("a left-out PR is gone"))?;
        left.push(pr);
    }
    left.sort_by_key(|p| std::cmp::Reverse((p.merged_at, p.number)));
    let branch = format!(
        "leave-out/{}-{}",
        version_of(&release),
        &lo.id[..8.min(lo.id.len())]
    );
    if method == GATE {
        return Ok(json!({
            "id": lo.id, "branch": branch, "repo_url": repo.url, "release": release.name,
            "reverts": left.iter().map(|p| json!({
                "number": p.number, "from": p.base_sha, "to": p.merged_sha,
            })).collect::<Vec<_>>(),
        }));
    }
    let head = params["head"]
        .as_str()
        .ok_or_else(|| invalid("'head' is required"))?;
    let cache = repo.fetch(app)?;
    if repo::tip(&cache, &branch).as_deref() != Some(head) {
        return Err(conflict(format!(
            "{branch} on {} isn't at {head}; push it first",
            repo.name
        )));
    }
    let facts = repo::facts(&cache, head)?;
    let required = checks::required(&cache, &facts.base_sha, head)?;
    let card = left.last().map(|p| p.item_id.clone()).unwrap_or_default();
    let names: Vec<String> = left
        .iter()
        .rev()
        .map(|p| format!("#{}", p.number))
        .collect();
    let title = format!("Leave out {} from {}", names.join(", "), release.name);
    let pr = app.db.board_tx(|t| {
        let pr = t.insert_pr(&NewPr {
            project_id: &bot.project_id,
            repo: &repo.name,
            item_id: &card,
            branch: &branch,
            base_sha: &facts.base_sha,
            head_sha: head,
            patch_id: &facts.patch_id,
            author: &lo.by,
            title: &title,
            change_note: "The owner left these out of the release; this reverts them on main.",
        })?;
        t.set_pr_needs(&pr.id, &[])?;
        t.queue_checks(
            &bot.project_id,
            &repo.name,
            head,
            &required.tree,
            &required.checks,
        )?;
        t.add_item_link(
            &card,
            LinkKind::Pr,
            &format!("#{}", pr.number),
            Some(&title),
            &Actor::User,
        )?;
        t.set_leave_out(&lo.id, "pr_open", None, Some(&pr.id))?;
        Ok(pr)
    })?;
    if let Some(task) = &lo.task_id {
        app.db.try_close_task(task, TaskState::Done)?;
    }
    Ok(json!({ "number": pr.number, "branch": branch, "head": head }))
}

/// After a merge: a Leave out's revert re-cuts its release at the new main,
/// marks the left-out PRs reverted, and sends their cards back to Doing.
pub fn after_merge(app: &Arc<AppState>, merged: &Pr) -> anyhow::Result<()> {
    let Some(lo) = app.db.board_read(|t| t.leave_out_of_revert(&merged.id))? else {
        return Ok(());
    };
    let project = merged.project_id.as_str();
    let release = app.db.board_read(|t| load(t, project, &lo.release_id))?;
    let cut = release
        .cut
        .clone()
        .ok_or_else(|| conflict("the release has no cut"))?;
    let repo = repo::of_project(app, project, Some(&cut.repo))?;
    let cache = repo.fetch(app)?;
    let commit = merged.merged_sha.clone().unwrap_or_default();
    app.db.board_tx(|t| {
        for pr in &lo.pr_ids {
            t.mark_pr_reverted(pr, &merged.id, &release.id)?;
        }
        Ok(())
    })?;
    let range = cut::range(app, &cache, project, &repo.name, &commit, Some(&release.id))?;
    let mut feed = app.board.writer();
    let note = format!("left out of {}", version_of(&release));
    let moved = app.db.board_tx(|t| {
        cut::write(t, &release, &range, &cut.planned, &lo.by, "left_out")?;
        t.reset_release_builds(&release.id)?;
        t.set_leave_out(&lo.id, "done", None, None)?;
        let mut moved = Vec::new();
        for pr_id in &lo.pr_ids {
            let Some(pr) = t.pr_by_id(pr_id)? else {
                continue;
            };
            let from = daemon_move(
                t,
                project,
                &pr.item_id,
                ColumnCategory::Doing,
                &note,
                true,
                &Actor::User,
            )?;
            moved.extend(from.map(|f| (pr.item_id.clone(), f)));
        }
        Ok(moved)
    })?;
    publish_moves(app, &mut feed, project, &moved);
    Ok(())
}
