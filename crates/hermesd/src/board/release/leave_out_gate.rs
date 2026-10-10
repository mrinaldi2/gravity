//! DevOps' side of the owner's Leave out (H-272; see `leave_out`): the
//! `hermesd release leave-out` gates, and the re-cut once the owner's revert
//! PR merged.
//!
//! - IPC `hermes/leave_out {id}`: DevOps, holding the Leave out's task, gets
//!   the branch name and each PR's merged range, newest first.
//! - IPC `hermes/leave_out_pushed {id, branch, head}`: the daemon's fetch must
//!   show the branch at `head`; it becomes the owner's PR (checks, no bot
//!   review, the normal queue).
//! - `after_merge`: when that PR merges, the left-out PRs are `reverted`, the
//!   release is cut again at the new main and their cards go back to Doing.

use std::sync::Arc;

use bus::TaskState;
use serde_json::{json, Value};

use crate::actor::Actor;
use crate::app::AppState;
use crate::board::model::{ColumnCategory, LinkKind};
use crate::db::prs::NewPr;
use crate::decisions::{conflict, forbidden, invalid, not_found};
use crate::prs::model::Pr;
use crate::prs::{checks, repo};

use super::cut::{self, version_of};
use super::{daemon_move, git_cache, load, publish_moves};

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
    let cache = repo.fetch(app)?;
    if method == GATE {
        // Main as the revert starts from it: what the pushed branch must be
        // the exact revert of (H-272 M1).
        let main_at = git_cache::resolve(&cache, "refs/heads/main")
            .ok_or_else(|| conflict(format!("{} has no main branch", repo.name)))?;
        app.db
            .board_tx(|t| t.set_leave_out_main(&lo.id, &main_at))?;
        return Ok(json!({
            "id": lo.id, "branch": branch, "repo_url": repo.url, "release": release.name,
            "main_at": main_at,
            "reverts": left.iter().map(|p| json!({
                "number": p.number, "from": p.base_sha, "to": p.merged_sha,
            })).collect::<Vec<_>>(),
        }));
    }
    let head = params["head"]
        .as_str()
        .ok_or_else(|| invalid("'head' is required"))?;
    if repo::tip(&cache, &branch).as_deref() != Some(head) {
        return Err(conflict(format!(
            "{branch} on {} isn't at {head}; push it first",
            repo.name
        )));
    }
    let expected = expected_tree(&cache, &lo, &left, head)?;
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
        t.set_leave_out_revert(&lo.id, head, &expected)?;
        Ok(pr)
    })?;
    if let Some(task) = &lo.task_id {
        app.db.try_close_task(task, TaskState::Done)?;
    }
    Ok(json!({ "number": pr.number, "branch": branch, "head": head }))
}

/// The tree the pushed branch must have (H-272 M1): the daemon reverts the
/// left-out PRs on the main it named at the gate itself. The branch must
/// start from that main and hold exactly that tree: nothing added, nothing
/// else changed, since the owner's PR needs no review.
fn expected_tree(
    cache: &std::path::Path,
    lo: &crate::db::LeaveOut,
    newest_first: &[Pr],
    head: &str,
) -> anyhow::Result<String> {
    let main_at = lo
        .main_at
        .as_deref()
        .ok_or_else(|| conflict("ask what to revert first (hermes/leave_out)"))?;
    if !git_cache::contains(cache, head, main_at)? {
        return Err(conflict(format!(
            "{head} doesn't start from main as it was ({main_at}); nothing opened"
        )));
    }
    let reverts: Vec<(String, String)> = newest_first
        .iter()
        .map(|p| (p.base_sha.clone(), p.merged_sha.clone().unwrap_or_default()))
        .collect();
    let expected = git_cache::revert_tree(cache, main_at, &reverts)?
        .ok_or_else(|| conflict("the PRs don't revert cleanly on main; nothing opened"))?;
    if git_cache::tree_of(cache, head).as_deref() != Some(expected.as_str()) {
        return Err(conflict(format!(
            "{head} isn't exactly the revert of the left-out PRs: its files differ from \
             reverting them on main, so it isn't opened as the owner's PR"
        )));
    }
    Ok(expected)
}

/// The owner's revert PR may merge only as opened: its head, with the tree
/// the daemon computed (H-272 M1). `pr merge` asks this before pushing.
pub fn check_revert(app: &AppState, cache: &std::path::Path, pr: &Pr) -> anyhow::Result<()> {
    let lo = app.db.board_read(|t| t.leave_out_of_revert(&pr.id))?;
    let ok = lo.is_some_and(|lo| {
        lo.head.as_deref() == Some(pr.head_sha.as_str())
            && lo.expected_tree.is_some()
            && git_cache::tree_of(cache, &pr.head_sha) == lo.expected_tree
    });
    if !ok {
        return Err(conflict(format!(
            "PR #{} is the owner's only as the revert the daemon checked; its head changed",
            pr.number
        )));
    }
    Ok(())
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
