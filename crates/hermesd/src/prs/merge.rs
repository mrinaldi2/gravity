//! The daemon's side of `hermesd pr merge <n>` (H-284; H-261 §5.2, §5.3,
//! §15.2). The command runs in DevOps' session and asks over the local
//! endpoint, which knows the bot by its process, the same model as
//! `release land` (H-117 X2):
//!
//! - `hermes/pr_merge {number}`: the bot holds the `pr_merge` extra, is the
//!   project's DevOps and holds the PR's open `pr_merge` task. The daemon
//!   fetches the repo, checks the branch is still at the PR's head,
//!   recomputes the roles and the shape the change needs **at the merge**
//!   (main's tip to the head, CE on H-269), then every §5.1 condition. A
//!   PR that no longer holds leaves the queue and its task closes.
//! - `hermes/pr_merged {number, sha, branch}`: the daemon's own fetch must
//!   show main at `sha`, the PR's head. The PR is merged, its task done, and
//!   its card moves Review → Verify once every PR it has, in every repo, is
//!   merged (§6.5). A remote branch DevOps kept is said on the card.

use std::sync::Arc;

use bus::{PermissionExtra, TaskState};
use chrono::Utc;
use serde_json::{json, Value};

use crate::actor::Actor;
use crate::app::AppState;
use crate::board::model::{ColumnCategory, Role};
use crate::board::release::{daemon_move, git_cache, publish_moves};
use crate::bot_permissions::Stored;
use crate::decisions::{conflict, forbidden, invalid, not_found};
use crate::prs::model::{Pr, PrState};
use crate::prs::{mergeable, repo, review};

const MERGE: &str = "hermes/pr_merge";
const MERGED: &str = "hermes/pr_merged";

/// Answers one of the merge gates from `bot_id`'s session, or `None` for
/// any other request. Git runs off the async workers.
pub async fn serve(app: &Arc<AppState>, bot_id: &str, request: &Value) -> Option<Value> {
    let method = request.get("method").and_then(Value::as_str)?;
    if ![MERGE, MERGED].contains(&method) {
        return None;
    }
    let id = request.get("id").cloned().unwrap_or(Value::Null);
    let params = request.get("params").cloned().unwrap_or(json!({}));
    let (app, bot_id, method) = (app.clone(), bot_id.to_string(), method.to_string());
    let answer = tokio::task::spawn_blocking(move || answer(&app, &bot_id, &method, &params))
        .await
        .unwrap_or_else(|e| Err(anyhow::anyhow!("the merge gate failed: {e}")));
    Some(match answer {
        Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
        Err(e) => json!({ "jsonrpc": "2.0", "id": id,
                          "error": { "code": -32001, "message": e.to_string() } }),
    })
}

pub fn answer(
    app: &Arc<AppState>,
    bot_id: &str,
    method: &str,
    params: &Value,
) -> anyhow::Result<Value> {
    let bot = app
        .db
        .get_live_bot(bot_id)?
        .ok_or_else(|| forbidden("no such bot"))?;
    let number = params
        .get("number")
        .and_then(Value::as_u64)
        .and_then(|n| u32::try_from(n).ok())
        .ok_or_else(|| invalid("'number' is required"))?;
    let pr = gate(app, &bot, number)?;
    if method == MERGE {
        check(app, &pr)
    } else {
        let sha = params
            .get("sha")
            .and_then(Value::as_str)
            .ok_or_else(|| invalid("'sha' is required"))?;
        merged(app, &bot, &pr, sha, &params["branch"])
    }
}

/// Who may, for which PR: the extra, DevOps, the PR handed to this bot.
fn gate(app: &Arc<AppState>, bot: &bus::Bot, number: u32) -> anyhow::Result<Pr> {
    // A merge pushes main: never from a scratch daemon (H-171).
    if app.cfg.scratch {
        return Err(forbidden("a scratch daemon merges nothing"));
    }
    if !Stored::load(&app.db, bot)?
        .extras
        .contains(&PermissionExtra::PrMerge)
    {
        return Err(forbidden(
            "this needs the pr_merge extra; the owner grants it in your permissions",
        ));
    }
    let devops = app
        .db
        .project_roles(&bot.project_id)?
        .into_iter()
        .any(|r| r.bot_id == bot.id && r.role == Role::Devops);
    if !devops {
        return Err(forbidden("only the project's DevOps merges a PR"));
    }
    let (pr, row) = app.db.board_read(|t| {
        let pr = t
            .pr(&bot.project_id, number)?
            .ok_or_else(|| not_found(format!("no PR #{number} in this project")))?;
        let row = t.queue_row(&pr.id)?;
        Ok((pr, row))
    })?;
    let task = row
        .filter(|r| r.state == "handed")
        .and_then(|r| r.task_id)
        .ok_or_else(|| {
            conflict(format!(
                "PR #{number} isn't waiting to merge; the daemon hands DevOps a pr_merge task \
                 when it is"
            ))
        })?;
    let mine = app
        .db
        .get_task(&task)?
        .is_some_and(|t| t.to_bot_id == bot.id && t.state == TaskState::Open);
    if !mine {
        return Err(forbidden(format!(
            "PR #{number}'s pr_merge task isn't yours"
        )));
    }
    Ok(pr)
}

/// Before the push: the branch is still at the head, the needs are read
/// again at the merge, and every §5.1 condition holds.
fn check(app: &Arc<AppState>, pr: &Pr) -> anyhow::Result<Value> {
    let repo = repo::of_project(app, &pr.project_id, Some(&pr.repo))?;
    let cache = repo.fetch(app)?;
    let main = git_cache::resolve(&cache, "refs/heads/main")
        .ok_or_else(|| anyhow::anyhow!("{} has no main branch", repo.name))?;
    let tip = repo::tip(&cache, &pr.branch);
    if tip.as_deref() != Some(pr.head_sha.as_str()) {
        return refuse(
            app,
            pr,
            format!(
                "{} is at {}, not the PR's head {}: nothing merged",
                pr.branch,
                tip.as_deref().unwrap_or("nothing"),
                pr.head_sha
            ),
        );
    }
    // The owner's revert merges unreviewed only as the daemon checked it
    // (H-272 M1).
    if super::owner::is_owners(pr) {
        if let Err(e) = crate::board::release::leave_out_gate::check_revert(app, &cache, pr) {
            return refuse(app, pr, e.to_string());
        }
    }
    // What the merge brings is main..head: its roles and shape decide, not
    // a value cached at an earlier head (CE on H-269).
    let at_merge = Pr {
        base_sha: main.clone(),
        ..pr.clone()
    };
    let (roles, shape) = review::needs(app, &at_merge)?;
    let names: Vec<&str> = roles.iter().map(|r| r.as_str()).collect();
    app.db.board_tx(|t| {
        t.set_pr_needs(&pr.id, &names)?;
        t.set_pr_shape(&pr.id, &shape)
    })?;
    let now = mergeable::compute(app, pr, true)?;
    if !now.ok() {
        let why: Vec<&str> = now.blockers.iter().map(|b| b.text.as_str()).collect();
        return refuse(
            app,
            pr,
            format!("PR #{} isn't mergeable: {}", pr.number, why.join("; ")),
        );
    }
    // The pass `pr_merged` needs (ARCH M1): this head, onto this main.
    app.db
        .board_tx(|t| t.record_merge_check(&pr.id, &pr.head_sha, &main, Utc::now()))?;
    Ok(json!({
        "number": pr.number,
        "head_sha": pr.head_sha,
        "branch": pr.branch,
        "main_sha": main,
        "repo": repo.name,
        // Where the result is checked: the project's repo, not the
        // checkout's own remote.
        "repo_url": repo.url,
    }))
}

/// The PR no longer holds: it leaves the queue (it rejoins when it holds
/// again), its task closes, and DevOps is told why.
fn refuse(app: &Arc<AppState>, pr: &Pr, why: String) -> anyhow::Result<Value> {
    let task = app.db.board_tx(|t| {
        let task = t.queue_row(&pr.id)?.and_then(|r| r.task_id);
        t.dequeue(&pr.id)?;
        if pr.state == PrState::Merging {
            t.set_pr_state(pr, PrState::Open)?;
        }
        Ok(task)
    })?;
    if let Some(task) = task {
        app.db.try_close_task(&task, TaskState::Cancelled)?;
    }
    Err(conflict(why))
}

/// How long a gate pass stays good for the push that follows it.
pub const CHECK_FRESH: chrono::Duration = chrono::Duration::minutes(15);

/// Main is at `sha`: it got there through a recent gate pass for this head,
/// from the main that pass saw (ARCH M1 on H-284). Otherwise main moved
/// outside the gate: nothing is recorded, the card stays, and the owner
/// sees it.
fn passed(app: &Arc<AppState>, cache: &std::path::Path, pr: &Pr, sha: &str) -> anyhow::Result<()> {
    let now = Utc::now();
    let check = app.db.board_read(|t| t.merge_check(&pr.id))?;
    let ok = match &check {
        Some((head, main, at)) => {
            head == sha
                && now - *at <= CHECK_FRESH
                && git_cache::contains(cache, sha, main).unwrap_or(false)
        }
        None => false,
    };
    if ok {
        return Ok(());
    }
    app.db.board_tx(|t| t.mark_main_moved(pr, sha, now))?;
    tracing::warn!(
        pr = pr.number,
        sha,
        "main reached a PR's head without a merge gate pass; nothing recorded"
    );
    Err(conflict(format!(
        "main reached {sha} without a passed `hermesd pr merge` check for PR #{}: main moved \
         outside the gate, so nothing is recorded and the owner is told",
        pr.number
    )))
}

/// After the push: main is at the head as the daemon fetches it.
fn merged(
    app: &Arc<AppState>,
    bot: &bus::Bot,
    pr: &Pr,
    sha: &str,
    branch: &Value,
) -> anyhow::Result<Value> {
    if sha != pr.head_sha {
        return Err(forbidden(format!(
            "{sha} isn't PR #{}'s head {}",
            pr.number, pr.head_sha
        )));
    }
    let repo = repo::of_project(app, &pr.project_id, Some(&pr.repo))?;
    let cache = repo.fetch(app)?;
    let main = git_cache::resolve(&cache, "refs/heads/main");
    if main.as_deref() != Some(sha) {
        return Err(conflict(format!(
            "{} has main at {}, not {sha}; nothing recorded",
            repo.name,
            main.as_deref().unwrap_or("nothing")
        )));
    }
    passed(app, &cache, pr, sha)?;
    let actor = Actor::Bot {
        id: &bot.id,
        project_id: &bot.project_id,
    };
    let kept = branch["deleted"].as_bool() != Some(true);
    let kept_note = branch["note"].as_str().unwrap_or("not deleted");
    let task = app
        .db
        .board_read(|t| t.queue_row(&pr.id))?
        .and_then(|r| r.task_id);
    let mut feed = app.board.writer();
    let (waiting, moved) = app.db.board_tx(|t| {
        t.set_pr_merged(pr, sha, &bot.id)?;
        t.dequeue(&pr.id)?;
        if kept {
            let body = format!(
                "PR #{} merged at {sha}; its branch {} was kept: {kept_note}",
                pr.number, pr.branch
            );
            t.add_item_comment(&pr.item_id, &body, None, &actor)?;
        }
        // A card with PRs in several repos moves once all of them merged.
        let waiting: Vec<String> = t
            .prs_of_item(&pr.item_id)?
            .into_iter()
            .filter(|p| p.state.is_live())
            .map(|p| format!("#{} ({})", p.number, p.repo))
            .collect();
        let in_review = t
            .item(&pr.item_id)?
            .is_some_and(|i| i.category == ColumnCategory::Review);
        if !waiting.is_empty() || !in_review {
            return Ok((waiting, Vec::new()));
        }
        let note = format!("PR #{} merged", pr.number);
        let moved = daemon_move(
            t,
            &pr.project_id,
            &pr.item_id,
            ColumnCategory::Verify,
            &note,
            false,
            &actor,
        )?
        .map(|from| vec![(pr.item_id.clone(), from)])
        .unwrap_or_default();
        Ok((waiting, moved))
    })?;
    publish_moves(app, &mut feed, &pr.project_id, &moved);
    drop(feed);
    // A Leave out's revert re-cuts its release (H-272).
    let done = app.db.board_read(|t| t.pr_by_id(&pr.id))?;
    if let Some(done) = done {
        if let Err(e) = crate::board::release::leave_out_gate::after_merge(app, &done) {
            tracing::warn!(pr = pr.number, error = %e, "the Leave out couldn't re-cut its release");
        }
    }
    // Cleanup after merge (CL-1): the PR's worktrees, on every computer.
    match app.db.board_read(|t| t.pr_by_id(&pr.id)) {
        Ok(Some(merged)) => match crate::cleanup::enqueue(app, &merged) {
            Ok(_) => crate::cleanup::kick(app),
            Err(error) => tracing::warn!(pr = pr.number, %error, "cleanup not queued"),
        },
        _ => tracing::warn!(pr = pr.number, "cleanup not queued: the PR can't be read"),
    }
    if let Some(task) = task {
        app.db.try_close_task(&task, TaskState::Done)?;
    }
    Ok(json!({
        "number": pr.number,
        "sha": sha,
        "card": pr.item_id,
        "moved": !moved.is_empty(),
        "waiting_for": waiting,
        "branch_kept": kept,
    }))
}
